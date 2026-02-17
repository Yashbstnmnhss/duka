use std::iter;

use duka_shared::{
    ast::{BinOp, Block, Expr, ExprKind, Field, FuncBody, Param, Path, PathSuffix, Stmt, StmtKind},
    constants::{catt, ccallish, cgen},
    error::{DukaIRError, DukaIRErrorKind, Span},
    ir::{
        Allocator, Constants, Cst, DukaIR, ExpDesc, IR, Labels, Place, Reg, Scope, Scopes,
        TablePlace, UpIndex, ValuePlace,
    },
    types::{DebugInfo, DukaChunk, DukaGenerator, ValueCount},
    utils::{OrError, is_consecutive},
    value::ConstValue,
};

#[derive(Debug)]
pub struct IRGenerator {
    allocator: Allocator,
    labels: Labels,

    instructions: Vec<IR>,
    nesteds: Vec<DukaIR>,
    constants: Constants,
    scopes: Scopes,
    debug_info: DebugInfo,
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
        Self::new()
    }
}

impl IRGenerator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            labels: Labels::new(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            nesteds: vec![],
            used_reg_count: 0,
        }
    }

    #[inline]
    fn emit(&mut self, ir: IR) {
        self.instructions.push(ir);
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

            self.gen_stmt(Stmt(stmt, span))?;

            let end = self.instructions.len();
            self.debug_info.inst_spans.push((start..end, span));
        }
        Ok(())
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

            let eds = self.do_consecutive_top(items.to_vec())?;
            let (start_reg, count) = self.take_all(eds)?;
            self.emit(IR::Return(start_reg, count));

            let end = self.instructions.len();
            self.debug_info.inst_spans.push((start..end, span));
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
            ConstValue::ConstTable(array_map) => {
                let idx = self.constants.push(ConstValue::ConstTable(array_map));
                self.emit(IR::LoadConst(reg, idx));
            }
            ConstValue::String(items) => {
                self.emit(IR::LoadString(reg, items));
            }
        }
    }

    fn take_first(&mut self, exp: ExpDesc) -> Result<Place, DukaIRError> {
        match exp {
            ExpDesc::Single(pl) => Ok(pl),
            ExpDesc::Many(fixeds, vararg) => {
                let mut fixeds = fixeds.into_iter();
                if let Some(reg) = fixeds.next() {
                    self.allocator.free_many(fixeds);
                    Ok(Place::R(reg))
                } else if let Some(start) = vararg {
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
            ExpDesc::Many(fixeds, vararg) => {
                let fixed_count = fixeds.len();
                many.extend(fixeds.into_iter().take(needs).map(Place::R));

                if let Some(start) = vararg {
                    if fixed_count < needs {
                        let rest = needs - fixed_count;
                        many.extend(self.allocator.alloc_consecutive(start, rest)?.map(Place::R));
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
            ExpDesc::Many(fixeds, vararg) => {
                let fixed_count = fixeds.len();
                if let Some(start) = vararg {
                    assert!(is_consecutive(&[fixeds.as_slice(), &[start]].concat()));
                    let fixed_start = fixeds.iter().min().cloned();
                    self.emit(IR::TakeAll);
                    Ok((fixed_start.unwrap_or(start), ValueCount::VarArg))
                } else {
                    assert!(is_consecutive(&fixeds), "{fixeds:?}");
                    let fixed_start = fixeds.iter().min().cloned();
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
            ExpDesc::Many(fixeds, vararg) => {
                self.allocator.free_many(fixeds.into_iter());
                if let Some(start) = vararg {
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
                                    name,
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
                let base = self.get_path_to(*parent, global_first, ToReg::New)?;
                let table = self.only_modifiable(base)?;
                match suffix {
                    PathSuffix::Colon(func) => {
                        return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(format!(
                            "trying to assign value(s) to a function self-calling ({})",
                            func.0
                        ))));
                    }
                    PathSuffix::Dot((name, _)) => {
                        LValue::SetByKey(table, self.constants.push(name.into()))
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
                    "trying to assign value(s) to an expression".to_owned(),
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
                        let exp = self.do_expr_to(*idx, to_reg)?;
                        let reg = self.get_reg(to_reg)?;

                        let idx_pl = self.take_first(exp)?;
                        let idx = self.without_up_val(idx_pl, ToReg::To(reg))?;

                        self.emit(IR::GetField(reg, table, idx));
                        Place::R(reg)
                    }
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
                    kind: DukaIRErrorKind::TryModifyReadonly(pl.to_string()),
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

    fn gen_params(&mut self, params: Vec<Expr>) -> Result<ValueCount, DukaIRError> {
        let exp = self.do_consecutive_top(params)?;
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
        to_reg: ToReg,
    ) -> Result<ExpDesc, DukaIRError> {
        let callish = callee.0.is_callish_keyword();
        let self_call = callee.0.is_self_call();

        let expr_len = params.len();

        let callee = if callish.is_some() {
            self.allocator.alloc_temp()?
        } else {
            let exp = self.do_expr_to(callee, to_reg)?;
            let pl = self.take_first(exp)?;
            self.ensure_allocated(pl, to_reg)?
        };

        if let Some(ccallish::SPAWN) = callish {
            (expr_len != 1).then_error(|| {
                DukaIRError::from(DukaIRErrorKind::InvalidParams(
                    ccallish::SPAWN.to_owned(),
                    1,
                    expr_len,
                ))
            })?;

            let func = self.do_consecutive_top(params)?;
            let one = self.take_first_allocated(func)?;

            self.emit(IR::Spawn(callee, one));
            return Ok(ExpDesc::Single(Place::R(callee)));
        }

        // Params
        if self_call {
            self.emit(IR::SelfParam());
        }
        let mut count = self.gen_params(params)?;
        if self_call {
            count = count + 1;
        }

        // Call
        if let Some(callish) = callish {
            (expr_len < 1).then_error(|| {
                DukaIRError::from(DukaIRErrorKind::InvalidParams(
                    ccallish::SPAWN.to_owned(),
                    1,
                    expr_len,
                ))
            })?;
            self.allocator.ensure_allocated(callee)?;
            self.emit(match callish {
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

    /// - reg: target register (if has, or allocate new one)
    /// - keep_im: whether `ConstValue` should be allocated or not
    fn do_expr_to(&mut self, Expr(expr, span): Expr, reg: ToReg) -> Result<ExpDesc, DukaIRError> {
        use ExprKind::*;

        expr.is_sugar().then_error(|| {
            DukaIRError::from(DukaIRErrorKind::UnsupportedFeature(expr.to_string()))
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
                self.emit(IR::SysCall(sys_call));
                let reg = self.get_reg(reg)?;
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Table(fields) => self.do_table_to(reg, fields.to_vec())?,
            Function(func_body) => {
                let mut ir = self.gen_func_block(func_body, false)?;
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
                    if let Binary(le2, re2, BinOp::Concat) = std::mem::take(&mut current).0 {
                        exprs.push(*le2);
                        current = *re2;
                    } else {
                        exprs.push(std::mem::take(&mut current));
                        break;
                    }
                }

                let count = exprs.len();
                let ed = self.do_consecutive_from(exprs, start + 1)?;
                assert!(is_consecutive(
                    &self
                        .take_many(ed, count)?
                        .into_iter()
                        .enumerate()
                        .map(|(i, pl)| self.ensure_allocated(pl, ToReg::To(start + i + 1)))
                        .collect::<Result<Vec<_>, _>>()?,
                ));

                let reg = self.get_reg(reg)?;
                self.emit(IR::Concat(reg, start, count));

                Place::R(reg)
            }
            Binary(le, re, bin_op) => self.do_binary_to(reg, le, re, bin_op)?,
            If(ifs) => self.do_if_to(reg, *ifs)?,
            _ => unreachable!(),
        }))
    }

    fn do_binary_to(
        &mut self,
        reg: ToReg,
        le: Box<Expr>,
        re: Box<Expr>,
        bin_op: BinOp,
    ) -> Result<Place, DukaIRError> {
        let reg = self.get_reg(reg)?;
        if bin_op.is_short() {
            let le = self.do_expr_to(*le, ToReg::To(reg))?;
            let lp = self.take_first(le)?;
            self.must_allocated_at(lp, ToReg::To(reg))?;

            let lab = self.labels.new_label(None);
            self.emit(IR::SkipNext(reg, matches!(bin_op, BinOp::Or)));
            self.emit(IR::Jump(lab));

            let re = self.do_expr_to(*re, ToReg::To(reg))?;
            let rp = self.take_first(re)?;
            self.must_allocated_at(rp, ToReg::To(reg))?;
            self.emit(IR::Label(lab));
        } else {
            let left = self.do_expr_to(*le, ToReg::To(reg))?;
            let right = self.do_expr_to(
                *re,
                matches!(left, ExpDesc::Single(Place::I(..)))
                    .then_some(ToReg::To(reg))
                    .unwrap_or(ToReg::Temp),
            )?;

            let left = self.take_first(left)?;
            let left = self.without_up_val(left, ToReg::To(reg))?;

            let right = self.take_first(right)?;
            let right = self.without_up_val(right, ToReg::Temp)?;

            self.emit(IR::Binary(reg, left, right, bin_op));
        }
        Ok(Place::R(reg))
    }

    fn do_if_to(&mut self, reg: ToReg, ifs: duka_shared::ast::If) -> Result<Place, DukaIRError> {
        let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);
        let end = self.labels.new_label(None);
        self.gen_skip_next(*(if_.1), true)?;
        let mut lab = self.labels.new_label(None);
        self.emit(IR::Jump(lab));
        let ed = self.gen_expr_block(*if_.0)?;
        let pl = self.take_first(ed)?;
        self.must_allocated_at(pl, reg)?;
        self.emit(IR::Jump(end));
        for ifelse in ifelses {
            self.emit(IR::Label(lab));

            self.gen_skip_next(*(ifelse.1), true)?;
            lab = self.labels.new_label(None);
            self.emit(IR::Jump(lab));

            let ed = self.gen_expr_block(*ifelse.0)?;
            let pl = self.take_first(ed)?;
            self.must_allocated_at(pl, reg)?;

            self.emit(IR::Jump(end));
        }
        self.emit(IR::Label(lab));
        let blk = else_.ok_or(DukaIRError::from(DukaIRErrorKind::InvalidAST(
            "No else block found".to_string(),
        )))?;
        let ed = self.gen_expr_block(*blk)?;
        let pl = self.take_first(ed)?;
        self.must_allocated_at(pl, reg)?;
        self.emit(IR::Label(end));
        Ok(Place::R(self.get_reg(reg)?))
    }

    fn do_table_to(&mut self, reg: ToReg, fields: Vec<Field>) -> Result<Place, DukaIRError> {
        let table = self.get_reg(reg)?;
        self.emit(IR::NewTable(table));
        let clean_from = self.allocator.top();
        let mut fields = fields.into_iter();
        while let Some(field) = fields.next() {
            match field {
                Field::KeyValue(k, v) => {
                    let k = self.do_expr(k)?;
                    let v = self.do_expr(v)?;

                    let kpl = self.take_first(k)?;
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
                    let exp = self.do_consecutive_top(batch)?;
                    let (start, count) = self.take_all(exp)?;
                    assert!(start == table + 1);
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
            self.allocator.free(from);
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
                self.allocator.free(reg);
            }
        }

        for (at, to) in self.labels.resolve_and_exit()? {
            self.emit_fixup(at, IR::Jump(to));
        }

        Ok(())
    }

    fn gen_func_block(&mut self, body: FuncBody, self_call: bool) -> Result<DukaIR, DukaIRError> {
        let has_var_arg = body.has_vararg();
        let FuncBody(params, blk) = body;
        let Block(stmts, ret) = *blk;
        let param_count = params.len();

        let mut irg = Self::new();
        irg.scopes = self.scopes.clone();
        irg.constants = Constants::default();
        irg.enter(true);

        if self_call {
            irg.scopes.declare_local(cgen::SELF, 1)?;
        }
        for param in params {
            match param {
                Param::Name((name, _)) => {
                    irg.scopes.declare_local(&name, irg.allocator.alloc()?)?; //NOTICE, there already exist values
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

            let (start_reg, count) =
                if let [Expr(ExprKind::Call(callee, params), _), ..] = items.as_mut() {
                    let callee = std::mem::take(callee);
                    let params = std::mem::take(params).to_vec();

                    let ed = irg.gen_call(*callee, params, true)?;
                    irg.take_all(ed)?
                } else {
                    let eds = irg.do_consecutive_top(items.to_vec())?;
                    irg.take_all(eds)?
                };
            irg.emit(IR::Return(start_reg, count));

            let end = irg.instructions.len();
            irg.debug_info.inst_spans.push((start..end, span));
        }

        let up_indexes = irg.exit_func()?;

        Ok(DukaIR {
            has_var_arg,
            param_count,
            used_reg_count: irg.used_reg_count,
            nesteds: irg.nesteds,
            instructions: irg.instructions,
            constants: Box::new(irg.constants),
            up_indexes,
            debug_info: Box::new(irg.debug_info),
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
                "No return in expr block".to_owned(),
            )));
        };
        let StmtKind::Return(items) = (*ret).0 else {
            return Err(DukaIRError::from(DukaIRErrorKind::InvalidAST(
                "No return expr at the end of expr block".to_owned(),
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
        stmt.is_sugar()
            .then_error(|| DukaIRErrorKind::UnsupportedFeature(stmt.to_string()))?;
        matches!(stmt, StmtKind::Return(..)).then_error(|| {
            DukaIRErrorKind::InvalidAST(
                "Invalid return statement, it must be the last statement".to_owned(),
            )
        })?;

        match stmt {
            Label(label) => {
                let lab = self.labels.new_label(Some(label));
                self.emit(IR::Label(lab))
            }
            Goto(to) => {
                let who = self.emit_placeholder();
                self.labels.new_goto(who, to);
            }

            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);

                let to_end = self.labels.new_label(None);

                self.gen_skip_next(*if_.1, true)?;
                let mut lab = self.labels.new_label(None);
                self.emit(IR::Jump(lab));

                self.gen_block_scoped(*if_.0, false)?;
                self.emit(IR::Jump(to_end));

                for ifelse in ifelses {
                    self.emit(IR::Label(lab));

                    self.gen_skip_next(*ifelse.1, true)?;
                    lab = self.labels.new_label(None);
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
                let start = self.labels.new_label(None);
                let end = self.labels.new_label(None);
                self.labels.new_loop(start, end);
                self.emit(IR::Label(start));

                self.gen_skip_next(*cond, true)?;
                self.emit(IR::Jump(end));

                self.gen_block_scoped(*blk, false)?;

                self.emit(IR::Jump(start));
                self.emit(IR::Label(end));
                self.labels.exit_loop();
            }
            ForGeneric(vars, from, blk) => {
                let start = self.labels.new_label(None);
                let to_call = self.labels.new_label(None);
                let end = self.labels.new_label(None);
                self.labels.new_loop(start, end);

                let ed = self.do_consecutive_top(from.to_vec())?;
                let generator = self
                    .take_many(ed, 4)?
                    .into_iter()
                    .map(|pl| self.ensure_allocated(pl, ToReg::New))
                    .collect::<Result<Vec<Reg>, _>>()?
                    .first()
                    .cloned()
                    .ok_or(DukaIRError::from(DukaIRErrorKind::InvalidAST(format!(
                        "Invalid forloop structure"
                    ))))?;
                let locals = vars
                    .into_iter()
                    .map(|var| match var {
                        Path::Base((name, _)) => self.allocator.alloc().map(|reg| (name, reg)),
                        _ => Err(DukaIRErrorKind::InvalidAST(format!(
                            "Invalid variable name in generic for-loop: {var}"
                        ))
                        .into()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let regs: Vec<_> = locals.iter().map(|i| i.1).collect();

                self.emit(IR::TForPrep(generator, to_call)); //TForPrep
                self.emit(IR::Label(start));

                self.gen_block_with_locals(*blk, false, locals)?;

                self.emit(IR::Label(to_call));
                self.emit(IR::TForCall(generator, regs.len()));
                self.emit(IR::TForLoop(generator, start));

                self.emit(IR::Label(end));
                self.labels.exit_loop();

                let range = generator..generator + 4;
                self.allocator.free_many(range);
                self.allocator.free_many(regs.into_iter());
            }
            ForNumberic(var, from, end, step, blk) => {
                let to_start = self.labels.new_label(None);
                let to_end = self.labels.new_label(None);
                self.labels.new_loop(to_start, to_end);

                let from = self.do_expr(*from)?;
                let end = self.do_expr(*end)?;
                let step = step
                    .map(|e| self.do_expr(*e))
                    .transpose()?
                    .unwrap_or(ExpDesc::Single(Place::I(1)));

                let from = self.take_first_allocated(from)?;
                let end = self.take_first_allocated(end)?;
                let step = self.take_first_allocated(step)?;

                self.emit(IR::ForPrep(from, to_end)); //ForPrep
                self.emit(IR::Label(to_start));

                self.gen_block_with_locals(
                    *blk,
                    false,
                    vec![match var {
                        Path::Base((name, _)) => (name, from),
                        _ => {
                            return Err(DukaIRErrorKind::InvalidAST(format!(
                                "Invalid variable name in numberic for-loop: {var}"
                            ))
                            .into());
                        }
                    }],
                )?;

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
                let mut ir = self.gen_func_block(*body, name.is_self_call())?;
                ir.debug_info.debug_name = Some(name.to_string());
                ir.debug_info.all_span = span;

                self.nesteds.push(ir);
                let reg = self.allocator.alloc()?;
                self.emit(IR::Closure(reg, self.nesteds.len() - 1));
                let assign_to = self.set_to_path(name, global)?;

                self.gen_assign(assign_to, ValuePlace::R(reg))?;
            }

            Define(attrnames, vals, global) => {
                let (consts, normals): (Vec<_>, Vec<_>) = attrnames
                    .into_iter()
                    .zip(vals.into_iter().map(Some).chain(std::iter::repeat(None)))
                    .map(|((((name, _), attrs), _), expr)| ((name, attrs), expr))
                    .partition(|((_, attrs), expr)| {
                        attrs.iter().any(|(a, _)| a == catt::CONST) && expr.is_some()
                    });

                for ((name, _), expr) in consts {
                    let cv = self.ensure_const(expr.unwrap().0)?;
                    self.scopes.declare_const(&name, self.constants.push(cv));
                }

                let (attrnames, exprs): (Vec<_>, Vec<_>) = normals.into_iter().unzip();

                let desc = self.do_consecutive_top(exprs.into_iter().map_while(|i| i).collect())?;
                let mut pls = self.take_many(desc, attrnames.len())?.into_iter();

                for (name, _) in attrnames {
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
                    .map(|path| self.set_to_path(path, false))
                    .collect::<Result<Vec<_>, _>>()?;
                exprs.truncate(needs);

                fn check_consecutive(lefts: &Vec<LValue>) -> Option<Reg> {
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
                            "break".to_owned(),
                        )))?;
                self.emit(IR::Jump(end))
            }
            Continue => {
                let (start, _) =
                    self.labels
                        .get_loop()
                        .ok_or(DukaIRError::from(DukaIRErrorKind::OutOfLoop(
                            "continue".to_owned(),
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

    fn generate(input: Self::InputType) -> Result<DukaIR, DukaIRError> {
        let mut generator = Self::new();
        let up_indexes = generator.gen_main(input.chunk)?;
        generator.debug_info.debug_name = Some(cgen::MAIN.to_owned());
        generator.debug_info.all_span = input.span;

        Ok(DukaIR {
            param_count: 0,
            used_reg_count: generator.used_reg_count,
            has_var_arg: true,
            instructions: generator.instructions,
            nesteds: generator.nesteds,
            constants: Box::new(generator.constants),
            up_indexes,
            debug_info: Box::new(generator.debug_info),
            logic: Some(input.logic),
            label_names: Box::new(generator.labels.into_names()),
        })
    }
}
