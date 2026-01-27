use std::collections::{HashMap, HashSet};
use std::usize;

use crate::codegen::logic::{LogicGenerator, LogicProto};
use crate::instructions::{
    Address, Bits9, Bits17, Bits25, Instruction as I, SignedBits17, SignedBits25,
};
use crate::value::DukaProto;
use duka_shared::ast::{
    Block, Expr, ExprKind, Field, FuncBody, If, IfClause, Param, Path, PathSuffix, Stmt, StmtKind,
};
use duka_shared::error::DukaCodegenErrorKind::{self, *};
use duka_shared::error::{DukaCodegenError, Span};
use duka_shared::types::{DukaChunk, DukaGenerator, LogicDatabase};
use duka_shared::utils::UniqueVec;
use duka_shared::value::ConstValue;

pub mod binary;
mod descriptor;
pub mod logic;

#[derive(Debug, Default)]
struct Constants(UniqueVec<ConstValue>);
impl Constants {
    fn add(&mut self, val: ConstValue) -> usize {
        self.0.push(val)
    }
    fn into_vec(self) -> Vec<ConstValue> {
        self.0.into_vec()
    }
}

#[derive(Debug, Copy, Clone)]
enum AllocIdx {
    R(usize),
    K(usize),
}
#[derive(Debug, Default)]
struct Scope {
    const_var: Vec<(String, usize)>,
    locals: Vec<(String, AllocIdx)>,
    upvalues: Vec<(String, AllocIdx)>,
}
impl Scope {
    fn declare(&mut self, name: String, alloc_pos: AllocIdx) {
        self.locals.push((name, alloc_pos))
    }
    fn find(&self, name: &str) -> Option<AllocIdx> {
        self.const_var
            .iter()
            .find_map(|(n, i)| (n == name).then_some(AllocIdx::K(*i)))
            .or_else(|| {
                self.locals
                    .iter()
                    .find_map(|(n, i)| (n == name).then_some(*i))
                    .or_else(|| {
                        self.upvalues
                            .iter()
                            .find_map(|(n, i)| (n == name).then_some(*i))
                    })
            })
    }
}

#[derive(Debug)]
struct Scopes(Vec<Scope>);
impl Scopes {
    fn new() -> Self {
        Self(vec![Scope::default()])
    }
    fn current(&self) -> &Scope {
        self.0.last().unwrap()
    }
    fn current_mut(&mut self) -> &mut Scope {
        self.0.last_mut().unwrap()
    }
    fn len(&self) -> usize {
        self.0.len()
    }

    fn find(&self, name: &str) -> Option<AllocIdx> {
        self.current()
            .find(name)
            .or_else(|| self.0.iter().rev().find_map(|scope| scope.find(name)))
    }

    fn enter(&mut self) {
        self.0.push(Scope::default());
    }
    fn exit(&mut self) {
        if self.len() == 1 {
            return;
        }
        self.0.pop();
    }
}

#[derive(Debug)]
struct Allocator {
    snapshots: Vec<AllocatorSnapshot>,
    current: AllocatorSnapshot,
}

/// into a function prototype
#[derive(Debug, Default)]
struct AllocatorSnapshot {
    top: usize,
    free_list: Vec<usize>,
    allocated: HashSet<usize>,
}

impl Allocator {
    fn new() -> Self {
        Self {
            snapshots: vec![],
            current: AllocatorSnapshot::default(),
        }
    }
    fn enter(&mut self) {
        let snapshot = std::mem::take(&mut self.current);
        self.snapshots.push(snapshot);
    }
    fn exit(&mut self) {
        if let Some(cur) = self.snapshots.pop() {
            self.current = cur
        }
    }
    // this has infinite registers NO!
    fn alloc(&mut self) -> usize {
        let idx = self.current.free_list.pop().unwrap_or_else(|| {
            let res = self.current.top;
            self.current.top += 1;
            res
        });
        self.current.allocated.insert(idx);
        idx
    }

