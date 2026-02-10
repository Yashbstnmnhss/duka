use std::collections::HashMap;
use std::{iter, usize};

use crate::DebugInfo;
use crate::codegen::types::{
    Allocator, Constants, Cst, DukaIR, ExpDesc, IR, Labels, Place, Reg, Scope, Scopes,
};
use crate::value::ValueCount;
use crate::{
    instructions::{Address, Bits17, DecodeInstruction, Instruction as I},
    value::DukaProto,
};
use duka_shared::ast::BinOp;
use duka_shared::constants::{catt, ccallish, cgen};
use duka_shared::error::{DukaCodegenError, Span};
use duka_shared::types::{DukaChunk, DukaGenerator};
use duka_shared::utils::{OrError, is_consecutive};
use duka_shared::value::ConstValue;
use duka_shared::{
    ast::{Block, Expr, ExprKind, Field, FuncBody, Param, Path, PathSuffix, Stmt, StmtKind},
    error::DukaCodegenErrorKind::{self},
};

pub mod binary;
pub mod logic;
pub mod types;

#[derive(Debug, Default)]
pub struct Linker {
    links: HashMap<usize, usize>,
}
#[derive(Debug, Clone, Copy)]
pub enum AffectAction {
    Insert,
    Remove,
}
impl Linker {
    pub fn link(&mut self, from: usize, to: usize) {
        self.links.insert(from, to);
    }
    pub fn find_affected(&self, action_at: usize) -> Vec<usize> {
        self.links
            .iter()
            .enumerate()
            .filter_map(|(idx, (from, to))| (from < &action_at || to < &action_at).then_some(idx))
            .collect()
    }
}

/// ### This `struct` represents two items:
/// - **Label**(name, target_pos)
/// - **PendingGoto**(target_name, goto_inst_pos)
#[derive(Debug)]
struct JumpInfo(String, usize);
#[derive(Debug, Default)]
struct Jumper {
    linker: Linker,

    labels: Vec<Vec<JumpInfo>>, // labels of scopes

    loop_heads: Vec<(usize, bool)>, // the start of every loop (contains itself)
    pending_breaks: Vec<Vec<usize>>, // position of pending breaks in loop scopes
    pending_gotos: Vec<JumpInfo>,   // all pending gotos (jump backwards)

    pending_onetime: Vec<usize>,     //一次性
    pending_branch: Vec<Vec<usize>>, //多对一
}
impl Jumper {
    pub fn new() -> Self {
        Self {
            linker: Linker::default(),
            labels: vec![vec![]],
            loop_heads: vec![],
            pending_gotos: vec![],
            pending_breaks: vec![],
            pending_onetime: vec![],
            pending_branch: vec![],
        }
    }

    pub fn branch_start(&mut self) {
        self.pending_branch.push(vec![]);
    }
    /// NOTICE: THIS IS ONLY BACKWARD
    /// 聚合 多起点一终点
    pub fn branch_jmp(&mut self, irs: &mut Vec<IR>) {
        if let Some(v) = self.pending_branch.last_mut() {
            v.push(irs.len());
        }
        irs.push(IR::default())
    }
    pub fn branch_end(&mut self, irs: &mut Vec<IR>) {
        if let Some(is) = self.pending_branch.pop() {
            for idx in is {
                let to = irs.len();
                self.linker.link(idx, to);
                //irs[idx] = IR::Jump(Self::calc_offset(to, idx));
            }
        }
    }

    /// NOTICE: THIS IS ONLY BACKWARD
    /// 单独 一起点一终点
    pub fn onetime_jmp(&mut self, irs: &mut Vec<IR>) {
        self.pending_onetime.push(irs.len());
        irs.push(IR::default());
    }

    pub fn onetime_end(&mut self, irs: &mut Vec<IR>) {
        let pos = self.pending_onetime.pop().expect("NO ONETIME START");
        let to = irs.len();
        self.linker.link(pos, to);
        //irs[pos] = IR::Jump(Self::calc_offset(to, pos));
    }

    pub fn enter(&mut self) {
        self.labels.push(vec![]);
    }
    pub fn enter_loop(&mut self, head: usize, jmp: bool) {
        self.loop_heads.push((head, jmp));
        self.pending_breaks.push(vec![]);
    }

