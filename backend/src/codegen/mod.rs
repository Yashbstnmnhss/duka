use std::usize;

use crate::DebugInfo;
use crate::codegen::types::{Allocator, Constants, DukaIR, ExpDesc, IR, Place, Reg, Scopes};
use crate::{
    instructions::{Address, Bits17, DecodeInstruction, Instruction as I},
    value::DukaProto,
};
use duka_shared::constants::{catt, cgen};
use duka_shared::error::DukaCodegenError;
use duka_shared::types::{DukaChunk, DukaGenerator};
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use duka_shared::{
    ast::{Block, Expr, ExprKind, Field, FuncBody, Param, Path, PathSuffix, Stmt, StmtKind},
    error::DukaCodegenErrorKind::{self},
};

pub mod binary;
pub mod logic;
mod types;

/// ### This `struct` represents two items:
/// - **Label**(name, target_pos)
/// - **PendingGoto**(target_name, goto_inst_pos)
#[derive(Debug)]
struct JumpInfo(String, usize);
#[derive(Debug, Default)]
struct IRJumper {
    labels: Vec<Vec<JumpInfo>>, // labels of scopes

    loop_heads: Vec<usize>, // the start of every loop (contains itself)
    pending_breaks: Vec<Vec<usize>>, // position of pending breaks in loop scopes
    pending_gotos: Vec<JumpInfo>, // all pending gotos (jump backwards)

    pending_onetime: Vec<usize>,     //一次性
    pending_branch: Vec<Vec<usize>>, //多对一
}
impl IRJumper {
    const PLACEHOLDER: i32 = 0;

    pub fn new() -> Self {
        Self {
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
        irs.push(IR::Jump(Self::PLACEHOLDER))
    }
    pub fn branch_end(&mut self, irs: &mut Vec<IR>) {
        if let Some(is) = self.pending_branch.pop() {
            for idx in is {
                irs[idx] = IR::Jump(Self::calc_offset(irs.len(), idx));
            }
        }
    }

    /// NOTICE: THIS IS ONLY BACKWARD
    /// 单独 一起点一终点
    pub fn onetime_jmp(&mut self, irs: &mut Vec<IR>) {
        self.pending_onetime.push(irs.len());
        irs.push(IR::Jump(Self::PLACEHOLDER));
    }

    pub fn onetime_end(&mut self, irs: &mut Vec<IR>) {
        let pos = self.pending_onetime.pop().unwrap();
        irs[pos] = IR::Jump(Self::calc_offset(irs.len(), pos));
    }

    pub fn loop_continue(&self, current: usize) -> IR {
        let pos = *self
            .loop_heads
            .last()
            .expect("CONTINUE MUST BE USED IN A LOOP");
        let offset = Self::calc_offset(pos, current);
        IR::Jump(offset)
    }
    pub fn loop_break(&mut self, current: usize) -> IR {
        self.pending_breaks
            .last_mut()
            .expect("BREAK MUST BE USED IN A LOOP")
            .push(current);
        Self::placeholder()
    }

    pub fn enter(&mut self) {
        self.labels.push(vec![]);
    }
    pub fn enter_loop(&mut self, head: usize) {
        self.loop_heads.push(head);
        self.pending_breaks.push(vec![]);
    }

    /// # When a loop scope exits, this should be called
    pub fn exit_loop(&mut self, end: usize, irs: &mut Vec<IR>) {
        if let Some(breaks) = self.pending_breaks.pop() {
            for from in breaks {
                let offset = Self::calc_offset(end, from);
                irs[from] = IR::Jump(offset);
            }
        }
        self.loop_heads.pop();
    }
    /// # When a common scope exits, this should be called
    pub fn exit_and_resolve(&mut self, irs: &mut Vec<IR>) -> Result<(), DukaCodegenError> {
        assert!(self.pending_onetime.len() == 0);

        self.resolve_pendings(irs)?;
        self.labels.pop();
        Ok(())
    }
    pub fn resolve_pendings(&mut self, irs: &mut Vec<IR>) -> Result<(), DukaCodegenError> {
        // JumpInfo is PendingGoto in this case
        for JumpInfo(name, goto_pos) in std::mem::take(&mut self.pending_gotos).into_iter() {
            let label_pos = self.find_label(&name).ok_or_else(|| {
                DukaCodegenError::from(DukaCodegenErrorKind::UnsolvedGoto(name.to_owned()))
            })?;
            irs[goto_pos] = IR::Jump(Self::calc_offset(label_pos, goto_pos));
        }
        Ok(())
    }
    #[inline(always)]
    const fn calc_offset(to: usize, from: usize) -> i32 {
        to as i32 - from as i32
    }
    #[inline(always)]
    fn placeholder() -> IR {
        IR::Jump(Self::PLACEHOLDER)
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
        IR::Jump(
            target
                .map(|label_pos| Self::calc_offset(label_pos, goto_pos))
                .unwrap_or(Self::PLACEHOLDER), // placeholder
        )
    }
}

#[derive(Debug)]
pub struct IRGenerator {
    allocator: Allocator,
    jumper: IRJumper,