    fn used_reg_count(&self) -> usize {
        self.current.allocated.len() + self.current.free_list.len()
    }

    fn free(&mut self, idx: usize) {
        if self.current.allocated.remove(&idx) {
            self.current.free_list.push(idx)
        }
    }
}

// todo!(循环的continue与break)

/// ### This `struct` represents two items:
/// - **Label**(name, target_pos)
/// - **PendingGoto**(target_name, goto_inst_pos)
#[derive(Debug)]
struct JumpInfo(String, usize);
#[derive(Debug, Default)]
struct Jumping {
    labels: Vec<Vec<JumpInfo>>, // labels of scopes

    loop_heads: Vec<usize>, // the start of every loop (contains itself)
    pending_breaks: Vec<Vec<usize>>, // position of pending breaks in loop scopes
    pending_gotos: Vec<JumpInfo>, // all pending gotos (jump backwards)

    pending_onetime: Vec<usize>,     //一次性
    pending_branch: Vec<Vec<usize>>, //多对一
}
impl Jumping {
    const PLACEHOLDER: i32 = 0;

    fn new() -> Self {
        Self {
            labels: vec![vec![]],
            loop_heads: vec![],
            pending_gotos: vec![],
            pending_breaks: vec![],
            pending_onetime: vec![],
            pending_branch: vec![],
        }
    }

    fn branch_start(&mut self) {
        self.pending_branch.push(vec![]);
    }
    /// NOTICE: THIS IS ONLY BACKWARD
    /// 聚合 多起点一终点
    fn branch_jmp(&mut self, instructions: &mut Vec<I>) {
        if let Some(v) = self.pending_branch.last_mut() {
            v.push(instructions.len());
        }
        instructions.push(I::Jump(Self::PLACEHOLDER))
    }
    fn branch_end(&mut self, instructions: &mut Vec<I>) {
        if let Some(is) = self.pending_branch.pop() {
            for idx in is {
                instructions[idx] = I::Jump(Self::calc_offset(instructions.len(), idx));
            }
        }
    }

    /// NOTICE: THIS IS ONLY BACKWARD
    /// 单独 一起点一终点
    fn onetime_jmp(&mut self, instructions: &mut Vec<I>) {
        self.pending_onetime.push(instructions.len());
        instructions.push(I::Jump(Self::PLACEHOLDER));
    }

    fn onetime_end(&mut self, instructions: &mut Vec<I>) {
        let pos = self.pending_onetime.pop().unwrap();
        instructions[pos] = I::Jump(Self::calc_offset(instructions.len(), pos));
    }

    fn loop_continue(&self, current: usize) -> I {
        let pos = *self
            .loop_heads
            .last()
            .expect("CONTINUE MUST BE USED IN A LOOP");
        let offset = Self::calc_offset(pos, current);
        I::Jump(offset)
    }
    fn loop_break(&mut self, current: usize) -> I {
        self.pending_breaks
            .last_mut()
            .expect("BREAK MUST BE USED IN A LOOP")
            .push(current);
        Self::placeholder()
    }

