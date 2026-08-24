use std::{iter, ops::Range};

use crate::parser::ast::{
    Block, DukaChunk, Expr, ExprKind, Field, FuncBody, If, Param, Path, PathSuffix, Stmt, StmtKind,
    has_attr,
};
use duka_shared::{
    config::DukaIRConfig,
    constants::{catt, ccallish, cgen},
    errors::{DukaIRError, DukaIRErrorKind, Span},
    ir::{
        Allocator, Constants, Cst, DukaIR, ExpDesc, IR, Labels, Place, Reg, RegLifetime, Scope,
        Scopes, TablePlace, UpIndex, ValuePlace,
    },
    types::{BinOp, DebugInfo, DukaGenerator, SourceInfo, ValueCount},
    utils::{OrError, is_consecutive},
    value::ConstValue,
};

#[derive(Debug)]
pub struct IRGenerator {
    source_info: SourceInfo,
    config: DukaIRConfig,

    allocator: Allocator,
    labels: Labels,

    instructions: Vec<IR>,
    nesteds: Vec<DukaIR>,
    constants: Constants,
    scopes: Scopes,

    inst_spans: Vec<(Range<usize>, Span)>,
    using_regs: Vec<Box<[Reg]>>,
    used_reg_count: usize,
}

#[derive(Debug)]
enum LValue {
    Local(Reg),
    NewLocal(String),
    UpVal(usize),
    /// (env, key)
    Global(TablePlace, Cst),
    /// (table, key)
    SetByKey(TablePlace, Cst),
    /// (table, index)
    SetByIndex(TablePlace, ValuePlace),
}

#[derive(Debug, Clone, Copy, Default)]
enum ToReg {
    To(Reg),
    Temp,
    #[default]
    New,
}

impl Default for IRGenerator {
    fn default() -> Self {
        Self::new(Default::default(), Default::default())
    }
}

impl IRGenerator {
    pub fn new(config: DukaIRConfig, source_info: SourceInfo) -> Self {
        Self {
            source_info,
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            labels: Labels::new(),
            instructions: vec![],
            nesteds: vec![],
            inst_spans: vec![],
            using_regs: vec![],
            used_reg_count: 0,
            config,
        }
    }

    #[inline]
    fn emit(&mut self, ir: IR) {
        self.instructions.push(ir);
        self.using_regs
            .push(self.allocator.get_allocated_regs().into())
    }
    #[inline]
    fn emit_placeholder(&mut self) -> usize {
        self.emit(IR::Void);
        self.instructions.len() - 1
    }
    #[inline]
    fn emit_fixup(&mut self, who: usize, ir: IR) {
        assert!(who < self.instructions.len());
        self.instructions[who] = ir;
    }

    fn gen_stmts(&mut self, stmts: Vec<Stmt>) -> Result<(), DukaIRError> {
        for Stmt(stmt, span) in stmts {
            let start = self.instructions.len();
            let from = self.allocator.top();

            self.gen_stmt(Stmt(stmt, span))?;
            // 只回收本语句分配的非局部寄存器
            // 不能全收,否则会吃掉外层 for 的框架寄存器(生成器/状态/控制)
            self.recycle_anonymous_from(from);

            let end = self.instructions.len();
            self.inst_spans.push((start..end, span));
        }
        Ok(())
    }

    /// 归还 [from, top) 内不再被局部作用域绑定的寄存器
    /// 与 RegLifetime 一致,不被局部持有的都是死寄存器
    #[inline]
    fn recycle_anonymous_from(&mut self, from: Reg) {
        let dead: Vec<Reg> = self
            .allocator
            .get_allocated_regs()
            .iter()
            .copied()
            .filter(|&r| r >= from && !self.scopes.is_local_reg(r) && !self.scopes.is_captured(r))
            .collect();
        for r in dead {
            self.allocator.free(r);
        }
    }

    fn do_consecutive_from(
        &mut self,
        exprs: Vec<Expr>,
        start_at: Reg,
    ) -> Result<ExpDesc, DukaIRError> {
        let len = exprs.len();
        let mut tail_many = None;
        let mut exps = vec![];

        for (i, expr) in exprs.into_iter().enumerate() {
            let to_reg = ToReg::To(start_at + i);
            let ed = self.do_expr_to(expr, to_reg)?;
            if matches!(ed, ExpDesc::Many(..)) && i == len - 1 {
                tail_many = Some(ed);
            } else {
                let val = self.take_first(ed)?;
                let reg = self.must_allocated_at(val, to_reg)?;
                exps.push(reg);
            }
        }

        Ok(if let Some(ExpDesc::Many(fixed, start)) = tail_many {
            exps.extend(fixed);
            ExpDesc::Many(exps, start)
        } else {
            ExpDesc::Many(exps, None)
        })
    }

    #[inline(always)]
    fn do_consecutive_top(&mut self, exprs: Vec<Expr>) -> Result<ExpDesc, DukaIRError> {
        self.do_consecutive_from(exprs, self.allocator.top())
    }

    fn gen_return(&mut self, items: Vec<Expr>) -> Result<(), DukaIRError> {
        // 用 top()(高水位)而非 available_top():多返回值中若有尾调用,
        // 其结果落在 alloc_fresh(= 当前 top),只有从高水位开始才能保证
        // 定长返回值与尾调用结果寄存器连续。
        let eds = self.do_consecutive_from(items, self.allocator.top())?;
        let (start_reg, count) = self.take_all(eds)?;
        self.emit(IR::Return(start_reg, count));
        Ok(())
    }

    fn gen_block_with_locals(
        &mut self,
        Block(stmts, ret): Block,
        is_func: bool,
        locals: Vec<(String, Reg)>,
    ) -> Result<Option<Box<[UpIndex]>>, DukaIRError> {
        self.enter(is_func);

        for local in locals {
            self.scopes.declare_local(&local.0, local.1)?;
        }

        self.gen_stmts(stmts.to_vec())?;

        if let Some(ret) = ret
            && let StmtKind::Return(items) = (*ret).0
        {
            let span = (*ret).1;
            let start = self.instructions.len();

            self.gen_return(items.into())?;

            let end = self.instructions.len();
            self.inst_spans.push((start..end, span));
        }

        Ok(if is_func {
            Some(self.exit_func()?)
        } else {
            self.exit_block()?;
            None
        })
    }

    #[inline(always)]
    /// always call this for block generation
    fn gen_block_scoped(
        &mut self,
        block: Block,
        is_func: bool,
    ) -> Result<Option<Box<[UpIndex]>>, DukaIRError> {
        self.gen_block_with_locals(block, is_func, vec![])
    }