    /// # When a loop scope exits, this should be called
    pub fn exit_loop(&mut self, end: usize, irs: &mut Vec<IR>) {
        let head = self
            .loop_heads
            .pop()
            .expect("BOTH ENTER AND EXIT LOOP MUST EXIST");
        if head.1 {
            let from = irs.len();
            self.linker.link(from, head.0);
            //irs.push(IR::Jump(Self::calc_offset(head.0, from)));
        }
        if let Some(breaks) = self.pending_breaks.pop() {
            for from in breaks {
                let to = if head.1 { end + 1 } else { end };
                self.linker.link(from, to);
                let _offset = Self::calc_offset(to, from);
                //irs[from] = IR::Jump(offset);
            }
        }
    }
    /// # When a common scope exits, this should be called
    pub fn exit_and_resolve(&mut self, irs: &mut Vec<IR>) -> Result<(), DukaCodegenError> {
        self.resolve_pendings(irs)?;
        self.labels.pop();
        Ok(())
    }
    pub fn resolve_pendings(&mut self, _irs: &mut Vec<IR>) -> Result<(), DukaCodegenError> {
        // JumpInfo is PendingGoto in this case
        for JumpInfo(name, goto_pos) in std::mem::take(&mut self.pending_gotos).into_iter() {
            let label_pos = self.find_label(&name).ok_or_else(|| {
                DukaCodegenError::from(DukaCodegenErrorKind::UnsolvedGoto(name.to_owned()))
            })?;
            self.linker.link(goto_pos, label_pos);
            //irs[goto_pos] = IR::Jump(Self::calc_offset(label_pos, goto_pos));
        }
        Ok(())
    }
    #[inline(always)]
    const fn calc_offset(to: usize, from: usize) -> i32 {
        to as i32 - from as i32
    }

    pub fn label(&mut self, name: String, label_pos: usize) {
        // no duplicated, already checked in analyzer
        self.labels
            .last_mut()
            .expect("NO I COULD HAVE CHECKED THIS")
            .push(JumpInfo(name, label_pos));
    }
    fn find_label(&self, name: &str) -> Option<usize> {
        // JumpInfo is Label
        self.labels.iter().rev().find_map(|scope| {
            scope
                .iter()
                .find_map(|JumpInfo(n, pos)| (n == name).then_some(*pos))
        })
    }
    /// ## Declare a `goto`
    /// When call this, it tries to find target label first,
    /// if have, then `return Some(label_pos)`
    /// or else the `goto` will be inserted into *pending_gotos* then `return None`
    fn declare_goto(&mut self, name: &str, goto_inst_pos: usize) -> Option<usize> {
        // JumpInfo is PendingGoto
        let pos = self.find_label(name);
        if pos.is_none() {
            self.pending_gotos
                .push(JumpInfo(name.to_owned(), goto_inst_pos));
        }
        pos
    }

    /// # Attention: 这个会直接从跳转处开始执行 而不是从它的下一条
    /// ## Create a `jump` opcode
    /// If method `declare_goto` returned `None`, then the **sJ** parameter will be zero, waiting to be resolved
    pub fn goto(&mut self, label: &str, goto_pos: usize) -> IR {
        let target = self.declare_goto(label, goto_pos);

        target
            .map(|label_pos| {
                self.linker.link(goto_pos, label_pos);
                //IR::Jump(Self::calc_offset(label_pos, goto_pos))
                todo!()
            })
            .unwrap_or_default() // placeholder
    }
}

#[derive(Debug)]
pub struct IRGenerator {
    allocator: Allocator,
    labels: Labels,

    instructions: Vec<IR>,
    nesteds: Vec<DukaIR>,
    constants: Constants,
    scopes: Scopes,
    debug_info: DebugInfo,
}

#[derive(Debug)]
enum LValue {
    Local(Reg),
    NewLocal(String),
    UpVal(usize),
    /// (env, key)
    Global(Place, Cst),
    /// (table, key)
    SetByKey(Place, Cst),
    /// (table, index)
    SetByIndex(Place, Place),
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

    fn gen_stmts(&mut self, stmts: Vec<Stmt>) -> Result<(), DukaCodegenError> {
        for Stmt(stmt, span) in stmts {
            let start = self.instructions.len();

            self.gen_stmt(Stmt(stmt, span))?;

            let end = self.instructions.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }
        Ok(())
    }