    fn enter(&mut self) {
        self.labels.push(vec![]);
    }
    fn enter_loop(&mut self, head: usize) {
        self.loop_heads.push(head);
        self.pending_breaks.push(vec![]);
    }
    fn exit_loop(&mut self, end: usize, instructions: &mut Vec<I>) {
        if let Some(breaks) = self.pending_breaks.pop() {
            for from in breaks {
                let offset = Self::calc_offset(end, from);
                instructions[from] = I::Jump(offset);
            }
        }
        self.loop_heads.pop();
    }
    fn exit_and_resolve(&mut self, instructions: &mut Vec<I>) -> Result<(), DukaCodegenError> {
        self.resolve_pendings(instructions)?;
        self.labels.pop();
        Ok(())
    }
    fn resolve_pendings(&mut self, instructions: &mut Vec<I>) -> Result<(), DukaCodegenError> {
        // JumpInfo is PendingGoto in this case
        for JumpInfo(name, goto_pos) in std::mem::take(&mut self.pending_gotos).into_iter() {
            let label_pos = self
                .find_label(&name)
                .ok_or_else(|| DukaCodegenError::from(UnsolvedGoto(name.to_owned())))?;
            instructions[goto_pos] = I::Jump(Self::calc_offset(label_pos, goto_pos));
        }
        Ok(())
    }
    #[inline(always)]
    const fn calc_offset(to: usize, from: usize) -> i32 {
        to as i32 - from as i32
    }
    #[inline(always)]
    fn placeholder() -> I {
        I::Jump(Self::PLACEHOLDER)
    }

    fn declare_label(&mut self, name: String, label_pos: usize) {
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
    fn jump(&mut self, label: &str, goto_pos: usize) -> I {
        let target = self.declare_goto(label, goto_pos);
        I::Jump(
            target
                .map(|label_pos| Self::calc_offset(label_pos, goto_pos))
                .unwrap_or(Self::PLACEHOLDER), // placeholder
        )
    }
}

#[derive(Debug)]
pub struct Generator {
    constants: Constants,
    scopes: Scopes,
    allocator: Allocator,
    jumping: Jumping,

    instructions: Vec<I>,
    nested_protos: Vec<DukaProto>,
}

impl Generator {
    fn load_const(&mut self, val: ConstValue, reg: Address) -> I {
        let i = self.constants.add(val);
        I::LoadK(reg, i as Bits17)
    }
    fn emit(&mut self, inst: I) {
        self.instructions.push(inst);
    }
    /// TOP OF INSTRUCIONS!
    fn top(&self) -> usize {
        self.instructions.len()
    }

    fn enter(&mut self) {
        self.scopes.enter();
        self.allocator.enter();
        self.jumping.enter();
    }

    fn exit(&mut self) -> Result<(), DukaCodegenError> {
        self.scopes.exit();
        self.allocator.exit();
        self.jumping.exit_and_resolve(&mut self.instructions)?;
        Ok(())
    }

    fn get_var_address(&mut self, name: &str) -> Result<AllocIdx, DukaCodegenError> {
        self.scopes
            .find(name)
            .ok_or_else(|| DukaCodegenErrorKind::UndefinedVariable(name.to_string()).into())
    }

    fn allocate_temp(&mut self) -> usize {
        self.allocator.alloc()
    }

    fn free_temp(&mut self, idx: usize) {
        self.allocator.free(idx);
    }

