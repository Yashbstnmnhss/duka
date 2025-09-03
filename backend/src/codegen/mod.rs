use std::collections::{HashMap, HashSet};
use std::usize;

use crate::types::DukaProto;
use crate::value::RuntimeValue;
use crate::vm::instructions::{Address, Bits17, Instruction as I, SignedBits17};
use duka_shared::ast::{Block, Expr, ExprKind, FuncBody, Stmt, StmtKind};
use duka_shared::error::DukaCodegenError;
use duka_shared::error::DukaCodegenErrorKind::*;
use duka_shared::types::DukaGenerator;
use duka_shared::value::ConstValue;

pub mod binary;
mod descriptor;

#[derive(Debug, Default)]
struct Constants(Vec<RuntimeValue>, HashMap<RuntimeValue, usize>);
impl Constants {
    fn add(&mut self, val: RuntimeValue) -> usize {
        self.1.get(&val).map(|v| *v).unwrap_or_else(|| {
            let i = self.0.len();
            self.0.push(val.clone());
            self.1.insert(val, i);
            i
        })
    }
    fn into_vec(self) -> Vec<RuntimeValue> {
        self.0
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
    // this has infinite registers
    fn alloc(&mut self) -> usize {
        let idx = self.current.free_list.pop().unwrap_or_else(|| {
            let res = self.current.top;
            self.current.top += 1;
            res
        });
        self.current.allocated.insert(idx);
        idx
    }

    fn free(&mut self, idx: usize) {
        if self.current.allocated.remove(&idx) {
            self.current.free_list.push(idx)
        }
    }
}

/// ### This `struct` represents two items:
/// - **Label**(name, target_pos)
/// - **PendingGoto**(target_name, goto_inst_pos)
#[derive(Debug)]
struct JumpInfo(String, usize);
#[derive(Debug)]
struct GotoManager {
    labels: Vec<Vec<JumpInfo>>,
    pending_gotos: Vec<JumpInfo>,
}
impl GotoManager {
    fn new() -> Self {
        Self {
            labels: vec![vec![]],
            pending_gotos: vec![],
        }
    }
    fn enter(&mut self) {
        self.labels.push(vec![]);
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
            instructions[goto_pos] = I::Jump(Self::calc_jump_param(label_pos, goto_pos));
        }
        Ok(())
    }
    #[inline(always)]
    const fn calc_jump_param(label_pos: usize, goto_pos: usize) -> i32 {
        label_pos as i32 - goto_pos as i32
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
    /// ## Create a `jump` opcode
    /// If method `declare_goto` returned `None`, then the **sJ** parameter will be zero, waiting to be resolved
    fn create_goto(&mut self, label: &str, goto_pos: usize) -> I {
        let target = self.declare_goto(label, goto_pos);
        I::Jump(
            target
                .map(|label_pos| Self::calc_jump_param(label_pos, goto_pos))
                .unwrap_or(0),
        )
    }
}

#[derive(Debug)]
pub struct Generator {
    constants: Constants,
    scopes: Scopes,
    allocator: Allocator,
    goto_manager: GotoManager,

    instructions: Vec<I>,
}

impl Generator {
    fn load_const(&mut self, val: RuntimeValue, a: Address) -> I {
        let i = self.constants.add(val);
        I::LoadK(a, i as Bits17)
    }
    fn emit(&mut self, inst: I) {
        self.instructions.push(inst);
    }
    fn top(&self) -> usize {
        self.instructions.len()
    }

    fn enter(&mut self) {
        self.scopes.enter();
        self.allocator.enter();
        self.goto_manager.enter();
    }

    fn exit(&mut self) -> Result<(), DukaCodegenError> {
        self.scopes.exit();
        self.allocator.exit();
        self.goto_manager.exit_and_resolve(&mut self.instructions)?;
        Ok(())
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            goto_manager: GotoManager::new(),
            instructions: vec![],
        }
    }

    fn do_stmt(&mut self, stmt: StmtKind) -> Result<(), DukaCodegenError> {
        match stmt {
            StmtKind::Empty => (), // nothing
            StmtKind::Define(..) => todo!(),
            StmtKind::Continue => todo!(),
            StmtKind::Break => todo!(),
            StmtKind::Label(name) => {
                self.goto_manager.declare_label(name, self.top());
            }
            StmtKind::Expr(_) => todo!(),
            StmtKind::Call(_, items) => todo!(),
            StmtKind::Goto(label) => {
                let inst = self.goto_manager.create_goto(&label, self.top());
                self.emit(inst);
            }
            StmtKind::Return(items) => self.do_return()?,

            StmtKind::If(_) => todo!(),
            StmtKind::ForNumberic(path, _, _, _, block) => todo!(),
            StmtKind::ForGeneric(paths, items, block) => todo!(),
            StmtKind::While(_, block) => todo!(),
            StmtKind::Do(block) => todo!(),
            StmtKind::Assign(paths, items) => todo!(),
            StmtKind::Function(path, attrs, FuncBody(params, block), is_global) => {
                self.do_block_with_scope(block)?;
            }

            sk if sk.is_sugar() => unimplemented!(),
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

    fn gen_proto(&mut self, block: Block) -> Result<DukaProto, DukaCodegenError> {
        let gt = Self::new();
        gt.generate(block)
    }

    fn do_block_with_scope(&mut self, block: Block) -> Result<(), DukaCodegenError> {
        self.enter();
        self.do_block(block)?;
        self.exit()?;
        Ok(())
    }

    fn do_return(&mut self) -> Result<(), DukaCodegenError> {
        Ok(())
    }

    fn do_expr(&mut self, expr: Expr) -> Result<(), DukaCodegenError> {
        match expr.0 {
            ExprKind::Literal(val) => todo!(),
            _ => todo!(),
        }
        Ok(())
    }
    fn do_const_val(&mut self, val: ConstValue) -> Result<(), DukaCodegenError> {
        match val {
            ConstValue::Bool(b) => self.emit(if b { I::LoadTrue(0) } else { I::LoadFalse(0) }),
            ConstValue::Nil => self.emit(I::LoadNil(0, 1)),
            ConstValue::Int(i) => {
                if let Ok(n) = SignedBits17::try_from(i) {
                    self.emit(I::LoadI(0, n))
                } else {
                    let c = self.load_const(val.into(), 0);
                    self.emit(c)
                }
            }
            ConstValue::String(_) | ConstValue::Float(_) => {
                let c = self.load_const(val.into(), 0);
                self.emit(c);
            }
            _ => unimplemented!(),
        }
        Ok(())
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = Block;

    fn generate(mut self, chunk: Self::InputType) -> Result<DukaProto, DukaCodegenError> {
        self.do_block(chunk)?;
        Ok(DukaProto {
            constants: self.constants.into_vec(),
            instructions: self.instructions,
            upvalue_count: 1, // _ENV
            param_count: 0,
            has_vararg: true, // ...
            nested_protos: vec![],
            debug_name: None,
        })
    }
}