    fn do_exprs_consecutive_at(
        &mut self,
        exprs: Vec<Expr>,
        start_at: Reg,
    ) -> Result<ExpDesc, DukaCodegenError> {
        let len = exprs.len();
        let mut tail_many = None;
        let mut exps = vec![];

        for (i, expr) in exprs.into_iter().enumerate() {
            let to_reg = ToReg::To(start_at + i);
            let ed = self.do_expr_to(expr, to_reg)?;
            if matches!(ed, ExpDesc::Many(..)) && i == len - 1 {
                tail_many = Some(ed);
            } else {
                let val = self.take_first(ed);
                let reg = self.must_allocated_at(val, to_reg);
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
    fn do_exprs_consecutive(&mut self, exprs: Vec<Expr>) -> Result<ExpDesc, DukaCodegenError> {
        self.do_exprs_consecutive_at(exprs, self.allocator.top())
    }

    fn gen_block_with_locals(
        &mut self,
        Block(stmts, ret): Block,
        is_func: bool,
        locals: Vec<(String, Reg)>,
    ) -> Result<(), DukaCodegenError> {
        self.enter(is_func);

        for local in locals {
            self.scopes.declare_local(&local.0, local.1);
        }

        self.gen_stmts(stmts)?;

        if let Some(ret) = ret
            && let StmtKind::Return(items) = (*ret).0
        {
            let span = (*ret).1;
            let start = self.instructions.len();

            let eds = self.do_exprs_consecutive(items)?;
            let (start_reg, count) = self.take_all(eds);
            self.emit(IR::Return(start_reg, count));

            let end = self.instructions.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }

        self.exit(is_func);

        Ok(())
    }

    #[inline(always)]
    /// always call this for block generation
    fn gen_block_scoped(&mut self, block: Block, is_func: bool) -> Result<(), DukaCodegenError> {
        self.gen_block_with_locals(block, is_func, vec![])
    }

    fn load_nil(&mut self) -> Reg {
        let reg = self.allocator.alloc();
        self.emit(IR::LoadNil(reg));
        reg
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
                self.emit(if b { IR::LoadTrue(reg) } else { IR::LoadFalse(reg) });
            }
            ConstValue::ConstTable(array_map) => {
                let idx = self.constants.add(ConstValue::ConstTable(array_map));
                self.emit(IR::LoadConst(reg, idx));
            }
            ConstValue::String(items) => {
                self.emit(IR::LoadString(reg, items));
            }
        }
    }

    fn take_first(&mut self, exp: ExpDesc) -> Place {
        match exp {
            ExpDesc::Single(pl) => pl,
            ExpDesc::Many(fixeds, vararg) => {
                let mut fixeds = fixeds.into_iter();
                if let Some(reg) = fixeds.next() {
                    self.allocator.free_many(fixeds);
                    Place::R(reg)
                } else if let Some(start) = vararg {
                    self.emit(IR::Take(1));
                    Place::R(start)
                } else {
                    Place::R(self.load_nil())
                }
            }
        }
    }

    /// # Consecutive
    fn take_many(&mut self, exp: ExpDesc, needs: usize) -> Vec<Place> {
        let mut many = Vec::with_capacity(needs);
        match exp {
            ExpDesc::Single(pl) => many.push(pl),
            ExpDesc::Many(fixeds, vararg) => {
                let fixed_count = fixeds.len();
                many.extend(fixeds.into_iter().take(needs).map(Place::R));

                if let Some(start) = vararg {
                    if fixed_count < needs {
                        let rest = needs - fixed_count;
                        many.extend(self.allocator.alloc_consecutive(start, rest).map(Place::R));
                        self.emit(IR::Take(rest));
                        return many;
                    } else {
                        self.allocator.free(start);
                    }
                }
            }
        }

        if many.len() < needs {
            let rest = needs - many.len();
            for _ in 0..rest {
                many.push(Place::R(self.load_nil()));
            }
        }
        many
    }

    /// # Consecutive
    fn take_all(&mut self, exp: ExpDesc) -> (Reg, ValueCount) {
        match exp {
            ExpDesc::Single(pl) => match pl {
                Place::R(r) => {
                    let reg = self.allocator.alloc();
                    self.gen_move(reg, r);
                    (reg, ValueCount::Exact(1))
                }
                pl => {
                    let reg = self.ensure_allocated(pl, ToReg::New);
                    (reg, ValueCount::Exact(1))
                }
            },
            ExpDesc::Many(mut fixeds, vararg) => {
                let fixed_count = fixeds.len();
                if let Some(start) = vararg {
                    assert!({
                        fixeds.push(start);
                        is_consecutive(&fixeds)
                    });
                    let fixed_start = fixeds.iter().min().cloned();
                    self.emit(IR::TakeAll);
                    (fixed_start.unwrap_or(start), ValueCount::VarArg)
                } else {
                    assert!(is_consecutive(&fixeds), "{fixeds:?}");
                    let fixed_start = fixeds.iter().min().cloned();
                    (
                        fixed_start.unwrap_or_default(),
                        ValueCount::Exact(fixed_count),
                    )
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

    fn set_to_path(&mut self, path: Path, global_first: bool) -> Result<LValue, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                // push name into constant pool
                if !global_first {
                    if let Some(pl) = self.scopes.find(&name) {
                        match pl {
                            Place::K(_) | Place::I(_) => {
                                return Err(DukaCodegenError::from(
                                    DukaCodegenErrorKind::TryAssignConst(name),
                                ));
                            }
                            Place::R(r) => LValue::Local(r),
                            Place::U(u) => LValue::UpVal(u),
                        }
                    } else {
                        LValue::NewLocal(name)
                    }
                } else {
                    LValue::Global(self.scopes.ensure_global(), self.constants.add(name.into()))
                }
            }
            Path::Chain(parent, suffix) => {
                let base = self.get_path_to(*parent, global_first, ToReg::New)?;
                match suffix {
                    PathSuffix::Colon(func) => {
                        return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                            format!(
                                "trying to assign value(s) to a function self-calling ({})",
                                func.0
                            ),
                        )));
                    }
                    PathSuffix::Dot((name, _)) => {
                        LValue::SetByKey(base, self.constants.add(name.into()))
                    }
                    PathSuffix::Index(idx) => {
                        let idx = self.do_expr_to(*idx, ToReg::New)?;

                        LValue::SetByIndex(base, self.take_first(idx))
                    }
                }
            }
            Path::Expr(_) => {
                return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
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
    ) -> Result<Place, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                if !global_first && let Some(idx) = self.scopes.find(&name) {
                    idx
                } else {
                    // _ENV
                    let idx = self.constants.add(name.into());
                    let env = self.scopes.ensure_global();
                    let reg = self.get_reg(to_reg);
                    self.emit(IR::GetField(reg, env, Place::K(idx)));
                    Place::R(reg)
                }
            }
            Path::Expr(expr) => {
                let exp = self.do_expr_to(*expr, to_reg)?;
                // for expression, we don't reuse the register
                self.take_first(exp)
            }
            Path::Chain(parent, suffix) => {
                let table = self.get_path_to(*parent, global_first, to_reg)?;
                match suffix {
                    // NOTICE: this is special, it only appears in function calling, but we don't deal it here
                    PathSuffix::Colon((key, _)) |
                    // We can reuse the register, reuse table register is perfect
                    PathSuffix::Dot((key, _)) => {
                        let key = self.constants.add(key.into());
                        let reg = self.get_reg(to_reg);
                        self.emit(IR::GetField(reg, table, Place::K(key)));
                        Place::R(reg)
                    }
                    PathSuffix::Index(idx) => {
                        let exp = self.do_expr_to(*idx, to_reg)?;
                        let reg = self.get_reg(to_reg);
                        let idx = self.take_first(exp);
                        self.emit(IR::GetField(reg, table, idx));
                        Place::R(reg)
                    }
                }
            }
        })
    }

    fn must_allocated_at(&mut self, pl: Place, to_reg: ToReg) -> Reg {
        let to = self.get_reg(to_reg);
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
        to
    }

    /// # Ensure pl has been allocated, which means if pl is already in a register
    fn ensure_allocated(&mut self, pl: Place, if_not: ToReg) -> Reg {
        if let Place::R(r) = pl {
            self.allocator.ensure_allocated(r);
            r
        } else {
            self.must_allocated_at(pl, if_not)
        }
    }

    fn gen_params(&mut self, params: Vec<Expr>) -> Result<ValueCount, DukaCodegenError> {
        let exp = self.do_exprs_consecutive(params)?;
        let (start, count) = self.take_all(exp);
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
    ) -> Result<ExpDesc, DukaCodegenError> {
        let callish = callee.0.is_callish_keyword();
        let self_call = callee.0.is_self_call();

        let expr_len = params.len();

        let callee = if callish.is_some() {
            self.allocator.alloc_temp()
        } else {
            let exp = self.do_expr_to(callee, to_reg)?;
            let pl = self.take_first(exp);
            self.ensure_allocated(pl, to_reg)
        };

        if let Some(ccallish::SPAWN) = callish {
            (expr_len != 1).then_error(|| {
                DukaCodegenError::from(DukaCodegenErrorKind::InvalidParams(
                    ccallish::SPAWN.to_owned(),
                    1,
                    expr_len,
                ))
            })?;

            let func = self.do_exprs_consecutive(params)?;
            let one = self.take_first_allocated(func);

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
                DukaCodegenError::from(DukaCodegenErrorKind::InvalidParams(
                    ccallish::SPAWN.to_owned(),
                    1,
                    expr_len,
                ))
            })?;
            self.allocator.ensure_allocated(callee);
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
    ) -> Result<ExpDesc, DukaCodegenError> {
        self.gen_call_to(callee, params, tailcall, ToReg::New)
    }

    /// DO NOT INPUT EMPTY EXPR
    /// # Always allocate new register
    #[inline(always)]
    fn do_expr(&mut self, expr: Expr) -> Result<ExpDesc, DukaCodegenError> {
        self.do_expr_to(expr, ToReg::New)
    }
    fn get_reg(&mut self, reg: ToReg) -> Reg {
        match reg {
            ToReg::Temp => self.allocator.alloc_temp(),
            ToReg::New => self.allocator.alloc(),
            ToReg::To(reg) => {
                self.allocator.ensure_allocated(reg);
                reg
            }
        }
    }

    /// - reg: target register (if has, or allocate new one)
    /// - keep_im: whether `ConstValue` should be allocated or not
    fn do_expr_to(
        &mut self,
        Expr(expr, span): Expr,
        reg: ToReg,
    ) -> Result<ExpDesc, DukaCodegenError> {
        use ExprKind::*;

        expr.is_sugar().then_error(|| {
            DukaCodegenError::from(DukaCodegenErrorKind::UnsupportedFeature(expr.to_string()))
        })?;

        Ok(ExpDesc::Single(match expr {
            Empty => {
                let reg = self.get_reg(reg);
                self.emit(IR::LoadNil(reg));
                Place::R(reg)
            }
            VarArg => {
                let reg = self.get_reg(reg);
                self.emit(IR::VarArg(reg));
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Literal(cv) => {
                if let ConstValue::Int(val) = cv {
                    Place::I(val)
                } else {
                    let reg = self.get_reg(reg);
                    self.load_const_to(cv, reg);
                    Place::R(reg)
                }
            }
            Do(block) => return self.gen_expr_block(block),
            Access(path) => self.get_path_to(path, false, reg)?,
            Call(callee, params) => {
                // the tailcall place is already processed
                return self.gen_call_to(*callee, params, false, reg);
            }
            SysCall(sys_call) => {
                self.emit(IR::SysCall(sys_call));
                let reg = self.get_reg(reg);
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Table(fields) => {
                let table = self.get_reg(reg);
                self.emit(IR::NewTable(table));
                let clean_from = self.allocator.top();

                let mut fields = fields.into_iter();
                while let Some(field) = fields.next() {
                    match field {
                        Field::KeyValue(k, v) => {
                            let k = self.do_expr(k)?;
                            let v = self.do_expr(v)?;

                            let kpl = self.take_first(k);
                            let vpl = self.take_first(v);

                            self.emit(IR::SetField(Place::R(table), kpl, vpl));
                        }
                        Field::NameValue((n, _), v) => {
                            let k = self.constants.add(n.into());
                            let v = self.do_expr_to(v, ToReg::New)?;

                            let pl = self.take_first(v);
                            self.emit(IR::SetField(Place::R(table), Place::K(k), pl));
                        }
                        Field::Value(v) => {
                            let mut batch = vec![v];
                            while let Some(Field::Value(v)) = fields.next() {
                                batch.push(v);
                            }
                            let exp = self.do_exprs_consecutive(batch)?;
                            let (start, count) = self.take_all(exp);
                            assert!(start == table + 1);
                            self.emit(IR::Array(Place::R(table), count));
                        }
                    }
                }
                self.allocator.free_many(clean_from..);
                Place::R(table)
            }
            Function(func_body) => {
                let mut ir = self.gen_func_block(func_body, false)?;
                ir.debug_info.all_span = span;
                self.nesteds.push(ir);
                let reg = self.get_reg(reg);
                self.emit(IR::Closure(reg, self.nesteds.len() - 1));
                Place::R(reg)
            }
            Unary(expr, un_op) => {
                let ed = self.do_expr(*expr)?;
                let operand = self.take_first(ed);

                let reg = self.get_reg(reg);
                self.emit(IR::Unary(reg, operand, un_op));

                Place::R(reg)
            }
            Binary(le, re, bin_op) => {
                let reg = self.get_reg(reg);
                if bin_op.is_short() {
                    let le = self.do_expr_to(*le, ToReg::To(reg))?;
                    let lp = self.take_first(le);
                    self.must_allocated_at(lp, ToReg::To(reg));

                    let lab = self.labels.new_label(None);
                    self.emit(IR::SkipNext(reg, matches!(bin_op, BinOp::Or)));
                    self.emit(IR::Jump(lab));

                    let re = self.do_expr_to(*re, ToReg::To(reg))?;
                    let rp = self.take_first(re);
                    self.must_allocated_at(rp, ToReg::To(reg));
                    self.emit(IR::Label(lab));
                } else {
                    let left = self.do_expr_to(*le, ToReg::To(reg))?;
                    let right = self.do_expr_to(
                        *re,
                        matches!(left, ExpDesc::Single(Place::I(..)))
                            .then_some(ToReg::To(reg))
                            .unwrap_or(ToReg::Temp),
                    )?;

                    let left = self.take_first(left);
                    let right = self.take_first(right);

                    self.emit(IR::Binary(reg, left, right, bin_op));
                }
                Place::R(reg)
            }
            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);
                let end = self.labels.new_label(None); // 当分支条件为真时 负责运行完分支后跳到最后面

                self.gen_skip_next(*(if_.1), true)?;
                let mut lab = self.labels.new_label(None);
                self.emit(IR::Jump(lab));

                let ed = self.gen_expr_block(if_.0)?;
                let pl = self.take_first(ed);
                self.must_allocated_at(pl, reg);

                self.emit(IR::Jump(end));

                for ifelse in ifelses {
                    self.emit(IR::Label(lab));

                    self.gen_skip_next(*(ifelse.1), true)?;
                    lab = self.labels.new_label(None);
                    self.emit(IR::Jump(lab));

                    let ed = self.gen_expr_block(ifelse.0)?;
                    let pl = self.take_first(ed);
                    self.must_allocated_at(pl, reg);

                    self.emit(IR::Jump(end));
                }

                self.emit(IR::Label(lab));
                let blk = else_.ok_or(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                    "No else block found".to_string(),
                )))?;
                let ed = self.gen_expr_block(blk)?;
                let pl = self.take_first(ed);
                self.must_allocated_at(pl, reg);

                self.emit(IR::Label(end));
                Place::R(self.get_reg(reg))
            }
            _ => unreachable!(),
        }))
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

    fn gen_assign(&mut self, left: LValue, val: Place) -> Result<(), DukaCodegenError> {
        match left {
            LValue::Global(env, key) => self.emit(IR::SetField(env, Place::K(key), val)),
            LValue::SetByKey(tab, key) => self.emit(IR::SetField(tab, Place::K(key), val)),
            LValue::Local(to) => {
                self.must_allocated_at(val, ToReg::To(to));
            }
            LValue::NewLocal(name) => {
                let reg = self.ensure_allocated(val, ToReg::New);
                self.scopes.declare_local(&name, reg);
            }
            LValue::UpVal(u) => self.emit(IR::SetUpVal(u, val)),
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
    fn exit(&mut self, is_func: bool) -> Result<(), DukaCodegenError> {
        if is_func {
            self.allocator.exit();
        }

        let scope = self.scopes.exit();

        if !is_func && let Scope::Block { locals, .. } = scope {
            for (_, reg) in locals {
                self.allocator.free(reg);
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
    ) -> Result<DukaIR, DukaCodegenError> {
        let has_var_arg = body.has_vararg();
        let FuncBody(params, Block(stmts, ret)) = body;
        let param_count = params.len();

        let mut irg = Self::new();
        irg.scopes = self.scopes.clone();
        irg.constants = Constants::default();
        irg.enter(true);

        if self_call {
            irg.scopes.declare_local(cgen::SELF, 1);
        }
        for param in params {
            match param {
                Param::Name((name, _)) => {
                    irg.scopes.declare_local(&name, irg.allocator.alloc()) //NOTICE, there already exist values
                }
                _ => break,
            }
        }

        irg.gen_stmts(stmts)?;
        if let Some(ret) = ret
            && let StmtKind::Return(mut items) = (*ret).0
        {
            let span = (*ret).1;
            let start = irg.instructions.len();

            let (start_reg, count) =
                if let [Expr(ExprKind::Call(callee, params), _), ..] = items.as_mut_slice() {
                    let callee = std::mem::take(callee);
                    let params = std::mem::take(params);

                    let ed = irg.gen_call(*callee, params, true)?;
                    irg.take_all(ed)
                } else {
                    let eds = irg.do_exprs_consecutive(items)?;
                    irg.take_all(eds)
                };
            irg.emit(IR::Return(start_reg, count));

            let end = irg.instructions.len();
            irg.debug_info.inst_spans.insert(start..end, span);
        }

        irg.exit(true)?;

        Ok(DukaIR {
            has_var_arg,
            param_count,
            nesteds: irg.nesteds,
            instructions: irg.instructions,
            constants: irg.constants,
            scopes: irg.scopes,
            debug_info: irg.debug_info,
            logic: None,
            label_names: irg.labels.into_names(),
        })
    }

    fn gen_expr_block(&mut self, Block(stmts, ret): Block) -> Result<ExpDesc, DukaCodegenError> {
        self.enter(false);

        for stmt in stmts {
            self.gen_stmt(stmt)?;
        }

        let Some(ret) = ret else {
            return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                "No return in expr block".to_owned(),
            )));
        };
        let StmtKind::Return(items) = (*ret).0 else {
            return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                "No return expr at the end of expr block".to_owned(),
            )));
        };

        let exp = self.do_exprs_consecutive(items)?;

        self.exit(false)?;

        Ok(exp)
    }

    fn ensure_const(&mut self, expr: ExprKind) -> Result<ConstValue, DukaCodegenError> {
        if let ExprKind::Literal(cv) = expr {
            Ok(cv)
        } else {
            Err(DukaCodegenError::from(DukaCodegenErrorKind::NotConstExpr))
        }
    }
    #[inline]
    fn take_first_allocated(&mut self, exp: ExpDesc) -> Reg {
        let pl = self.take_first(exp);
        self.ensure_allocated(pl, ToReg::New)
    }

    fn gen_skip_next(&mut self, cond: Expr, when: bool) -> Result<(), DukaCodegenError> {
        let exp = self.do_expr(cond)?;
        let reg = self.take_first_allocated(exp);
        self.emit(IR::SkipNext(reg, when));
        self.allocator.free(reg);
        Ok(())
    }

    fn gen_stmt(&mut self, Stmt(stmt, span): Stmt) -> Result<(), DukaCodegenError> {
        use StmtKind::*;

        if stmt.is_empty() {
            return Ok(());
        }
        stmt.is_sugar()
            .then_error(|| DukaCodegenErrorKind::UnsupportedFeature(stmt.to_string()))?;
        matches!(stmt, StmtKind::Return(..)).then_error(|| {
            DukaCodegenErrorKind::InvalidAST(
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

                self.gen_block_scoped(if_.0, false)?;
                self.emit(IR::Jump(to_end));

                for ifelse in ifelses {
                    self.emit(IR::Label(lab));

                    self.gen_skip_next(*ifelse.1, true)?;
                    lab = self.labels.new_label(None);
                    self.emit(IR::Jump(lab));

                    self.gen_block_scoped(ifelse.0, false)?;
                    self.emit(IR::Jump(to_end));
                }

                self.emit(IR::Label(lab));
                if let Some(blk) = else_ {
                    self.gen_block_scoped(blk, false)?;
                }

                self.emit(IR::Label(to_end));
            }

            While(cond, blk) => {
                let start = self.labels.new_label(None);
                let end = self.labels.new_label(None);
                self.labels.new_loop(start, end);
                self.emit(IR::Label(start));

                self.gen_skip_next(cond, true)?;
                self.emit(IR::Jump(end));

                self.gen_block_scoped(blk, false)?;

                self.emit(IR::Jump(start));
                self.emit(IR::Label(end));
                self.labels.exit_loop();
            }
            ForGeneric(vars, from, blk) => {
                let start = self.labels.new_label(None);
                let to_call = self.labels.new_label(None);
                let end = self.labels.new_label(None);
                self.labels.new_loop(start, end);

                let ed = self.do_exprs_consecutive(from)?;
                let generator = self
                    .take_many(ed, 4)
                    .into_iter()
                    .map(|pl| self.ensure_allocated(pl, ToReg::New))
                    .collect::<Vec<Reg>>()
                    .first()
                    .cloned()
                    .expect("WTF");
                let locals = vars
                    .into_iter()
                    .map(|var| match var {
                        Path::Base((name, _)) => Ok((name, self.allocator.alloc())),
                        _ => Err(DukaCodegenErrorKind::InvalidAST(format!(
                            "Invalid variable name in generic for-loop: {var}"
                        ))),
                    })
                    .collect::<Result<Vec<(String, Reg)>, _>>()?;
                let regs: Vec<_> = locals.iter().map(|i| i.1).collect();

                self.emit(IR::TForPrep(generator, to_call)); //TForPrep
                self.emit(IR::Label(start));

                self.gen_block_with_locals(blk, false, locals)?;

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

                let from = self.do_expr(from)?;
                let end = self.do_expr(end)?;
                let step = step
                    .map(|e| self.do_expr(e))
                    .transpose()?
                    .unwrap_or(ExpDesc::Single(Place::I(1)));

                let from = self.take_first_allocated(from);
                let end = self.take_first_allocated(end);
                let step = self.take_first_allocated(step);

                self.emit(IR::ForPrep(from, to_end)); //ForPrep
                self.emit(IR::Label(to_start));

                self.gen_block_with_locals(
                    blk,
                    false,
                    vec![match var {
                        Path::Base((name, _)) => (name, from),
                        _ => {
                            return Err(DukaCodegenErrorKind::InvalidAST(format!(
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
                self.gen_block_scoped(blk, false)?;
            }
            Function(name, _attrs, body, global) => {
                let mut ir = self.gen_func_block(body, name.is_self_call())?;
                ir.debug_info.debug_name = Some(name.to_string());
                ir.debug_info.all_span = span;

                self.nesteds.push(ir);
                let reg = self.allocator.alloc();
                self.emit(IR::Closure(reg, self.nesteds.len() - 1));
                let assign_to = self.set_to_path(name, global)?;

                self.gen_assign(assign_to, Place::R(reg))?;
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
                    self.scopes.declare_const(&name, self.constants.add(cv));
                }

                let (attrnames, exprs): (Vec<_>, Vec<_>) = normals.into_iter().unzip();

                let desc =
                    self.do_exprs_consecutive(exprs.into_iter().map_while(|i| i).collect())?;
                let mut pls = self.take_many(desc, attrnames.len()).into_iter();

                for (name, _) in attrnames {
                    let pl = pls
                        .next()
                        .unwrap_or_else(|| /*unreachable*/Place::R(self.load_nil()));

                    if global {
                        let left = self.set_to_path(Path::Base((name, Span::EMPTY)), true)?;
                        self.gen_assign(left, pl)?;
                    } else {
                        let wh = self.ensure_allocated(pl, ToReg::New);
                        self.scopes.declare_local(&name, wh);
                    }
                }
            }
            Assign(names, mut exprs) => {
                let needs = names.len();
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
                    self.do_exprs_consecutive_at(exprs, start)?
                } else {
                    self.do_exprs_consecutive(exprs)?
                };

                let mut vals = self.take_many(ed, needs).into_iter();
                for left in lefts {
                    let val = vals.next().expect("WTF"); // this is unreachable, all work done in take_many
                    self.gen_assign(left, val)?;
                }
            }
            Break => {
                let (_, end) = self.labels.get_loop().ok_or(DukaCodegenError::from(
                    DukaCodegenErrorKind::OutOfLoop("break".to_owned()),
                ))?;
                self.emit(IR::Jump(end))
            }
            Continue => {
                let (start, _) = self.labels.get_loop().ok_or(DukaCodegenError::from(
                    DukaCodegenErrorKind::OutOfLoop("continue".to_owned()),
                ))?;
                self.emit(IR::Jump(start))
            }
            Expr(e) => {
                let from = self.allocator.top();
                let ed = self.do_expr(e)?;
                self.take_none(ed);
                self.allocator.free_many(from..);
            }
            Call(callee, params) => {
                let from = self.allocator.top();
                let ed = self.gen_call(callee, params, false)?;
                self.take_none(ed);
                self.allocator.free_many(from..);
            }
            _ => {
                debug_assert!(true, "Non-exhausted matching for StmtKind");
                unreachable!()
            }
        }

        Ok(())
    }

    fn gen_main(&mut self, blk: Block) -> Result<(), DukaCodegenError> {
        self.gen_block_scoped(blk, true)?;
        self.emit(IR::Return(0, ValueCount::Exact(0))); // ensure
        Ok(())
    }
}

impl DukaGenerator<DukaIR> for IRGenerator {
    type InputType = DukaChunk;

    fn generate(input: Self::InputType) -> Result<DukaIR, DukaCodegenError> {
        let mut generator = Self::new();
        generator.gen_main(input.chunk)?;
        generator.debug_info.debug_name = Some(cgen::MAIN.to_owned());
        generator.debug_info.all_span = input.span;

        Ok(DukaIR {
            param_count: 0,
            has_var_arg: true,
            instructions: generator.instructions,
            nesteds: generator.nesteds,
            constants: generator.constants,
            scopes: generator.scopes,
            debug_info: generator.debug_info,
            logic: Some(input.logic),
            label_names: generator.labels.into_names(),
        })
    }
}

#[derive(Debug)]
pub struct Generator {
    constants: Constants,
    scopes: Scopes,
    debug_info: DebugInfo,
    instructions: Vec<I>,

    allocator: Allocator,
    jumping: Jumper,

    nested_protos: Vec<DukaProto>,
}

#[doc = "有可能合并优化的指令"]
impl Generator {
    fn load_nil(&mut self, from: Address, count: Bits17) -> Result<(), DukaCodegenError> {
        if let Some(v) = self.instructions.last_mut()
            && let DecodeInstruction::LoadNil(pfrom, pcount) = v.decode()?
        {
            let (from, pfrom) = (from as u32, pfrom as u32);
            if (pfrom <= from && from <= pfrom + pcount) || (from <= pfrom && pfrom <= from + count)
            {
                // 起点取最小 终点取最大
                let end = (from + count).max(pfrom + pcount) as Bits17;
                let from = from.min(pfrom) as Address;

                *v = I::LoadNil(from, end - (from as Bits17));
                return Ok(());
            }
        }

        //self.emit(I::LoadNil(from, count));
        Ok(())
    }
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            jumping: Jumper::new(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            nested_protos: vec![],
        }
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = DukaIR;

    fn generate(_ir: Self::InputType) -> Result<DukaProto, DukaCodegenError> {
        // let DukaChunk {
        //     chunk,
        //     span: _,
        //     logic,
        // } = chunk;
        // let logic = LogicGenerator::generate(logic)?;
        // let mut proto = Self::new().generate_proto(chunk, Some("main".to_owned()), None, true)?;
        // proto.logic = Some(logic);
        // Ok(proto)
        Ok(todo!())
    }
}