    fn path_assign(&mut self, path: Path) -> Result<(), DukaCodegenError> {
        match path {
            Path::Base((name, _)) => match self.get_var_address(&name)? {
                AllocIdx::R(pos) => {
                    self.emit(I::Move(pos as Address, 0));
                    Ok(())
                }
                AllocIdx::K(i) => Err(DukaCodegenErrorKind::UnsupportedFeature(name).into()),
            },
            Path::Expr(expr) => {
                self.do_expr(*expr)?;
                Ok(())
            }
            Path::Chain(base, suffix) => {
                let mut cur = *base;
                let mut temp_regs: Vec<usize> = vec![];
                let last_suffix = suffix;

                use PathSuffix::*;

                match last_suffix {
                    Dot((field, _)) => match cur {
                        Path::Base((name, _)) => match self.get_var_address(&name)? {
                            AllocIdx::R(tab_reg) => {
                                let key_idx = self.constants.add(field.into()) as usize;
                                self.emit(I::SetField(tab_reg as Address, key_idx as u8, 0, false));
                                Ok(())
                            }
                            AllocIdx::K(_) => {
                                Err(DukaCodegenErrorKind::UnsupportedFeature(name).into())
                            }
                        },
                        _ => {
                            Err(DukaCodegenErrorKind::UnsupportedFeature("path".to_string()).into())
                        }
                    },
                    Index(expr) => {
                        self.do_expr(*expr)?;
                        match cur {
                            Path::Base((name, _)) => match self.get_var_address(&name)? {
                                AllocIdx::R(tab_reg) => {
                                    self.emit(I::SetTable(tab_reg as Address, 0, 0, false));
                                    Ok(())
                                }
                                AllocIdx::K(_) => {
                                    Err(DukaCodegenErrorKind::UnsupportedFeature(name).into())
                                }
                            },
                            _ => {
                                Err(DukaCodegenErrorKind::UnsupportedFeature("path".to_string())
                                    .into())
                            }
                        }
                    }
                    Colon(_) => {
                        Err(DukaCodegenErrorKind::UnsupportedFeature("path".to_string()).into())
                    }
                }
            }
        }
    }
}

impl Generator {
    fn do_call_expr(&mut self, callee: Expr, args: Vec<Expr>) -> Result<(), DukaCodegenError> {
        let mut arg_regs = vec![];
        for arg in args.iter() {
            self.do_expr(arg.clone())?;
            let r = self.allocate_temp();
            self.emit(I::Move(r as Address, 0));
            arg_regs.push(r);
        }
        self.do_expr(callee)?;
        self.emit(I::Call(0, (arg_regs.len() + 1) as u8, 1));
        for r in arg_regs.into_iter().rev() {
            self.free_temp(r);
        }
        Ok(())
    }

    fn do_path_access(&mut self, path: Path) -> Result<(), DukaCodegenError> {
        match path {
            Path::Base((name, _)) => match self.scopes.find(&name) {
                Some(AllocIdx::R(idx)) => {
                    self.emit(I::Move(0, idx as Address));
                    Ok(())
                }
                Some(AllocIdx::K(i)) => {
                    self.emit(I::LoadK(0, i as Bits17));
                    Ok(())
                }
                None => Err(DukaCodegenErrorKind::UndefinedVariable(name).into()),
            },
            Path::Expr(expr) => {
                self.do_expr(*expr)?;
                Ok(())
            }
            Path::Chain(base, suffix) => {
                let mut cur = *base;
                use PathSuffix::*;
                match suffix {
                    Dot((field, _)) => match cur {
                        Path::Base((name, _)) => match self.scopes.find(&name) {
                            Some(AllocIdx::R(tab_reg)) => {
                                let key_idx = self.constants.add(field.into()) as Bits9;
                                self.emit(I::GetField(0, tab_reg as Address, key_idx));
                                Ok(())
                            }
                            Some(AllocIdx::K(_)) => {
                                Err(DukaCodegenErrorKind::UndefinedVariable(name).into())
                            }
                            None => Err(DukaCodegenErrorKind::UndefinedVariable(name).into()),
                        },
                        _ => Err(DukaCodegenErrorKind::UnsupportedFeature(todo!()).into()),
                    },
                    Index(expr) => match cur {
                        Path::Base((name, _)) => match self.scopes.find(&name) {
                            Some(AllocIdx::R(tab_reg)) => {
                                let tmp = self.allocate_temp();
                                self.emit(I::Move(tmp as Address, tab_reg as Address));
                                self.do_expr(*expr)?;
                                self.emit(I::GetTable(0, tmp as Address, 0));
                                self.free_temp(tmp);
                                Ok(())
                            }
                            _ => Err(DukaCodegenErrorKind::UndefinedVariable(name).into()),
                        },
                        _ => Err(DukaCodegenErrorKind::UnsupportedFeature(todo!()).into()),
                    },
                    Colon(_) => Err(DukaCodegenErrorKind::UnsupportedFeature(todo!()).into()),
                }
            }
        }
    }