    fn load_nil(&mut self) -> Result<Reg, DukaIRError> {
        let reg = self.allocator.alloc()?;
        self.emit(IR::LoadNil(reg));
        Ok(reg)
    }
    #[inline(always)]
    fn load_nil_to(&mut self, to: Reg) {
        self.emit(IR::LoadNil(to));
    }
    fn load_const_to(&mut self, cv: ConstValue, reg: Reg) {
        match cv {
            ConstValue::Nil => self.load_nil_to(reg),
            ConstValue::Int(i) => {
                self.emit(IR::LoadInt(reg, i));
            }
            ConstValue::Float(f) => {
                self.emit(IR::LoadFloat(reg, f));
            }
            ConstValue::Bool(b) => {
                self.emit(if b {
                    IR::LoadTrue(reg)
                } else {
                    IR::LoadFalse(reg)
                });
            }
            // ConstValue::ConstTable(array_map) => {
            //     let idx = self.constants.push(ConstValue::ConstTable(array_map));
            //     self.emit(IR::LoadConst(reg, idx));
            // }
            ConstValue::String(items) => {
                self.emit(IR::LoadString(reg, items));
            }
        }
    }

    fn take_first(&mut self, exp: ExpDesc) -> Result<Place, DukaIRError> {
        match exp {
            ExpDesc::Single(pl) => Ok(pl),
            ExpDesc::Many(fixed, var_arg) => {
                let mut fixed = fixed.into_iter();
                if let Some(reg) = fixed.next() {
                    self.allocator.free_many(fixed);
                    Ok(Place::R(reg))
                } else if let Some(start) = var_arg {
                    self.emit(IR::Take(1));
                    Ok(Place::R(start))
                } else {
                    Ok(Place::R(self.load_nil()?))
                }
            }
        }
    }

    /// # Consecutive
    fn take_many(&mut self, exp: ExpDesc, needs: usize) -> Result<Vec<Place>, DukaIRError> {
        let mut many = Vec::with_capacity(needs);
        match exp {
            ExpDesc::Single(pl) => many.push(pl),
            ExpDesc::Many(fixed, var_arg) => {
                let fixed_count = fixed.len();
                many.extend(fixed.into_iter().take(needs).map(Place::R));

                if let Some(start) = var_arg {
                    if fixed_count < needs {
                        let rest = needs - fixed_count;
                        many.extend(
                            self.allocator
                                .alloc_consecutive_from(start, rest)?
                                .map(Place::R),
                        );
                        self.emit(IR::Take(rest));
                        return Ok(many);
                    } else {
                        self.allocator.free(start);
                    }
                }
            }
        }

        if many.len() < needs {
            let rest = needs - many.len();
            for _ in 0..rest {
                many.push(Place::R(self.load_nil()?));
            }
        }
        Ok(many)
    }

    /// # Consecutive
    fn take_all(&mut self, exp: ExpDesc) -> Result<(Reg, ValueCount), DukaIRError> {
        match exp {
            ExpDesc::Single(pl) => match pl {
                Place::R(r) => {
                    let reg = self.allocator.alloc()?;
                    self.gen_move(reg, r);
                    Ok((reg, ValueCount::Exact(1)))
                }
                pl => {
                    let reg = self.ensure_allocated(pl, ToReg::New)?;
                    Ok((reg, ValueCount::Exact(1)))
                }
            },
            ExpDesc::Many(fixed, var_arg) => {
                let fixed_count = fixed.len();
                if let Some(start) = var_arg {
                    assert!(is_consecutive(&[fixed.as_slice(), &[start]].concat()));
                    let fixed_start = fixed.iter().min().cloned();
                    self.emit(IR::TakeAll);
                    Ok((fixed_start.unwrap_or(start), ValueCount::VarArg))
                } else {
                    assert!(is_consecutive(&fixed), "{fixed:?}");
                    let fixed_start = fixed.iter().min().cloned();
                    Ok((
                        fixed_start.unwrap_or_default(),
                        ValueCount::Exact(fixed_count),
                    ))
                }
            }
        }
    }

    fn take_none(&mut self, exp: ExpDesc) {
        match exp {
            ExpDesc::Many(fixeds, var_arg) => {
                self.allocator.free_many(fixeds.into_iter());
                if let Some(start) = var_arg {
                    self.emit(IR::Take(0));
                    self.allocator.free(start);
                }
            }
            ExpDesc::Single(pl) => {
                if let Place::R(r) = pl {
                    self.allocator.free(r);
                }
            }
        }
    }

