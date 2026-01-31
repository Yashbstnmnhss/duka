use std::usize;

use crate::DebugInfo;
use crate::codegen::types::{Allocator, Constants, DukaIR, IR, Place, Reg, Scopes};
use crate::{
    instructions::{Address, Bits17, DecodeInstruction, Instruction as I},
    value::DukaProto,
};
use duka_shared::constants::cgen;
use duka_shared::error::DukaCodegenError;
use duka_shared::types::{DukaChunk, DukaGenerator};
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use duka_shared::{
    ast::{
        Block, Expr, ExprKind, Field, FuncBody, If, IfClause, Param, Path, PathSuffix, Stmt,
        StmtKind,
    },
    error::DukaCodegenErrorKind::{self, *},
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

#[cfg(test)]
mod tests {
    use duka_shared::{
        ast::{Block, Expr, ExprKind, Field, Path, PathSuffix, Stmt, StmtKind},
        error::Span,
        value::ConstValue,
    };

    use crate::codegen::IRGenerator;

    macro_rules! test_gen {
        () => {{
            let mut g = IRGenerator::new();
            g.scopes.enter(true);
            g
        }};
    }

    #[test]
    fn path_test() {
        let mut g = test_gen!();
        let v = g.get_from_path(
            Path::Base(("aa".into(), Span::EMPTY))
                + PathSuffix::Dot(("bb".into(), Span::EMPTY))
                + PathSuffix::Dot(("cc".into(), Span::EMPTY)),
        );
        println!("{:?}", v);
        println!("{:#?}", g);
    }

    #[test]
    fn table_test() {
        let mut ir = test_gen!();
        let r = ir.do_expr(ExprKind::Do(Block(
            vec![],
            Some(Box::new(Stmt(
                StmtKind::Return(vec![Expr(
                    ExprKind::Table(vec![
                        Field::Value(Expr(
                            ExprKind::Literal(ConstValue::Bool(false)),
                            Span::EMPTY,
                        )),
                        Field::Value(Expr(
                            ExprKind::Literal(ConstValue::Bool(false)),
                            Span::EMPTY,
                        )),
                        Field::KeyValue(
                            Expr(ExprKind::Table(vec![]), Span::EMPTY),
                            Expr(ExprKind::Literal(ConstValue::Bool(true)), Span::EMPTY),
                        ),
                    ]),
                    Span::EMPTY,
                )]),
                Span::EMPTY,
            ))),
        )));
        println!("{:?}", r);
        println!("{:#?}", &ir);
        println!("{:?}", ir.irs);
    }

    #[test]
    fn call_test() {
        let mut ir = test_gen!();
        let r = ir.do_expr(ExprKind::Call(
            Box::new(Expr(
                ExprKind::Access(
                    Path::Base(("cc".into(), Span::EMPTY))
                        + PathSuffix::Colon(("name".to_owned(), Span::EMPTY)),
                ),
                Span::EMPTY,
            )),
            vec![Expr(
                ExprKind::Literal("fuck".to_owned().into()),
                Span::EMPTY,
            )],
        ));
        println!("{:?}", r);
        println!("{:#?}", &ir);
        println!("{:?}", ir.irs);
    }
}

#[derive(Debug)]
pub struct IRGenerator {
    allocator: Allocator,
    jumper: IRJumper,

    irs: Vec<IR>,
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
    /// (table, key, is_const?)
    Index(usize, usize, bool),
    /// (table, key)
    IndexString(usize, usize),
}

impl IRGenerator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            jumper: IRJumper::new(),
            debug_info: DebugInfo::default(),
            irs: vec![],
        }
    }

    fn gen_block_raw(&mut self, Block(stmts, ret): Block) -> Result<(), DukaCodegenError> {
        for Stmt(stmt, span) in stmts {
            let start = self.irs.len();

            self.gen_stmt(stmt)?;

            let end = self.irs.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }

        if let Some(ret) = ret
            && let StmtKind::Return(items) = (*ret).0
        {
            let span = (*ret).1;
            let start = self.irs.len();

            self.irs.push(IR::Return());
            for expr in items {
                self.do_expr(expr.0)?;
            }
            self.irs.push(IR::Return());

            let end = self.irs.len();
            self.debug_info.inst_spans.insert(start..end, span);
        }

        Ok(())
    }

    /// always call this for block generation
    fn gen_block_scoped(&mut self, blk: Block, is_func: bool) -> Result<(), DukaCodegenError> {
        self.enter(is_func);

        self.gen_block_raw(blk)?;

        self.exit(is_func)?;

        Ok(())
    }

    fn set_to_path(&mut self, path: Path) -> Result<LValue, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                // push name into constant pool
                if let Some(pl) = self.scopes.find(&name) {
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
                let base = self.set_to_path(*parent)?;
                match suffix {
                    PathSuffix::Colon(func) => {
                        return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                            format!(
                                "trying to assign value(s) to a function self-calling ({})",
                                func.0
                            ),
                        )));
                    }
                    PathSuffix::Dot((name, _)) => {}
                    PathSuffix::Index(idx) => {
                        let idx = self.do_expr((*idx).0)?;
                    }
                }
                todo!()
            }
            Path::Expr(_) => {
                return Err(DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                    "trying to assign value(s) to an expression".to_owned(),
                )));
            }
        })
    }

    /// Notice, this won't process colon path `obj:func`, which should be in function calling
    fn get_from_path(&mut self, path: Path) -> Result<Place, DukaCodegenError> {
        Ok(match path {
            Path::Base((name, _)) => {
                if let Some(idx) = self.scopes.find(&name) {
                    idx
                } else {
                    // _ENV
                    let idx = self.constants.add(name.into());
                    let env = self.scopes.ensure_global();
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::GetField(reg, env, Place::K(idx)));
                    Place::R(reg)
                }
            }
            Path::Expr(expr) => self.do_expr((*expr).0)?,
            Path::Chain(parent, suffix) => {
                let table = self.get_from_path(*parent)?;
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
                        self.irs.push(IR::GetField(reg, table, Place::K(key)));
                        Place::R(reg)
                    }
                    PathSuffix::Index(idx) => {
                        let idx = self.do_expr((*idx).0)?;
                        let reg = if let Place::R(r) = table {
                            r
                        } else {
                            self.allocator.alloc()
                        };
                        self.irs.push(IR::GetField(reg, table, idx));
                        Place::R(reg)
                    }
                }
            }
        })
    }

    fn ensure_allocated(&mut self, ai: Place) -> Reg {
        match ai {
            Place::K(k) => {
                let reg = self.allocator.alloc();
                self.irs.push(IR::LoadConst(reg, k));
                reg
            }
            Place::U(u) => {
                let reg = self.allocator.alloc();
                self.irs.push(IR::GetUpVal(reg, u));
                reg
            }
            Place::R(r) => r,
            // Place::I(v) => {
            //     let reg = self.allocator.alloc();

            //     self.irs.push(match v {
            //         ConstValue::Bool(b) => {
            //             b.then_some(IR::LoadTrue(reg)).unwrap_or(IR::LoadFalse(reg))
            //         }
            //         ConstValue::Int(i) => IR::LoadInt(reg, i),
            //         _ => todo!(),
            //     });
            //     reg
            // }
        }
    }

    fn gen_param(&mut self, params: Vec<Expr>) -> Result<usize, DukaCodegenError> {
        let len = params.len();
        for param in params {
            let place = self.do_expr(param.0)?;
            let reg = self.ensure_allocated(place);
            self.irs.push(IR::Param(reg));
        }
        Ok(len)
    }

    // DO NOT INPUT EMPTY EXPR
    ///
    fn do_expr(&mut self, expr: ExprKind) -> Result<Place, DukaCodegenError> {
        use ExprKind::*;

        expr.is_sugar().then_error(|| {
            DukaCodegenError::from(DukaCodegenErrorKind::UnsupportedFeature(expr.to_string()))
        })?;
        matches!(expr, ExprKind::Empty).then_error(|| {
            DukaCodegenError::from(DukaCodegenErrorKind::InvalidAST(
                "got empty expr".to_owned(),
            ))
        })?;

        Ok(match expr {
            VarArg => {
                self.irs.push(IR::VarArg(Reg::Idx(0), 0));
                todo!();
            }
            Literal(const_value) => match const_value {
                ConstValue::Nil => {
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::LoadNil(reg));
                    Place::R(reg)
                }
                ConstValue::Int(i) => {
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::LoadInt(reg, i));
                    Place::R(reg)
                }
                ConstValue::Float(f) => {
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::LoadFloat(reg, f));
                    Place::R(reg)
                }
                ConstValue::Bool(b) => {
                    let reg = self.allocator.alloc();
                    self.irs
                        .push(b.then_some(IR::LoadTrue(reg)).unwrap_or(IR::LoadFalse(reg)));
                    Place::R(reg)
                }
                ConstValue::ConstTable(array_map) => {
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::LoadConst(
                        reg,
                        self.constants.add(ConstValue::ConstTable(array_map)),
                    ));
                    Place::R(reg)
                }
                ConstValue::String(items) => {
                    let reg = self.allocator.alloc();
                    self.irs.push(IR::LoadString(reg, items));
                    Place::R(reg)
                }
            },
            Do(block) => self.gen_expr_block(block)?,
            Access(path) => self.get_from_path(path)?,
            Call(expr, exprs) => {
                let kind = (*expr).0;
                let self_ = matches!(
                    kind,
                    ExprKind::Access(ref p) if p.is_self_call()
                );
                let callee = self.do_expr(kind)?;

                if self_ {
                    self.irs.push(IR::SelF());
                }
                let count = self.gen_param(exprs)?;

                let reg = match callee {
                    Place::R(reg) => reg,
                    Place::U(u) => {
                        let reg = self.allocator.alloc();
                        self.irs.push(IR::GetUpVal(reg, u));
                        reg
                    }
                    _ => {
                        return Err(DukaCodegenError::from(
                            DukaCodegenErrorKind::UnsupportedFeature(
                                "trying to call a \"constant\" function, which shouldn't be there!"
                                    .to_owned(),
                            ),
                        ));
                    }
                };

                self.irs.push(IR::Call(reg, count, false));

                Place::R(self.allocator.alloc())
            }
            SysCall(sys_call) => {
                self.irs.push(IR::SysCall(sys_call));
                todo!()
            }
            Table(fields) => {
                let table = self.allocator.alloc();
                self.irs.push(IR::NewTable(table));

                let mut fields = fields.into_iter().peekable();
                while let Some(field) = fields.next() {
                    match field {
                        Field::KeyValue(k, v) => {
                            let k = self.do_expr(k.0)?;
                            let v = self.do_expr(v.0)?;
                            self.irs.push(IR::SetField(Place::R(table), k, v));
                        }
                        Field::NameValue((n, _), v) => {
                            let k = self.constants.add(n.into());
                            let v = self.do_expr(v.0)?;
                            self.irs.push(IR::SetField(Place::R(table), Place::K(k), v));
                        }
                        Field::Value(v) => {
                            let pl = self.do_expr(v.0)?;
                            let mut batch = vec![self.ensure_allocated(pl)];
                            while let Some(Field::Value(_)) = fields.peek() {
                                let Some(Field::Value(v)) = fields.next() else {
                                    unreachable!()
                                };
                                let pl = self.do_expr(v.0)?;
                                batch.push(self.ensure_allocated(pl));
                            }
                            self.irs.push(IR::SetArray(Place::R(table), batch.into()));
                        }
                        Field::Expand => {
                            todo!()
                        }
                    }
                }
                Place::R(table)
            }
            Function(func_body) => Place::K(1),
            Unary(expr, un_op) => {
                let operand = self.do_expr((*expr).0)?;
                let reg = self.allocator.alloc();
                self.irs.push(IR::Unary(reg, operand, un_op));
                Place::R(reg)
            }
            Binary(expr, expr1, bin_op) => {
                let left = self.do_expr((*expr).0)?;
                let right = self.do_expr((*expr1).0)?;
                let reg = self.allocator.alloc();
                self.irs.push(IR::Binary(reg, left, right, bin_op));
                Place::R(reg)
            }
            If(_) => todo!(),

            _ => unreachable!(),
        })
    }

    fn gen_assign(&mut self, lvs: Vec<LValue>, mut res: Vec<IR>) -> Result<(), DukaCodegenError> {
        res.reverse();
        for (i, left) in lvs.into_iter().enumerate() {
            let val = res.pop();

            match left {
                LValue::Local(reg) => {}
                _ => todo!(),
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
        self.jumper.exit_and_resolve(&mut self.irs)
    }

    fn gen_func_block(
        &self,
        FuncBody(params, block): FuncBody,
        self_call: bool,
    ) -> Result<DukaIR, DukaCodegenError> {
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
                    irg.scopes.declare_local(&name, irg.allocator.alloc_one()) //NOTICE, there already exist values
                }
                _ => break,
            }
        }

        irg.gen_block_raw(block)?;

        irg.exit(true)?;

        Ok(DukaIR {
            irs: irg.irs,
            constants: irg.constants,
            scopes: irg.scopes,
            debug_info: irg.debug_info,
            logic: None,
        })
    }

    fn gen_expr_block(&mut self, Block(stmts, ret): Block) -> Result<Place, DukaCodegenError> {
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

        let regs: Result<Vec<_>, _> = items
            .into_iter()
            .map(|item| {
                self.do_expr(item.0)
                    .map(|place| self.ensure_allocated(place))
            })
            .collect();

        self.exit(false)?;

        Ok(Place::R(regs?.into()))
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
                self.jumper.label(label, self.irs.len());
            }
            Goto(to) => {
                self.irs.push(self.jumper.goto(&to, self.irs.len()));
            }

            If(ifs) => {
                let (if_, ifelses, else_) = (ifs.0, ifs.1, ifs.2);

                self.jumper.branch_start(); // 当分支条件为真时 负责运行完分支后跳到最后面

                self.jumper.onetime_jmp(&mut self.irs); // 当分支条件为假时 负责跳到下一个分支处

                self.gen_block_scoped(if_.0, false)?;
                self.jumper.branch_jmp(&mut self.irs);

                for ifelse in ifelses {
                    self.jumper.onetime_end(&mut self.irs);

                    self.jumper.onetime_jmp(&mut self.irs);

                    self.gen_block_scoped(ifelse.0, false)?;
                    self.jumper.branch_jmp(&mut self.irs);
                }

                self.jumper.onetime_end(&mut self.irs);
                if let Some(blk) = else_ {
                    self.gen_block_scoped(blk, false)?;
                }

                self.jumper.branch_end(&mut self.irs);
            }

            While(cond, blk) => {
                self.jumper.enter_loop(self.irs.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper.exit_loop(self.irs.len(), &mut self.irs);
            }
            ForGeneric(vars, from, blk) => {
                self.jumper.enter_loop(self.irs.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper.exit_loop(self.irs.len(), &mut self.irs);
            }
            ForNumberic(var, from, cond, step, blk) => {
                self.jumper.enter_loop(self.irs.len());

                self.gen_block_scoped(blk, false)?;

                self.jumper.exit_loop(self.irs.len(), &mut self.irs);
            }

            Do(blk) => {
                self.gen_block_scoped(blk, false)?;
            }
            Function(name, _attrs, body, global) => {
                let ir = self.gen_func_block(body, name.is_self_call())?;
                if global {}
                let assign_to = self.set_to_path(name)?;
            }

            Define(attrnames, vals, global) => for (((name, _), attrs), _) in attrnames {},
            Assign(names, vals) => {
                let names: Result<Vec<_>, _> = names
                    .into_iter()
                    .map(|path| self.set_to_path(path))
                    .collect();
                let vals: Result<Vec<_>, _> =
                    vals.into_iter().map(|expr| self.do_expr(expr.0)).collect();

                //self.gen_assign(names?, vals?)?;
            }

            Break => {
                self.irs.push(self.jumper.loop_break(self.irs.len()));
            }
            Continue => {
                self.irs.push(self.jumper.loop_continue(self.irs.len()));
            }

            Call(callee, params) => {
                let callee_expr = self.do_expr(callee.0)?;
                let params: Result<Vec<_>, _> = params
                    .into_iter()
                    .map(|param| self.do_expr(param.0))
                    .collect();
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
            irs: generator.irs,
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