    fn do_table_expr(&mut self, fields: Vec<Field>) -> Result<(), DukaCodegenError> {
        let mut array_count: usize = 0;
        let mut map_count: usize = 0;
        let mut instructions = vec![];

        let reg = self.allocate_temp() as Address;

        for field in fields {
            match field {
                Field::Value(v) => {
                    array_count += 1;

                    self.do_expr(v)?;
                }
                Field::KeyValue(k, v) => {
                    map_count += 1;

                    self.do_expr(v)?;

                    self.do_expr(k)?;
                    instructions.push(I::SetTable(reg, 0, 0, false));
                }
                Field::NameValue(k, v) => {
                    map_count += 1;

                    self.do_expr(v)?;

                    let key_idx = self.constants.add(k.0.into()) as Address;
                    instructions.push(I::SetField(reg, key_idx, todo!(), todo!()))
                }
            }
        }

        self.emit(I::ExtraArg(0)); // unimplemented
        self.emit(I::NewTable(
            reg,
            array_count as Address,
            map_count as Address,
        ));
        self.instructions.extend(instructions);

        Ok(())
    }

    fn do_function_expr(&mut self, func_body: FuncBody) -> Result<(), DukaCodegenError> {
        let param_count = func_body.0.len();
        let has_var_arg = func_body.0.iter().any(|p| matches!(p, Param::Var(_)));
        let proto = self.generate_proto(
            func_body.1,
            Some(FuncBody::ANONYMOUS.to_string()),
            Some(param_count),
            has_var_arg,
        )?;

        let idx = self.nested_protos.len();
        self.nested_protos.push(proto);
        self.emit(I::Closure(0, idx as Bits17));
        Ok(())
    }

    fn do_unary_expr(
        &mut self,
        expr: Expr,
        op: duka_shared::ast::UnOp,
    ) -> Result<(), DukaCodegenError> {
        self.do_expr(expr)?;
        use duka_shared::ast::UnOp::*;
        match op {
            Minus => self.emit(I::Minus(0, 0)),
            BitNot => self.emit(I::BitNot(0, 0)),
            Not => self.emit(I::Not(0, 0)),
            Length => self.emit(I::Length(0, 0)),
        }
        Ok(())
    }

    fn do_binary_expr(
        &mut self,
        left: Expr,
        right: Expr,
        op: duka_shared::ast::BinOp,
    ) -> Result<(), DukaCodegenError> {
        self.do_expr(left)?;
        let lreg = self.allocate_temp();
        self.emit(I::Move(lreg as Address, 0));
        self.do_expr(right)?;
        use duka_shared::ast::BinOp::*;
        match op {
            Add => self.emit(I::Add(0, lreg as Address, 0)),
            Sub => self.emit(I::Sub(0, lreg as Address, 0)),
            Multiply => self.emit(I::Mul(0, lreg as Address, 0)),
            Mod => self.emit(I::Mod(0, lreg as Address, 0)),
            Pow => self.emit(I::Pow(0, lreg as Address, 0)),
            Divide => self.emit(I::Div(0, lreg as Address, 0)),
            BitAnd => self.emit(I::BitAnd(0, lreg as Address, 0)),
            BitOr => self.emit(I::BitOr(0, lreg as Address, 0)),
            BitXor => self.emit(I::BitXor(0, lreg as Address, 0)),
            ShiftL => self.emit(I::ShiftL(0, lreg as Address, 0)),
            ShiftR => self.emit(I::ShiftR(0, lreg as Address, 0)),
            Concat => self.emit(I::Concat(0, 0)),
            Equal | NotEqual | Greater | Less | GreaterEqual | LessEqual => {
                self.emit(I::Equal(0, lreg as Address));
            }
            _ => return Err(DukaCodegenErrorKind::UnsupportedFeature(op.to_string()).into()),
        }

        self.emit(I::MMBinary(0, 0, 0));

        self.free_temp(lreg);
        Ok(())
    }