    fn set_to_path(&mut self, path: Path, global_first: bool) -> Result<LValue, DukaIRError> {
        Ok(match path {
            Path::Base((name, _)) => {
                // push name into constant pool
                if !global_first {
                    if let Some(pl) = self.scopes.find(&name) {
                        match pl {
                            Place::K(_) | Place::I(_) => {
                                return Err(DukaIRError::from(DukaIRErrorKind::TryAssignConst(
                                    name.into(),
                                )));
                            }
                            Place::R(r) => LValue::Local(r),
                            Place::U(u) => LValue::UpVal(u),
                        }
                    } else {
                        LValue::NewLocal(name)
                    }
                } else {
                    let env = self.scopes.ensure_global();
                    LValue::Global(self.only_modifiable(env)?, self.constants.push(name.into()))
                }
            }
            Path::Chain(parent, suffix) => {
                let base = self.get_path_to(*parent, false, ToReg::New)?;
                let table = self.only_modifiable(base)?;
                match suffix {
                    PathSuffix::TypeArgs(..) => {
                        return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(
                            "cannot assign through generic instantiation".into(),
                        )));
                    }
                    PathSuffix::Colon((func, _)) | PathSuffix::Dot((func, _)) => {
                        // `function t:m()...` 等价于 `t.m = function(self, ...)`
                        LValue::SetByKey(table, self.constants.push(func.into()))
                    }
                    PathSuffix::Index(idx) => {
                        let idx = self.do_expr_to(*idx, ToReg::New)?;
                        let pl = self.take_first(idx)?;
                        let val = self.without_up_val(pl, ToReg::Temp)?;
                        LValue::SetByIndex(table, val)
                    }
                }
            }
            Path::Expr(_) => {
                return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(
                    "trying to assign value(s) to an expression".into(),
                )));
            }
        })
    }

    fn get_path_to(
        &mut self,
        path: Path,
        global_first: bool,
        to_reg: ToReg,
    ) -> Result<Place, DukaIRError> {
        Ok(match path {
            Path::Base((name, _)) => {
                if !global_first && let Some(idx) = self.scopes.find(&name) {
                    idx
                } else {
                    // _ENV
                    let idx = self.constants.push(name.into());
                    let env = self.scopes.ensure_global();
                    let tab = self.only_modifiable(env)?;
                    let reg = self.get_reg(to_reg)?;
                    self.emit(IR::GetField(reg, tab, ValuePlace::K(idx)));
                    Place::R(reg)
                }
            }
            Path::Expr(expr) => {
                let exp = self.do_expr_to(*expr, to_reg)?;
                // for expression, we don't reuse the register
                self.take_first(exp)?
            }
            Path::Chain(parent, suffix) => {
                let place = self.get_path_to(*parent, global_first, to_reg)?;
                if matches!(&suffix, PathSuffix::TypeArgs(..)) {
                    return Ok(place);
                }
                let table = self.only_modifiable(place)?;
                match suffix {
                    // NOTICE: this is special, it only appears in function calling, but we don't deal it here
                    PathSuffix::Colon((key, _)) |
                    // We can reuse the register, reuse table register is perfect
                    PathSuffix::Dot((key, _)) => {
                        let key = self.constants.push(key.into());
                        let reg = self.get_reg(to_reg)?;
                        self.emit(IR::GetField(reg, table, ValuePlace::K(key)));
                        Place::R(reg)
                    }
                    PathSuffix::Index(idx) => {
                        let exp = self.do_expr_to(*idx, ToReg::New)?;
                        let reg = self.get_reg(to_reg)?;

                        let idx_pl = self.take_first(exp)?;
                        let idx = self.without_up_val(idx_pl, ToReg::New)?;

                        self.emit(IR::GetField(reg, table, idx));
                        Place::R(reg)
                    }
                    PathSuffix::TypeArgs(..) => unreachable!(),
                }
            }
        })
    }

    fn must_allocated_at(&mut self, pl: Place, to_reg: ToReg) -> Result<Reg, DukaIRError> {
        let to = self.get_reg(to_reg)?;
        match pl {
            Place::K(k) => {
                self.emit(IR::LoadConst(to, k));
            }
            Place::U(u) => {
                self.emit(IR::GetUpVal(to, u));
            }
            Place::R(r) => {
                self.gen_move(to, r);
            }
            Place::I(i) => {
                self.emit(IR::LoadInt(to, i));
            }
        }
        Ok(to)
    }

    fn only_modifiable(&mut self, pl: Place) -> Result<TablePlace, DukaIRError> {
        Ok(match pl {
            Place::R(r) => TablePlace::R(r),
            Place::U(u) => TablePlace::U(u),
            pl => {
                return Err(DukaIRError {
                    kind: DukaIRErrorKind::TryModifyReadonly(pl.to_string().into()),
                });
            }
        })
    }

    fn without_up_val(&mut self, pl: Place, if_up_val: ToReg) -> Result<ValuePlace, DukaIRError> {
        Ok(match pl {
            Place::R(r) => ValuePlace::R(r),
            Place::K(k) => ValuePlace::K(k),
            Place::U(..) => ValuePlace::R(self.must_allocated_at(pl, if_up_val)?),
            Place::I(i) => ValuePlace::I(i),
        })
    }

    /// # Ensure pl has been allocated, which means if pl is already in a register
    fn ensure_allocated(&mut self, pl: Place, if_not: ToReg) -> Result<Reg, DukaIRError> {
        if let Place::R(r) = pl {
            self.allocator.ensure_allocated(r)?;
            Ok(r)
        } else {
            self.must_allocated_at(pl, if_not)
        }
    }

    fn gen_params(&mut self, params: Vec<Expr>, start_at: Reg) -> Result<ValueCount, DukaIRError> {
        let exp = self.do_consecutive_from(params, start_at)?;
        let (start, count) = self.take_all(exp)?;
        // clear register
        match count {
            ValueCount::Exact(n) => self.allocator.free_many(start..start + n),
            ValueCount::VarArg => self.allocator.free_many(start..),
        }
        Ok(count)
    }

    fn gen_call_to(
        &mut self,
        callee: Expr,
        params: Vec<Expr>,
        tailcall: bool,
        _to_reg: ToReg,
    ) -> Result<ExpDesc, DukaIRError> {
        let is_call_like = callee.0.is_callable_keyword();
        let self_call = callee.0.is_self_call();

        let expr_len = params.len();

        let callee = if is_call_like.is_some() {
            self.allocator.alloc_temp()?
        } else if self_call {
            // `a:b(args)` 脱糖为 `a.b(a, args)`。
            // 接收者直接求值到 self 实参槽 top+1,避免在 top 之下留下
            // 死寄存器(否则外层调用会把残留的接收者当成额外实参)。
            let (parent, key) = match &callee.0 {
                ExprKind::Access(path) => match &**path {
                    Path::Chain(parent, PathSuffix::Colon((key, _))) => {
                        (parent.clone(), key.clone())
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            let top = self.allocator.alloc_fresh()?;
            let pl = self.get_path_to(*parent, false, ToReg::To(top + 1))?;
            match pl {
                Place::R(r) if r != top + 1 => self.gen_move(top + 1, r),
                _ => {
                    self.ensure_allocated(pl, ToReg::To(top + 1))?;
                }
            }
            let key_c = self.constants.push(key.into());
            self.emit(IR::GetField(
                top,
                TablePlace::R(top + 1),
                ValuePlace::K(key_c),
            ));
            top
        } else {
            // The callee (and therefore the call frame) must sit above every
            // live register: both user and native frames resolve arguments
            // from `func+1..` up to the current stack top, so values still
            // live below `func` survive the call untouched.
            let top = self.allocator.alloc_fresh()?;
            let exp = self.do_expr_to(callee, ToReg::To(top))?;
            let pl = self.take_first(exp)?;
            match pl {
                Place::R(r) if r != top => self.gen_move(top, r),
                _ => {
                    self.ensure_allocated(pl, ToReg::To(top))?;
                }
            }
            top
        };

        if let Some(ccallish::SPAWN) = is_call_like {
            (expr_len != 1).then_error(|| {
                DukaIRError::from(DukaIRErrorKind::InvalidParams(
                    ccallish::SPAWN.into(),
                    1,
                    expr_len,
                ))
            })?;

            let func = self.do_consecutive_top(params)?;
            let one = self.take_first_allocated(func)?;

            self.emit(IR::Spawn(callee, one));
            return Ok(ExpDesc::Single(Place::R(callee)));
        }

        // Params: place arguments right after the callee (`func+1..`), which is
        // what both user and native call frames expect. 对方法调用,self 已位于 callee+1。
        // self 槽必须高于 allocator 高水位:alloc_fresh 只从 current.top 分配,
        // 否则参数表达式内部(如 concat 的搬移寄存器)会复用 self 槽位。参数槽则
        // 允许被复用——结果最终会写回该槽。
        let count = if self_call {
            self.allocator.ensure_allocated(callee + 1)?;
            match self.gen_params(params, callee + 2)? {
                ValueCount::Exact(n) => ValueCount::Exact(n + 1),
                ValueCount::VarArg => ValueCount::VarArg,
            }
        } else {
            self.gen_params(params, callee + 1)?
        };

        // Call
        if let Some(kw) = is_call_like {
            (expr_len < 1).then_error(|| {
                DukaIRError::from(DukaIRErrorKind::InvalidParams(
                    ccallish::SPAWN.into(),
                    1,
                    expr_len,
                ))
            })?;
            self.allocator.ensure_allocated(callee)?;
            self.emit(match kw {
                ccallish::GO => IR::Go(callee, count - 1),
                ccallish::YIELD => IR::Yield(callee, count),
                _ => unreachable!(),
            });
        } else {
            self.emit(if tailcall {
                IR::TailCall(callee, count)
            } else {
                IR::Call(callee, count)
            });
        }
        Ok(ExpDesc::Many(vec![], Some(callee)))
    }

    // return the start reg
    #[inline(always)]
    fn gen_call(
        &mut self,
        callee: Expr,
        params: Vec<Expr>,
        tailcall: bool,
    ) -> Result<ExpDesc, DukaIRError> {
        self.gen_call_to(callee, params, tailcall, ToReg::New)
    }

    /// DO NOT INPUT EMPTY EXPR
    /// # Always allocate new register
    #[inline(always)]
    fn do_expr(&mut self, expr: Expr) -> Result<ExpDesc, DukaIRError> {
        self.do_expr_to(expr, ToReg::New)
    }
    fn get_reg(&mut self, reg: ToReg) -> Result<Reg, DukaIRError> {
        match reg {
            ToReg::Temp => self.allocator.alloc_temp(),
            ToReg::New => self.allocator.alloc(),
            ToReg::To(reg) => {
                self.allocator.ensure_allocated(reg)?;
                Ok(reg)
            }
        }
    }

    /// - reg: target register (if it has, or allocate new one)
    /// - keep_im: whether `ConstValue` should be allocated or not
    fn do_expr_to(&mut self, Expr(expr, span): Expr, reg: ToReg) -> Result<ExpDesc, DukaIRError> {
        use ExprKind::*;

        expr.is_sugar().then_error(|| {
            DukaIRError::from(DukaIRErrorKind::UnsupportedFeature(expr.to_string().into()))
        })?;

        Ok(ExpDesc::Single(match expr {
            Empty => {
                let reg = self.get_reg(reg)?;
                self.emit(IR::LoadNil(reg));
                Place::R(reg)
            }
            VarArg => {
                let reg = self.get_reg(reg)?;
                self.emit(IR::VarArg(reg));
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Literal(cv) => {
                if let ConstValue::Int(val) = cv {
                    Place::I(val)
                } else {
                    let reg = self.get_reg(reg)?;
                    self.load_const_to(cv, reg);
                    Place::R(reg)
                }
            }
            Do(block) => return self.gen_expr_block(*block),
            Access(path) => self.get_path_to(*path, false, reg)?,
            Call(callee, params) => {
                // the tailcall place is already processed
                return self.gen_call_to(*callee, params.to_vec(), false, reg);
            }
            SysCall(sys_call) => {
                let reg = self.get_reg(reg)?;
                self.emit(IR::SysCall(reg, sys_call));
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Table(fields) => self.do_table_to(reg, fields.to_vec(), false)?,
            Array(items) => {
                self.do_table_to(reg, items.iter().cloned().map(Field::Value).collect(), true)?
            }
            Function(func_body) => {
                let mut ir = self.gen_func_block(func_body, false, None, span)?;
                ir.debug_info.all_span = span;
                self.nesteds.push(ir);
                let reg = self.get_reg(reg)?;
                self.emit(IR::Closure(reg, self.nesteds.len() - 1));
                Place::R(reg)
            }
            Unary(expr, un_op) => {
                let ed = self.do_expr(*expr)?;
                let reg = self.get_reg(reg)?;
                let operand_pl = self.take_first(ed)?;
                let operand = self.without_up_val(operand_pl, ToReg::To(reg))?;

                self.emit(IR::Unary(reg, operand, un_op));

                Place::R(reg)
            }
            Binary(le, re, BinOp::Concat) => {
                let le = self.do_expr(*le)?;
                let start = self.take_first_allocated(le)?;

                let mut current = *re;
                let mut exprs = vec![];
                loop {
                    // Take the kind out of `current` WITHOUT destroying it:
                    // `std::mem::take` in the match scrutinee used to empty the
                    // whole expr, so a non-concat operand (`a .. b` with
                    // variables) was pushed as `Empty` and lost.
                    let kind = std::mem::take(&mut current.0);
                    if let ExprKind::Binary(le2, re2, BinOp::Concat) = kind {
                        exprs.push(*le2);
                        current = *re2;
                    } else {
                        current.0 = kind;
                        exprs.push(std::mem::take(&mut current));
                        break;
                    }
                }

                let count = exprs.len();

                // The Concat instruction needs every operand in one consecutive
                // run starting at the left operand's register. That only holds
                // when no live register sits above `start`; otherwise the right
                // operands would clobber it (e.g. `s = s .. "a"` with a local
                // `i` living right after `s`). In that case relocate the left
                // operand above everything live first.
                let left_at_top = self
                    .allocator
                    .get_allocated_regs()
                    .iter()
                    .max()
                    .map_or(true, |m| *m <= start);
                let base = if left_at_top {
                    start
                } else {
                    let fresh = self.allocator.alloc_fresh()?;
                    if fresh != start {
                        self.emit(IR::Move(fresh, start));
                    }
                    fresh
                };

                let ed = self.do_consecutive_from(exprs, base + 1)?;
                let ic = is_consecutive(
                    &self
                        .take_many(ed, count)?
                        .into_iter()
                        .enumerate()
                        .map(|(i, pl)| self.must_allocated_at(pl, ToReg::To(base + i + 1)))
                        .collect::<Result<Vec<_>, _>>()?,
                );
                assert!(ic);

                let reg = self.get_reg(reg)?;
                // `count + 1` total operands: the left operand sits at `base`,
                // the rest at `base + 1..`, so the run includes the left too.
                self.emit(IR::Concat(reg, base, count + 1));

                Place::R(reg)
            }
            Binary(le, re, bin_op) => self.do_binary_to(reg, *le, *re, bin_op)?,
            If(ifs) => self.do_if_to(reg, *ifs)?,
            _ => unreachable!(),
        }))
    }

    fn do_binary_to(
        &mut self,
        reg: ToReg,
        le: Expr,
        re: Expr,
        bin_op: BinOp,
    ) -> Result<Place, DukaIRError> {
        let reg = self.get_reg(reg)?;
        if bin_op.is_short() {
            let le = self.do_expr_to(le, ToReg::To(reg))?;
            let lp = self.take_first(le)?;
            self.must_allocated_at(lp, ToReg::To(reg))?;

            let lab = self.labels.new_label(None)?;
            // Test 指令语义: `eval_to_bool(reg) == target` 时跳过下一条(Jump)。
            // and: 左真 → 跳过 Jump → 求右(结果为右);左假 → Jump → 结果为左。
            // or:  左假 → 跳过 Jump → 求右(结果为右);左真 → Jump → 结果为左。
            self.emit(IR::SkipNext(reg, bin_op == BinOp::And));
            self.emit(IR::Jump(lab));

            let re = self.do_expr_to(re, ToReg::To(reg))?;
            let rp = self.take_first(re)?;
            self.must_allocated_at(rp, ToReg::To(reg))?;
            self.emit(IR::Label(lab));
        } else {
            let left = self.do_expr_to(le, ToReg::To(reg))?;
            let left_is_imm = matches!(left, ExpDesc::Single(Place::I(..)));
            // Consume the left operand *now*: codegen requires every `Call` to
            // be followed immediately by its `Take`, so the left call's `Take`
            // must not be deferred until after the right operand is evaluated.
            let left = self.take_first(left)?;

            let right = self.do_expr_to(
                re,
                left_is_imm.then_some(ToReg::To(reg)).unwrap_or(ToReg::Temp),
            )?;

            let left = self.without_up_val(left, ToReg::To(reg))?;

            let right = self.take_first(right)?;
            let right = self.without_up_val(right, ToReg::Temp)?;

            // Binary 之后 right 不再被引用
            // 非局部且非目标(左立即数时 right 可能就是 reg)才还槽
            let right_free = match right {
                ValuePlace::R(r) if r != reg && !self.scopes.is_local_reg(r) => Some(r),
                _ => None,
            };
            self.emit(IR::Binary(reg, left, right, bin_op));
            if let Some(r) = right_free {
                self.allocator.free(r);
            }
        }
        Ok(Place::R(reg))
    }

    fn do_if_to(&mut self, reg: ToReg, ifs: If) -> Result<Place, DukaIRError> {
        let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);
        let end = self.labels.new_label(None)?;
        self.gen_skip_next(*(if_.1), true)?;
        let mut lab = self.labels.new_label(None)?;
        self.emit(IR::Jump(lab));
        let ed = self.gen_expr_block(*if_.0)?;
        let pl = self.take_first(ed)?;
        self.must_allocated_at(pl, reg)?;
        self.emit(IR::Jump(end));
        for ifelse in ifelses {
            self.emit(IR::Label(lab));

            self.gen_skip_next(*(ifelse.1), true)?;
            lab = self.labels.new_label(None)?;
            self.emit(IR::Jump(lab));

            let ed = self.gen_expr_block(*ifelse.0)?;
            let pl = self.take_first(ed)?;
            self.must_allocated_at(pl, reg)?;

            self.emit(IR::Jump(end));
        }
        self.emit(IR::Label(lab));
        let blk = else_.ok_or(DukaIRError::from(DukaIRErrorKind::InvalidAST(
            "No else block found".into(),
        )))?;
        let ed = self.gen_expr_block(*blk)?;
        let pl = self.take_first(ed)?;
        self.must_allocated_at(pl, reg)?;
        self.emit(IR::Label(end));
        Ok(Place::R(self.get_reg(reg)?))
    }

    fn do_table_to(
        &mut self,
        reg: ToReg,
        fields: Vec<Field>,
        is_array: bool,
    ) -> Result<Place, DukaIRError> {
        let table = self.get_reg(reg)?;
        if is_array {
            self.emit(IR::NewArray(table));
        } else {
            self.emit(IR::NewTable(table));
        }
        let clean_from = self.allocator.top();
        let mut fields = fields.into_iter();
        while let Some(field) = fields.next() {
            match field {
                Field::KeyValue(k, v) => {
                    let k = self.do_expr(k)?;
                    // Consume `k` before evaluating `v`: a `Call` in `k` needs
                    // its `Take` emitted immediately after it.
                    let kpl = self.take_first(k)?;
                    let v = self.do_expr(v)?;

                    let key = self.without_up_val(kpl, ToReg::To(self.allocator.top()))?;
                    let vpl = self.take_first(v)?;
                    let val = self.without_up_val(vpl, ToReg::To(self.allocator.top() + 1))?;

                    self.emit(IR::SetField(TablePlace::R(table), key, val));
                }
                Field::NameValue((n, _), v) => {
                    let k = self.constants.push(n.into());
                    let v = self.do_expr_to(v, ToReg::New)?;

                    let pl = self.take_first(v)?;
                    let val = self.without_up_val(pl, ToReg::Temp)?;

                    self.emit(IR::SetField(TablePlace::R(table), ValuePlace::K(k), val));
                }
                Field::Value(v) => {
                    let mut batch = vec![v];
                    while let Some(Field::Value(v)) = fields.next() {
                        batch.push(v);
                    }
                    // 数组项必须从 table+1 起连续布局(嵌套表/数组求值会推高) table与其元素间不得有空洞的地方
                    let exp = self.do_consecutive_from(batch, table + 1)?;
                    let (start, count) = self.take_all(exp)?;
                    assert_eq!(start, table + 1);
                    self.emit(IR::Array(table, count));
                }
            }
        }
        self.allocator.free_many(clean_from..);
        Ok(Place::R(table))
    }

    /// # Never Move An Allocated Variable!
    ///
    #[inline(always)]
    fn gen_move(&mut self, to: Reg, from: Reg) {
        if to != from {
            self.emit(IR::Move(to, from));
            // 只有当 `from` 不是仍在作用域内的局部变量寄存器时才释放:
            // 局部变量的寄存器由作用域持有,若提前归还分配器,后续 alloc
            // 可能复用该寄存器覆盖变量值(如 `a.x = a` 中 RHS 的 a)
            if !self.scopes.is_local_reg(from) && !self.scopes.is_captured(from) {
                self.allocator.free(from);
            }
        }
    }

    fn gen_assign(&mut self, left: LValue, val: ValuePlace) -> Result<(), DukaIRError> {
        match left {
            LValue::Global(env, key) => self.emit(IR::SetField(env, ValuePlace::K(key), val)),
            LValue::SetByKey(tab, key) => self.emit(IR::SetField(tab, ValuePlace::K(key), val)),
            LValue::Local(to) => {
                self.must_allocated_at(val.into(), ToReg::To(to))?;
            }
            LValue::NewLocal(name) => {
                let reg = self.ensure_allocated(val.into(), ToReg::New)?;
                self.scopes.declare_local(&name, reg)?;
            }
            LValue::UpVal(u) => {
                let reg = self.ensure_allocated(val.into(), ToReg::New)?;
                self.emit(IR::SetUpVal(u, reg))
            }
            LValue::SetByIndex(tab, idx) => self.emit(IR::SetField(tab, idx, val)),
        }

        Ok(())
    }

    #[inline]
    fn enter(&mut self, is_func: bool) {
        self.labels.enter(is_func);
        self.scopes.enter(is_func);
        if is_func {
            self.allocator.enter();
        }
    }
    #[inline]
    fn exit_func(&mut self) -> Result<Box<[UpIndex]>, DukaIRError> {
        self.used_reg_count = self.allocator.used_reg_count();
        self.allocator.exit();

        let scope = self.scopes.exit();

        let Scope::Function { up_vals, .. } = scope else {
            unreachable!()
        };

        for (at, to) in self.labels.resolve_and_exit()? {
            self.emit_fixup(at, IR::Jump(to));
        }

        Ok(up_vals.into_iter().map(|v| v.1).collect())
    }
    #[inline]
    fn exit_block(&mut self) -> Result<(), DukaIRError> {
        let scope = self.scopes.exit();

        if let Scope::Block { locals, .. } = scope {
            for (_, reg) in locals {
                // 被闭包捕获的 local 寄存器必须保留,否则后续 alloc 复用会破坏 open upvalue
                if !self.scopes.is_captured(reg) {
                    self.allocator.free(reg);
                }
            }
        }

        for (at, to) in self.labels.resolve_and_exit()? {
            self.emit_fixup(at, IR::Jump(to));
        }

        Ok(())
    }

    fn gen_func_block(
        &mut self,
        body: FuncBody,
        self_call: bool,
        name: Option<String>,
        span: Span,
    ) -> Result<DukaIR, DukaIRError> {
        let has_var_arg = body.has_var_arg();
        let FuncBody(params, _, _, blk) = body;
        let Block(stmts, ret) = *blk;
        // 方法定义 `function t:m(a)` 时 self 是隐式第一参数,R0 由调用方传入
        // `...` 不计入定长参数(param_count),由 VarArgPrepare 收集变长部分
        let param_count = params
            .iter()
            .filter(|p| !matches!(p, Param::Var(_)))
            .count()
            + (self_call as usize);

        let mut irg = Self::new(self.config.clone(), self.source_info.clone());
        std::mem::swap(&mut irg.scopes, &mut self.scopes);
        irg.constants = Constants::default();
        irg.enter(true);

        if self_call {
            let self_reg = irg.allocator.alloc()?;
            irg.scopes.declare_local(cgen::SELF, self_reg)?;
        }
        for param in params {
            match param {
                Param::Name((name, _)) | Param::Typed((name, _), _) => {
                    let reg = irg.allocator.alloc()?;
                    irg.scopes.declare_local(&name, reg)?; //NOTICE, there already exist values
                }
                _ => break,
            }
        }

        irg.gen_stmts(stmts.to_vec())?;
        if let Some(ret) = ret
            && let StmtKind::Return(mut items) = (*ret).0
        {
            let span = (*ret).1;
            let start = irg.instructions.len();

            // 尾调用优化仅当 return 的唯一/最后表达式是一个函数调用:
            // `return f()`(单个元素)才走;`return f(), 10` 等必须整体求值。
            if let [Expr(ExprKind::Call(callee, params), _)] = items.as_mut() {
                let callee = std::mem::take(callee);
                let params = std::mem::take(params).to_vec();

                let ed = irg.gen_call(*callee, params, true)?;
                let (start_reg, count) = irg.take_all(ed)?;
                irg.emit(IR::Return(start_reg, count));
            } else {
                irg.gen_return(items.into())?;
            }

            let end = irg.instructions.len();
            irg.inst_spans.push((start..end, span));
        } else {
            // 函数体没有显式 return:必须补尾部 Return0
            irg.emit(IR::Return(0, ValueCount::Exact(0)));
        }

        let up_indexes = irg.exit_func()?;
        std::mem::swap(&mut irg.scopes, &mut self.scopes);

        Ok(DukaIR {
            has_var_arg,
            param_count,
            reg_lifetime: RegLifetime {
                count: irg.used_reg_count,
                using: irg.using_regs.into_boxed_slice(),
            },
            nesteds: irg.nesteds.into(),
            instructions: irg.instructions.into(),
            constants: Box::new(irg.constants),
            up_indexes,
            debug_info: Box::new(DebugInfo {
                inst_spans: irg.inst_spans.into(),
                all_span: span,
                debug_name: name.map(|s| s.into_boxed_str()),
                source_info: self.source_info.clone(),
            }),
            logic: None,
            label_names: Box::new(irg.labels.into_names()),
        })
    }

    fn gen_expr_block(&mut self, Block(stmts, ret): Block) -> Result<ExpDesc, DukaIRError> {
        self.enter(false);

        for stmt in stmts {
            self.gen_stmt(stmt)?;
        }

        let Some(ret) = ret else {
            return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(
                "No return in expr block".into(),
            )));
        };
        let StmtKind::Return(items) = (*ret).0 else {
            return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(
                "No return expr at the end of expr block".into(),
            )));
        };

        let exp = self.do_consecutive_top(items.to_vec())?;

        self.exit_block()?;

        Ok(exp)
    }

    fn ensure_const(&mut self, expr: ExprKind) -> Result<ConstValue, DukaIRError> {
        if let ExprKind::Literal(cv) = expr {
            Ok(cv)
        } else {
            Err(DukaIRError::from(DukaIRErrorKind::NotConstExpr))
        }
    }
    #[inline]
    fn take_first_allocated(&mut self, exp: ExpDesc) -> Result<Reg, DukaIRError> {
        let pl = self.take_first(exp)?;
        self.ensure_allocated(pl, ToReg::New)
    }

    fn gen_skip_next(&mut self, cond: Expr, when: bool) -> Result<(), DukaIRError> {
        let exp = self.do_expr(cond)?;
        let reg = self.take_first_allocated(exp)?;
        self.emit(IR::SkipNext(reg, when));
        self.allocator.free(reg);
        Ok(())
    }

    fn gen_stmt(&mut self, Stmt(stmt, span): Stmt) -> Result<(), DukaIRError> {
        use StmtKind::*;

        if stmt.is_empty() {
            return Ok(());
        }
        stmt.is_sugar().then_error(|| {
            DukaIRErrorKind::UnsupportedFeature(stmt.to_string().into_boxed_str())
        })?;
        matches!(stmt, Return(..)).then_error(|| {
            DukaIRErrorKind::InvalidAST(
                "Invalid return statement, it must be the last statement".into(),
            )
        })?;

        match stmt {
            Label(label) => {
                let lab = self.labels.new_label(Some(label))?;
                self.emit(IR::Label(lab))
            }
            Goto(to) => {
                let who = self.emit_placeholder();
                self.labels.new_goto(who, to);
            }

            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);

                let to_end = self.labels.new_label(None)?;

                self.gen_skip_next(*if_.1, true)?;
                let mut lab = self.labels.new_label(None)?;
                self.emit(IR::Jump(lab));

                self.gen_block_scoped(*if_.0, false)?;
                self.emit(IR::Jump(to_end));

                for ifelse in ifelses {
                    self.emit(IR::Label(lab));

                    self.gen_skip_next(*ifelse.1, true)?;
                    lab = self.labels.new_label(None)?;
                    self.emit(IR::Jump(lab));

                    self.gen_block_scoped(*ifelse.0, false)?;
                    self.emit(IR::Jump(to_end));
                }

                self.emit(IR::Label(lab));
                if let Some(blk) = else_ {
                    self.gen_block_scoped(*blk, false)?;
                }

                self.emit(IR::Label(to_end));
            }

            While(cond, blk) => {
                let start = self.labels.new_label(None)?;
                let end = self.labels.new_label(None)?;
                self.labels.new_loop(start, end);
                self.emit(IR::Label(start));

                self.gen_skip_next(*cond, true)?;
                self.emit(IR::Jump(end));

                self.gen_block_scoped(*blk, false)?;

                self.emit(IR::Jump(start));
                self.emit(IR::Label(end));
                self.labels.exit_loop();
            }
            // 注意, 此处vars不包含(bool, ...)的bool, bool仅内部可见, See docs/stdlib.md
            ForGeneric(vars, from, blk) => {
                if from.len() != 1 {
                    return Err(DukaIRError::from(DukaIRErrorKind::Custom(
                        "Generic for-loop requires exactly one iterator expression".into(),
                    )));
                }
                const GENERATOR_RESULTS: usize = 3; // R[a..a+2] 头部: a 迭代器, a+1/a+2 预留
                const TFORK_CALL_SLOTS: usize = 3; // TForCall 调用协议: a+3 调用槽, a+4.. 返回值

                let start = self.labels.new_label(None)?;
                let to_call = self.labels.new_label(None)?;
                let end = self.labels.new_label(None)?;
                // continue 跳 to_call(重新取下一个值),否则会重复当前迭代
                self.labels.new_loop(to_call, end);

                // R[a] = iterator
                // R[a+3..] = 循环变量: a+3 是 bool, a+4.. 是值; block 整体预留,见 docs/stdlib.md
                let n = vars.len();
                let block_size = GENERATOR_RESULTS + (n + 1).max(TFORK_CALL_SLOTS);
                let a = self.allocator.top();
                let ed = self.do_consecutive_top(from.to_vec())?;
                self.allocator
                    .alloc_consecutive_from(a, block_size)?
                    .count();

                let pl = self.take_many(ed, 1)?.remove(0);
                let reg = self.ensure_allocated(pl, ToReg::New)?;
                if reg != a {
                    self.gen_move(a, reg);
                }

                let locals = vars
                    .into_iter()
                    .enumerate()
                    .map(|(i, var)| match var {
                        Path::Base((name, _)) => Ok::<_, DukaIRError>((name, a + 3 + 1 + i)),
                        _ => Err(DukaIRErrorKind::InvalidAST(
                            format!("Invalid variable name in generic for-loop: {var}").into(),
                        )
                        .into()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                self.emit(IR::TForPrep(a, to_call));
                self.emit(IR::Label(start));

                self.gen_block_with_locals(*blk, false, locals)?;

                self.emit(IR::Label(to_call));
                self.emit(IR::TForCall(a, n));
                self.emit(IR::TForLoop(a, start));

                self.emit(IR::Label(end));
                self.labels.exit_loop();

                self.allocator.free_many(a..a + block_size);
            }
            ForNumeric(var, from, end, step, blk) => {
                let to_start = self.labels.new_label(None)?;
                let to_continue = self.labels.new_label(None)?;
                let to_end = self.labels.new_label(None)?;
                // continue 必须跳到 ForLoop(递增+检查)之前,而非循环体开头,
                // 否则循环变量不前进会死循环
                self.labels.new_loop(to_continue, to_end);

                let (var_name, var_span) = match var {
                    Path::Base((name, span)) => (name, span),
                    _ => {
                        return Err(DukaIRErrorKind::InvalidAST(
                            "Invalid variable name in numeric for-loop".into(),
                        )
                        .into());
                    }
                };
                // VM 的 ForPrepare/ForLoop 协议要求 init/limit/step 在三个
                // 连续寄存器 R[a], R[a+1], R[a+2],因此必须连续求值。
                let a = self.allocator.top();
                let step_expr = match step {
                    Some(s) => *s,
                    None => {
                        crate::parser::ast::Expr(ExprKind::Literal(ConstValue::Int(1)), var_span)
                    }
                };
                let ed = self.do_consecutive_from(vec![*from, *end, step_expr], a)?;
                let mut control = self.take_many(ed, 3)?;
                let step = self.ensure_allocated(control.pop().unwrap(), ToReg::New)?;
                let end = self.ensure_allocated(control.pop().unwrap(), ToReg::New)?;
                let from = self.ensure_allocated(control.pop().unwrap(), ToReg::New)?;

                self.emit(IR::ForPrep(from, to_end)); //ForPrep
                self.emit(IR::Label(to_start));

                self.gen_block_with_locals(*blk, false, vec![(var_name, from)])?;

                self.emit(IR::Label(to_continue));
                self.emit(IR::ForLoop(from, to_start));

                self.emit(IR::Label(to_end));
                self.labels.exit_loop();

                self.allocator
                    .free_many(iter::once(end).chain(iter::once(step)));
            }

            Do(blk) => {
                self.gen_block_scoped(*blk, false)?;
            }
            Function(name, _attrs, body, global) => {
                // For `local function`, bind the name to a local register BEFORE
                // generating the body, otherwise the self-recursion inside the
                // body resolves to the global env (`_ENV.f`) and reads nil.
                let name_str = name.to_string();
                let pre_alloc = if global {
                    None
                } else if let Path::Base(..) = name {
                    let reg = self.allocator.alloc()?;
                    self.scopes.declare_local(&name_str, reg)?;
                    Some(reg)
                } else {
                    None
                };

                let ir = self.gen_func_block(*body, name.is_self_call(), Some(name_str), span)?;

                self.nesteds.push(ir);
                let reg = match pre_alloc {
                    Some(r) => r,
                    None => self.allocator.alloc()?,
                };
                self.emit(IR::Closure(reg, self.nesteds.len() - 1));
                let assign_to = self.set_to_path(name, global)?;

                self.gen_assign(assign_to, ValuePlace::R(reg))?;
            }

            Define(attr_names, vals, global) => {
                let (consts, normals): (Vec<_>, Vec<_>) = attr_names
                    .into_iter()
                    .zip(vals.into_iter().map(Some).chain(iter::repeat(None)))
                    .map(|((((name, _), attrs, _ty), _), expr)| ((name, attrs), expr))
                    .partition(|((_, attrs), expr)| {
                        has_attr(&attrs, catt::CONST) && expr.is_some()
                    });

                for ((name, _), expr) in consts {
                    let cv = self.ensure_const(expr.unwrap().0)?;
                    self.scopes.declare_const(&name, self.constants.push(cv));
                }

                let (attr_names, exprs): (Vec<_>, Vec<_>) = normals.into_iter().unzip();

                let desc = self.do_consecutive_top(exprs.into_iter().map_while(|i| i).collect())?;
                let mut pls = self.take_many(desc, attr_names.len())?.into_iter();

                for (name, _) in attr_names {
                    let pl = pls
                        .next()
                        .map(Ok)
                        .unwrap_or_else(|| /*unreachable*/self.load_nil().map(Place::R))?;

                    if global {
                        let left = self.set_to_path(Path::Base((name, Span::EMPTY)), true)?;
                        let val = self.without_up_val(pl, ToReg::New)?;
                        self.gen_assign(left, val)?;
                    } else {
                        let wh = self.ensure_allocated(pl, ToReg::New)?;
                        self.scopes.declare_local(&name, wh)?;
                    }
                }
            }
            Assign(names, exprs) => {
                let needs = names.len();
                let mut exprs = exprs.to_vec();
                assert!(needs > 0);

                let lefts = names
                    .into_iter()
                    .map(|path| {
                        // A declared `local`/upvalue wins over the default-to-
                        // global policy (`var_default_local = false`): `local x;
                        // x = 1` must still write to the local, otherwise the
                        // write goes to the globals table and the local keeps
                        // its old value forever.
                        if let Path::Base((name, _)) = &path
                            && let Some(pl) = self.scopes.find(name)
                        {
                            match pl {
                                Place::K(_) | Place::I(_) => Err(DukaIRError::from(
                                    DukaIRErrorKind::TryAssignConst(name.clone().into_boxed_str()),
                                )),
                                Place::R(r) => Ok(LValue::Local(r)),
                                Place::U(u) => Ok(LValue::UpVal(u)),
                            }
                        } else {
                            self.set_to_path(path, !self.config.var_default_local)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                exprs.truncate(needs);

                fn check_consecutive(lefts: &[LValue]) -> Option<Reg> {
                    let mut iter = lefts.iter();
                    let Some(LValue::Local(start)) = iter.next() else {
                        return None;
                    };
                    iter.all(|i| matches!(i, LValue::Local(r) if *r == start + 1))
                        .then_some(*start)
                }
                let ed = if let Some(start) = check_consecutive(&lefts) {
                    self.do_consecutive_from(exprs, start)?
                } else {
                    self.do_consecutive_top(exprs)?
                };

                let mut vals = self.take_many(ed, needs)?.into_iter();
                for left in lefts {
                    let pl = vals.next().expect("WTF"); // this is unreachable, all work done in take_many
                    let val = self.without_up_val(pl, ToReg::Temp)?;
                    self.gen_assign(left, val)?;
                }
            }
            Break => {
                let (_, end) =
                    self.labels
                        .get_loop()
                        .ok_or(DukaIRError::from(DukaIRErrorKind::OutOfLoop(
                            "break".into(),
                        )))?;
                self.emit(IR::Jump(end))
            }
            Continue => {
                let (start, _) =
                    self.labels
                        .get_loop()
                        .ok_or(DukaIRError::from(DukaIRErrorKind::OutOfLoop(
                            "continue".into(),
                        )))?;
                self.emit(IR::Jump(start))
            }
            Expr(e) => {
                let from = self.allocator.top();
                let ed = self.do_expr(*e)?;
                self.take_none(ed);
                self.allocator.free_many(from..);
            }
            Call(callee, params) => {
                let from = self.allocator.top();
                let ed = self.gen_call(*callee, params.into_vec(), false)?;
                self.take_none(ed);
                self.allocator.free_many(from..);
            }
            TypeAlias(..) => {}
            TypeFunction(..) => {}
            InlineTypeFunction(..) => {}
            _ => {
                unreachable!()
            }
        }

        Ok(())
    }

    fn gen_main(&mut self, blk: Block) -> Result<Box<[UpIndex]>, DukaIRError> {
        let up_indexes = self.gen_block_scoped(blk, true)?;
        self.emit(IR::Return(0, ValueCount::Exact(0))); // ensure
        Ok(up_indexes.expect("WTF"))
    }
}

impl DukaGenerator<DukaIR> for IRGenerator {
    type InputType = DukaChunk;
    type ConfigType = DukaIRConfig;

    fn generate(input: Self::InputType, config: Self::ConfigType) -> Result<DukaIR, DukaIRError> {
        let mut generator = Self::new(config, input.source_info);
        let up_indexes = generator.gen_main(input.block)?;

        Ok(DukaIR {
            param_count: 0,
            reg_lifetime: RegLifetime {
                count: generator.used_reg_count,
                using: generator.using_regs.into_boxed_slice(),
            },
            has_var_arg: true,
            instructions: generator.instructions.into(),
            nesteds: generator.nesteds.into(),
            constants: Box::new(generator.constants),
            up_indexes,
            debug_info: Box::new(DebugInfo {
                source_info: generator.source_info,
                inst_spans: generator.inst_spans.into(),
                all_span: input.span,
                debug_name: Some(cgen::MAIN.into()),
            }),
            logic: Some(input.logic),
            label_names: Box::new(generator.labels.into_names()),
        })
    }
}