    instructions: Vec<IR>,
    nesteds: Vec<DukaIR>,
    constants: Constants,
    scopes: Scopes,
    debug_info: DebugInfo,
}

#[derive(Debug)]
enum LValue {
    Local(Reg),
    UpVal(usize),
    /// (env, key)
    Global(Place, usize),
    /// (table, key)
    SetByKey(Place, usize),
    /// (table, index)
    SetByIndex(Place, Place),
}

impl IRGenerator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            jumper: IRJumper::new(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            nesteds: vec![],
        }
    }

    #[inline]
    fn emit(&mut self, ir: IR) {
        self.instructions.push(ir);
    }

    fn gen_stmts(&mut self, stmts: Vec<Stmt>) -> Result<(), DukaCodegenError> {
        for Stmt(stmt, span) in stmts {
            let start = self.instructions.len();

            self.gen_stmt(stmt)?;

            let end = self.instructions.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }
        Ok(())
    }
    fn do_exprs(&mut self, exprs: Vec<Expr>) -> Result<ExpDesc, DukaCodegenError> {
        let len = exprs.len();
        let mut tail_many = None;
        let mut exps = vec![];

        for (i, Expr(expr, _)) in exprs.into_iter().enumerate() {
            let exp = self.do_expr(expr)?;
            if matches!(exp, ExpDesc::Many(..)) {
                if i != len - 1 {
                    let pl = self.take_first(exp);
                    exps.push(self.ensure_allocated(pl));
                } else {
                    tail_many = Some(exp);
                }
            }
        }

        Ok(if let Some(ExpDesc::Many(fixed, start)) = tail_many {
            exps.extend(fixed);
            ExpDesc::Many(exps, start)
        } else {
            ExpDesc::from_regs(exps)
        })
    }

    /// always call this for block generation
    fn gen_block_scoped(
        &mut self,
        Block(stmts, ret): Block,
        is_func: bool,
    ) -> Result<(), DukaCodegenError> {
        self.enter(is_func);

        self.gen_stmts(stmts)?;

        if let Some(ret) = ret
            && let StmtKind::Return(items) = (*ret).0
        {
            let span = (*ret).1;
            let start = self.instructions.len();

            self.emit(IR::Return(items.len()));
            self.do_exprs(items)?;

            let end = self.instructions.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }

        self.exit(is_func)?;

        Ok(())
    }

    fn load_nil(&mut self) -> Reg {
        let reg = self.allocator.alloc();
        self.emit(IR::LoadNil(reg));
        reg
    }
    fn load_const(&mut self, cv: ConstValue) -> Reg {
        match cv {
            ConstValue::Nil => self.load_nil(),
            ConstValue::Int(i) => {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadInt(reg, i));
                reg
            }
            ConstValue::Float(f) => {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadFloat(reg, f));
                reg
            }
            ConstValue::Bool(b) => {
                let reg = self.allocator.alloc();
                self.instructions
                    .push(b.then_some(IR::LoadTrue(reg)).unwrap_or(IR::LoadFalse(reg)));
                reg
            }
            ConstValue::ConstTable(array_map) => {
                let reg = self.allocator.alloc();
                self.instructions.push(IR::LoadConst(
                    reg,
                    self.constants.add(ConstValue::ConstTable(array_map)),
                ));
                reg
            }
            ConstValue::String(items) => {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadString(reg, items));
                reg
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
            ExpDesc::Immediate(cv) => Place::R(self.load_const(cv)),
        }
    }
    fn take_many(&mut self, exp: ExpDesc, needs: usize) -> Vec<Place> {
        let mut many = Vec::with_capacity(needs);
        match exp {
            ExpDesc::Single(pl) => many.push(pl),
            ExpDesc::Immediate(i) => many.push(Place::R(self.load_const(i))),
            ExpDesc::Many(fixeds, vararg) => {
                let fixed_count = fixeds.len();
                many.extend(fixeds.into_iter().map(Place::R));

                if fixed_count < needs
                    && let Some(start) = vararg
                {
                    self.allocator.alloc_to(start + needs);
                    self.emit(IR::Take(needs));
                    return many;
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

    fn set_to_path(&mut self, path: Path, global_first: bool) -> Result<LValue, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                // push name into constant pool
                if !global_first && let Some(pl) = self.scopes.find(&name) {
                    match pl {
                        Place::K(_) => {
                            return Err(DukaCodegenError::from(
                                DukaCodegenErrorKind::TryAssignConst(name),
                            ));
                        }
                        Place::R(r) => LValue::Local(r),
                        Place::U(u) => LValue::UpVal(u),
                    }
                } else {
                    LValue::Global(self.scopes.ensure_global(), self.constants.add(name.into()))
                }
            }
            Path::Chain(parent, suffix) => {
                let base = self.get_from_path(*parent, global_first)?;
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
                        let idx = self.do_expr((*idx).0)?;
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

    /// Notice, this won't process colon path `obj:func`, which should be in function calling
    fn get_from_path(&mut self, path: Path, global_first: bool) -> Result<Place, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                if !global_first && let Some(idx) = self.scopes.find(&name) {
                    idx
                } else {
                    // _ENV
                    let idx = self.constants.add(name.into());
                    let env = self.scopes.ensure_global();
                    let reg = self.allocator.alloc();
                    self.instructions
                        .push(IR::GetField(reg, env, Place::K(idx)));
                    Place::R(reg)
                }
            }
            Path::Expr(expr) => {
                let exp = self.do_expr((*expr).0)?;
                self.take_first(exp)
            }
            Path::Chain(parent, suffix) => {
                let table = self.get_from_path(*parent, global_first)?;
                match suffix {
                    // NOTICE: this is special, it only appears in function calling, but we don't deal it here
                    PathSuffix::Colon((key, _)) |
                    // We can reuse the register, reuse table register is perfect
                    PathSuffix::Dot((key, _)) => {
                        let key = self.constants.add(key.into());
                        let reg = if let Place::R(reg) = table {
                            reg
                        } else {
                            self.allocator.alloc()
                        };
                        self.emit(IR::GetField(reg, table, Place::K(key)));
                        Place::R(reg)
                    }
                    PathSuffix::Index(idx) => {
                        let exp = self.do_expr((*idx).0)?;
                        let reg = if let Place::R(r) = table {
                            r
                        } else {
                            self.allocator.alloc()
                        };
                        let idx = self.take_first(exp);
                        self.emit(IR::GetField(reg, table, idx));
                        Place::R(reg)
                    }
                }
            }
        })
    }

    fn ensure_allocated(&mut self, pl: Place) -> Reg {
        match pl {
            Place::K(k) => {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadConst(reg, k));
                reg
            }
            Place::U(u) => {
                let reg = self.allocator.alloc();
                self.emit(IR::GetUpVal(reg, u));
                reg
            }
            Place::R(r) => r,
        }
    }

    fn gen_param(&mut self, params: Vec<Expr>) -> Result<usize, DukaCodegenError> {
        let len = params.len();
        for param in params {
            let exp = self.do_expr(param.0)?;
            let pl = self.take_first(exp);
            let reg = self.ensure_allocated(pl);
            self.emit(IR::Param(reg));
        }
        Ok(len)
    }

    // return the start reg
    fn gen_call(
        &mut self,
        Expr(callee, _): Expr,
        params: Vec<Expr>,
        tailcall: bool,
    ) -> Result<Reg, DukaCodegenError> {
        let self_ = matches!(
            callee,
            ExprKind::Access(ref p) if p.is_self_call()
        );
        let exp = self.do_expr(callee)?;
        let pl = self.take_first(exp);
        let callee = self.ensure_allocated(pl);

        if self_ {
            self.emit(IR::Self_());
        }
        self.gen_param(params)?;

        let start_reg = self.allocator.alloc();
        self.emit(if tailcall {
            IR::TailCall(start_reg, callee)
        } else {
            IR::Call(start_reg, callee)
        });

        Ok(start_reg)
    }

    // DO NOT INPUT EMPTY EXPR
    ///
    fn do_expr(&mut self, expr: ExprKind) -> Result<ExpDesc, DukaCodegenError> {
        use ExprKind::*;

        expr.is_sugar().then_error(|| {
            DukaCodegenError::from(DukaCodegenErrorKind::UnsupportedFeature(expr.to_string()))
        })?;
        // matches!(expr, ExprKind::Empty).then_error(|| {
        //     DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
        //         "got empty expr".to_owned(),
        //     ))
        // })?;

        Ok(ExpDesc::Single(match expr {
            Empty => {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadNil(reg));
                Place::R(reg)
            }
            VarArg => {
                let reg = self.allocator.alloc();
                self.emit(IR::VarArg(reg));
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            Literal(cv) => return Ok(ExpDesc::Immediate(cv)),
            Do(block) => return Ok(self.gen_expr_block(block)?),
            Access(path) => self.get_from_path(path, false)?,
            Call(callee, params) => {
                // the tailcall place is already processed
                let reg = self.gen_call(*callee, params, false)?;
                return Ok(ExpDesc::Many(vec![], Some(reg)));
            }
            SysCall(sys_call) => {
                self.emit(IR::SysCall(sys_call));
                unimplemented!()
            }
            Table(fields) => {
                let table = self.allocator.alloc();
                self.emit(IR::NewTable(table));

                let mut fields = fields.into_iter().peekable();
                while let Some(field) = fields.next() {
                    match field {
                        Field::KeyValue(k, v) => {
                            let k = self.do_expr(k.0)?;
                            let v = self.do_expr(v.0)?;

                            let kpl = self.take_first(k);
                            let vpl = self.take_first(v);

                            self.instructions
                                .push(IR::SetField(Place::R(table), kpl, vpl));
                        }
                        Field::NameValue((n, _), v) => {
                            let k = self.constants.add(n.into());
                            let v = self.do_expr(v.0)?;
                            let pl = self.take_first(v);
                            self.instructions
                                .push(IR::SetField(Place::R(table), Place::K(k), pl));
                        }
                        Field::Value(v) => {
                            let exp = self.do_expr(v.0)?;
                            let pl = self.take_first(exp);
                            let mut batch = vec![self.ensure_allocated(pl)];
                            while let Some(Field::Value(_)) = fields.peek() {
                                let Some(Field::Value(v)) = fields.next() else {
                                    unreachable!()
                                };
                                let exp = self.do_expr(v.0)?;
                                let pl = self.take_first(exp);
                                batch.push(self.ensure_allocated(pl));
                            }
                            self.emit(IR::Array(Place::R(table), batch));
                        }
                        Field::Expand => {
                            todo!()
                        }
                    }
                }
                Place::R(table)
            }
            Function(func_body) => {
                let ir = self.gen_func_block(func_body, false)?;
                self.nesteds.push(ir);
                let reg = self.allocator.alloc();
                self.instructions
                    .push(IR::Closure(reg, self.nesteds.len() - 1));
                Place::R(reg)
            }
            Unary(expr, un_op) => {
                let exp = self.do_expr((*expr).0)?;
                let operand = self.take_first(exp);
                let reg = self.allocator.alloc();
                self.emit(IR::Unary(reg, operand, un_op));
                Place::R(reg)
            }
            Binary(expr, expr1, bin_op) => {
                let left = self.do_expr((*expr).0)?;
                let right = self.do_expr((*expr1).0)?;

                let left = self.take_first(left);

                Place::R(if let ExpDesc::Immediate(ConstValue::Int(int)) = right {
                    let reg = self.allocator.alloc();
                    self.emit(IR::BinaryI(reg, left, int, bin_op));
                    reg
                } else {
                    let right = self.take_first(right);
                    let reg = self.allocator.alloc();
                    self.emit(IR::Binary(reg, left, right, bin_op));
                    reg
                })
            }
            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);

                self.jumper.branch_start(); // 当分支条件为真时 负责运行完分支后跳到最后面

                self.jumper.onetime_jmp(&mut self.instructions); // 当分支条件为假时 负责跳到下一个分支处

                self.gen_block_scoped(if_.0, false)?;
                self.jumper.branch_jmp(&mut self.instructions);

                for ifelse in ifelses {
                    self.jumper.onetime_end(&mut self.instructions);

                    self.jumper.onetime_jmp(&mut self.instructions);

                    self.gen_block_scoped(ifelse.0, false)?;
                    self.jumper.branch_jmp(&mut self.instructions);
                }

                self.jumper.onetime_end(&mut self.instructions);
                if let Some(blk) = else_ {
                    self.gen_block_scoped(blk, false)?;
                }

                self.jumper.branch_end(&mut self.instructions);
                todo!()
            }

            _ => unreachable!(),
        }))
    }

    fn gen_assign(&mut self, lefts: Vec<LValue>, vals: Vec<Place>) -> Result<(), DukaCodegenError> {
        let mut vals = vals.into_iter();
        for left in lefts {
            let val = vals.next().unwrap_or_else(|| {
                let reg = self.allocator.alloc();
                self.emit(IR::LoadNil(reg));
                Place::R(reg)
            });

            match left {
                LValue::Local(reg) => {
                    let wh = self.ensure_allocated(val);
                    self.emit(IR::Move(reg, wh));
                }
                LValue::SetByKey(tab, k) | LValue::Global(tab, k) => {
                    self.emit(IR::SetField(tab, Place::K(k), val))
                }
                LValue::UpVal(u) => {
                    self.emit(IR::SetUpVal(u, val));
                }
                LValue::SetByIndex(tab, idx) => {
                    // todo!()
                    let wh = self.ensure_allocated(idx);
                    self.emit(IR::SetFieldI(tab, wh, val));
                }
            }
        }

        Ok(())
    }

    #[inline]
    fn enter(&mut self, is_func: bool) {
        self.jumper.enter();
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
        self.scopes.exit();
        self.jumper.exit_and_resolve(&mut self.instructions)
    }

    fn gen_func_block(
        &mut self,
        FuncBody(params, Block(stmts, ret)): FuncBody,
        self_call: bool,
    ) -> Result<DukaIR, DukaCodegenError> {
        let has_var_arg = params.iter().any(|p| matches!(p, Param::Var(..)));
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
            && let StmtKind::Return(items) = (*ret).0
        {
            let span = (*ret).1;
            let start = irg.instructions.len();

            if items.len() == 1
                && items
                    .first()
                    .is_some_and(|v| matches!(v.0, ExprKind::Call(..)))
            {
                let mut items = items;
                let ExprKind::Call(callee, params) = items.pop().unwrap().0 else {
                    unreachable!()
                };
                irg.gen_call(*callee, params, true)?;
            } else {
                irg.emit(IR::Return(items.len()));
                irg.do_exprs(items)?;
            }

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
        })
    }

    fn gen_expr_block(&mut self, Block(stmts, ret): Block) -> Result<ExpDesc, DukaCodegenError> {
        self.enter(false);

        for stmt in stmts {
            self.gen_stmt(stmt.0)?;
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

        let exp = self.do_exprs(items)?;

        // let regs: Result<Vec<_>, _> = items
        //     .into_iter()
        //     .map(|item| {
        //         self.do_expr(item.0)
        //             .map(|exp| self.take_first(exp))
        //             .map(|pl| self.ensure_allocated(pl))
        //     })
        //     .collect();

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

    fn gen_stmt(&mut self, stmt: StmtKind) -> Result<(), DukaCodegenError> {
        use StmtKind::*;

        if stmt.is_empty() {
            return Ok(());
        }
        stmt.is_sugar()
            .then_error(|| DukaCodegenErrorKind::UnsupportedFeature(stmt.to_string()))?;

        match stmt {
            Label(label) => {
                self.jumper.label(label, self.instructions.len());
            }
            Goto(to) => {
                self.instructions
                    .push(self.jumper.goto(&to, self.instructions.len()));
            }

            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);

                self.jumper.branch_start(); // 当分支条件为真时 负责运行完分支后跳到最后面

                self.jumper.onetime_jmp(&mut self.instructions); // 当分支条件为假时 负责跳到下一个分支处

                self.gen_block_scoped(if_.0, false)?;
                self.jumper.branch_jmp(&mut self.instructions);

                for ifelse in ifelses {
                    self.jumper.onetime_end(&mut self.instructions);

                    self.jumper.onetime_jmp(&mut self.instructions);

                    self.gen_block_scoped(ifelse.0, false)?;
                    self.jumper.branch_jmp(&mut self.instructions);
                }

                self.jumper.onetime_end(&mut self.instructions);
                if let Some(blk) = else_ {
                    self.gen_block_scoped(blk, false)?;
                }

                self.jumper.branch_end(&mut self.instructions);
            }

            While(cond, blk) => {
                self.jumper.enter_loop(self.instructions.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper
                    .exit_loop(self.instructions.len(), &mut self.instructions);
            }
            ForGeneric(vars, from, blk) => {
                self.jumper.enter_loop(self.instructions.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper
                    .exit_loop(self.instructions.len(), &mut self.instructions);
            }
            ForNumberic(var, from, cond, step, blk) => {
                self.jumper.enter_loop(self.instructions.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper
                    .exit_loop(self.instructions.len(), &mut self.instructions);
            }

            Do(blk) => {
                self.gen_block_scoped(blk, false)?;
            }
            Function(name, _attrs, body, global) => {
                let ir = self.gen_func_block(body, name.is_self_call())?;
                self.nesteds.push(ir);
                let reg = self.allocator.alloc();
                self.instructions
                    .push(IR::Closure(reg, self.nesteds.len() - 1));
                let assign_to = self.set_to_path(name, global)?;
            }

            Define(attrnames, vals, global) => {
                let mut vals = vals.into_iter();
                for (((name, _), attrs), _) in attrnames {
                    let expr = vals.next();
                    let is_const = attrs.iter().any(|(a, _)| a == catt::CONST);

                    if is_const {
                        let cv = expr
                            .map(|e| self.ensure_const(e.0))
                            .transpose()?
                            .unwrap_or_default();
                        self.scopes.declare_const(&name, self.constants.add(cv));
                        continue;
                    }

                    let val = expr
                        .map(|e| self.do_expr(e.0))
                        .unwrap_or_else(|| Ok(ExpDesc::Single(Place::R(self.load_nil()))))?;
                    let pl = self.take_first(val);
                    let wh = self.ensure_allocated(pl);
                    self.scopes.declare_local(&name, wh);
                }
            }
            Assign(names, vals) => {
                let lefts: Result<Vec<_>, _> = names
                    .into_iter()
                    .map(|path| self.set_to_path(path, false))
                    .collect();
                let vals: Result<Vec<_>, _> =
                    vals.into_iter().map(|expr| self.do_expr(expr.0)).collect();

                //self.gen_assign(lefts?, vals?)?;
            }

            Break => {
                self.instructions
                    .push(self.jumper.loop_break(self.instructions.len()));
            }
            Continue => {
                self.instructions
                    .push(self.jumper.loop_continue(self.instructions.len()));
            }

            Call(callee, params) => {
                self.gen_call(callee, params, false)?;
                //self.irs.push(IR::Call(0))
            }

            _ => unreachable!(),
        }

        Ok(())
    }

    fn gen_main(&mut self, blk: Block) -> Result<(), DukaCodegenError> {
        self.gen_block_scoped(blk, true)?;
        Ok(())
    }
}

impl DukaGenerator<DukaIR> for IRGenerator {
    type InputType = DukaChunk;

    fn generate(input: Self::InputType) -> Result<DukaIR, DukaCodegenError> {
        let mut generator = Self::new();
        generator.gen_main(input.chunk)?;
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
    jumping: IRJumper,

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

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            jumping: IRJumper::new(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            nested_protos: vec![],
        }
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = DukaIR;

    fn generate(ir: Self::InputType) -> Result<DukaProto, DukaCodegenError> {
        // let DukaChunk {
        //     chunk,
        //     span: _,
        //     logic,
        // } = chunk;
        // let logic = LogicGenerator::generate(logic)?;
        // let mut proto = Self::new().generate_proto(chunk, Some("main".to_owned()), None, true)?;
        // proto.logic = Some(logic);
        // Ok(proto)
        todo!()
    }
}