    fn do_if_expr(&mut self, ifexpr: If) -> Result<(), DukaCodegenError> {
        let If(if_, elseifs, else_) = ifexpr;
        let cond = *if_.1;
        self.do_expr(cond)?;
        self.emit(I::Test(0, false));
        self.jumping.onetime_jmp(&mut self.instructions);
        self.do_block_with_scope(if_.0)?;
        self.jumping.branch_jmp(&mut self.instructions);
        self.jumping.onetime_end(&mut self.instructions);

        for IfClause(block, cond) in elseifs {
            self.do_expr(*cond)?;
            self.emit(I::Test(0, false));
            self.jumping.onetime_jmp(&mut self.instructions);
            self.do_block_with_scope(block)?;
            self.jumping.branch_jmp(&mut self.instructions);
            self.jumping.onetime_end(&mut self.instructions);
        }

        if let Some(block) = else_ {
            self.do_block_with_scope(block)?;
        }

        self.jumping.branch_end(&mut self.instructions);
        Ok(())
    }

    fn do_return(&mut self, items: Vec<Expr>) -> Result<(), DukaCodegenError> {
        if items.is_empty() {
            self.emit(I::Return0())
        } else {
            let len = items.len();
            for (i, it) in items.into_iter().enumerate() {
                self.do_expr(it)?;
                if i != 0 {
                    self.emit(I::Move(i as Address, 0));
                }
            }
            self.emit(I::Return(0, len as u32));
        }
        Ok(())
    }

    fn expr_k_or_r(&mut self, Expr(expr, _): Expr) -> Result<usize, DukaCodegenError> {
        if let ExprKind::Literal(cv) = expr {
        } else {
        }
        Ok(1)
    }

    fn do_const_val(&mut self, val: ConstValue) -> Result<(), DukaCodegenError> {
        use ConstValue::*;
        Ok(match val {
            Bool(b) => self.emit(if b { I::LoadTrue(0) } else { I::LoadFalse(0) }),
            Nil => self.emit(I::LoadNil(0, 1)),
            Int(i) => {
                if let Some(n) = I::SignedBits17(i as usize) {
                    self.emit(I::LoadI(0, n))
                } else {
                    let c = self.load_const(val, 0);
                    self.emit(c)
                }
            }
            String(_) | Float(_) | ConstTable(_) => {
                let c = self.load_const(val, 0);
                self.emit(c);
            }
        })
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            jumping: Jumping::new(),
            instructions: vec![],
            nested_protos: vec![],
        }
    }

    fn do_stmt(&mut self, stmt: StmtKind) -> Result<(), DukaCodegenError> {
        match stmt {
            _ if stmt.is_empty() => (), // nothing
            StmtKind::Define(attrnames, mut vals, global) => {
                if global {}

                for ((name, _attrs), _) in attrnames {
                    let val = vals.pop();

                    match val {
                        Some(e) => self.do_expr(e)?,
                        None => self.emit(I::LoadNil(0, 1)),
                    };

                    let var_addr = self.allocate_temp();
                    self.scopes
                        .current_mut()
                        .declare(name.0, AllocIdx::R(var_addr));
                }
            }
            StmtKind::Continue => {
                let inst = self.jumping.loop_continue(self.top());
                self.emit(inst);
            }
            StmtKind::Break => {
                let inst = self.jumping.loop_break(self.top());
                self.emit(inst);
            }
            StmtKind::Label(name) => {
                self.jumping.declare_label(name, self.top());
            }
            StmtKind::Expr(expr) => self.do_expr(expr)?,
            StmtKind::Call(callee, args) => {
                let count = args.len() as u8;
                for arg in args {
                    self.do_expr(arg)?;
                }

                self.do_expr(callee)?;

                let res = self.allocate_temp();
                self.emit(I::Call(res as Address, count + 1, 1));
            }
            StmtKind::Goto(label) => {
                let inst = self.jumping.jump(&label, self.top());
                self.emit(inst);
            }
            StmtKind::Return(items) => self.do_return(items)?,

            StmtKind::If(If(if_, elseifs, else_)) => {
                self.jumping.branch_start();

                // if---
                let cond = *if_.1;
                self.do_expr(cond)?;
                self.emit(I::Test(0, false));
                // 条件不满足的跳跃
                self.jumping.onetime_jmp(&mut self.instructions);

                self.do_block_with_scope(if_.0)?;
                self.jumping.branch_jmp(&mut self.instructions); // 条件满足的跳跃

                self.jumping.onetime_end(&mut self.instructions);
                // fi---

                for IfClause(block, cond) in elseifs {
                    self.do_expr(*cond)?;
                    self.emit(I::Test(0, false));

                    self.jumping.onetime_jmp(&mut self.instructions);

                    self.do_block_with_scope(block)?;
                    self.jumping.branch_jmp(&mut self.instructions);

                    self.jumping.onetime_end(&mut self.instructions);
                }

                if let Some(block) = else_ {
                    self.do_block_with_scope(block)?;
                }

                self.jumping.branch_end(&mut self.instructions);
            }
            StmtKind::ForNumberic(path, start, cond, step, block) => {
                self.jumping.enter_loop(self.top());

                self.do_expr(start)?;
                let loop_var = self.allocate_temp();
                self.emit(I::Move(loop_var as Address, 0));

                if let Path::Base((name, _)) = path {
                    self.scopes
                        .current_mut()
                        .declare(name, AllocIdx::R(loop_var));
                }

                let loop_start = self.top();

                self.do_expr(cond)?;
                let cond_jump = self.top();
                self.emit(I::Test(0, false));

                self.do_block_with_scope(block)?;

                if let Some(step_expr) = step {
                    self.do_expr(step_expr)?;
                    self.emit(I::Add(loop_var as Address, loop_var as Address, 0));
                }

                let loop_offset = loop_start as i32 - self.top() as i32;
                self.emit(I::Jump(loop_offset));

                let cond_offset = self.top() as i32 - cond_jump as i32;
                self.instructions[cond_jump] = I::Test(0, false);

                self.jumping.exit_loop(self.top(), &mut self.instructions);
            }
            StmtKind::ForGeneric(paths, items, block) => {
                self.jumping.enter_loop(self.top());

                self.do_block_with_scope(block)?;
                self.jumping.exit_loop(self.top(), &mut self.instructions);
            }
            StmtKind::While(cond, block) => {
                self.jumping.enter_loop(self.top());

                let loop_start = self.top();
                self.do_expr(cond)?;
                let cond_jump = self.top();
                self.emit(I::Test(0, false)); // Test condition, break if false

                self.do_block_with_scope(block)?;

                // Jump back to condition
                let loop_offset = loop_start as i32 - self.top() as i32;
                self.emit(I::Jump(loop_offset));

                // Fix condition jump
                let cond_offset = self.top() as i32 - cond_jump as i32;
                self.instructions[cond_jump] = I::Test(0, false);

                self.jumping.exit_loop(self.top(), &mut self.instructions);
            }
            StmtKind::Do(block) => self.do_block_with_scope(block)?,
            StmtKind::Assign(paths, mut items) => {
                for path in paths {
                    let expr = items.pop().unwrap();
                    self.do_expr(expr)?;

                    self.path_assign(path)?;
                }
            }
            StmtKind::Function(path, _attrs, FuncBody(params, block), global) => {
                if global {}

                let param_count = params.len();
                let has_var_arg = params.iter().any(|p| matches!(p, Param::Var(_)));
                let proto = self.generate_proto(
                    block,
                    Some(path.to_string()),
                    Some(param_count),
                    has_var_arg,
                )?;

                let proto_idx = self.nested_protos.len();
                self.nested_protos.push(proto);
                let cls_reg = self.allocate_temp();

                self.emit(I::Closure(cls_reg as Address, proto_idx as Bits17));

                if let Path::Base((name, _)) = path {
                    let var_addr = self.allocate_temp();
                    self.scopes
                        .current_mut()
                        .declare(name, AllocIdx::R(var_addr));
                    self.emit(I::Move(var_addr as Address, cls_reg as Address));
                }
            }
            sk if sk.is_sugar() => {
                return Err(DukaCodegenErrorKind::UnsupportedFeature(sk.to_string()).into());
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn do_define(&mut self) -> Result<(), DukaCodegenError> {
        Ok(())
    }

    fn do_block(&mut self, block: Block) -> Result<(), DukaCodegenError> {
        for Stmt(stmt, _) in block.0 {
            self.do_stmt(stmt)?;
        }
        Ok(())
    }

    fn generate_proto(
        &mut self,
        block: Block,
        name: Option<String>,
        param_count: Option<usize>,
        has_var_arg: bool,
    ) -> Result<DukaProto, DukaCodegenError> {
        let param_count = param_count.unwrap_or_default();
        let mut proto = Self::new().generate_brief_proto(block, None)?;
        proto.debug_name = name;
        proto.param_count = param_count;
        proto.has_var_arg = has_var_arg;

        // ensure var arg maybe
        if has_var_arg {
            proto
                .instructions
                .insert(0, I::VarArgPrepare(param_count as Bits25));
        }
        // ensure safety, at least one instruction here
        proto.instructions.push(I::Return0());

        Ok(proto)
    }

    fn do_block_with_scope(&mut self, block: Block) -> Result<(), DukaCodegenError> {
        self.enter();
        self.do_block(block)?;
        self.exit()?;
        Ok(())
    }

    fn do_expr(&mut self, expr: Expr) -> Result<(), DukaCodegenError> {
        use ExprKind::*;
        match expr.0 {
            Literal(val) => self.do_const_val(val)?,

            SysCall(kind) => {
                self.emit(I::SysCall(0, 0, 0));
                unimplemented!();
            }

            VarArg => {
                self.emit(I::VarArg(0, 0));
            }
            Do(block) => {
                self.do_block_with_scope(block)?;
            }
            Access(path) => {
                self.do_path_access(path)?;
            }
            Call(callee, args) => {
                self.do_call_expr(*callee, args)?;
            }
            Table(fields) => {
                self.do_table_expr(fields)?;
            }
            Function(func_body) => {
                self.do_function_expr(func_body)?;
            }
            Unary(expr, op) => {
                self.do_unary_expr(*expr, op)?;
            }
            Binary(left, right, op) => {
                self.do_binary_expr(*left, *right, op)?;
            }
            If(if_expr) => {
                self.do_if_expr(if_expr)?;
            }
            Empty => {}
            ek if ek.is_sugar() => {
                return Err(DukaCodegenErrorKind::UnsupportedFeature(ek.to_string()).into());
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn generate_brief_proto(
        mut self,
        block: Block,
        logic: Option<LogicProto>,
    ) -> Result<DukaProto, DukaCodegenError> {
        self.do_block(block)?;
        Ok(DukaProto {
            constants: self.constants.into_vec(),
            instructions: self.instructions,
            upvalues: vec![],
            param_count: 0, //todo
            reg_count: self.allocator.used_reg_count(),
            has_var_arg: true, // ...
            nested_protos: self.nested_protos,
            debug_name: None,
            logic,
        })
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = DukaChunk;

    fn generate(chunk: Self::InputType) -> Result<DukaProto, DukaCodegenError> {
        let DukaChunk {
            chunk,
            span: _,
            logic,
        } = chunk;
        let logic = LogicGenerator::generate(logic)?;
        let mut proto = Self::new().generate_proto(chunk, Some("main".to_owned()), None, true)?;
        proto.logic = Some(logic);
        Ok(proto)
    }
}
