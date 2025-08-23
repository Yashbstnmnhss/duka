use std::collections::HashMap;
use std::usize;

use crate::types::DukaProto;
use crate::vm::instructions::{Address, Bits17, Instruction as I, SignedBits17};
use duka_shared::ast::{Expr, ExprKind, Stmt, StmtKind};
use duka_shared::types::{DukaChunk, DukaGenerator};
use duka_shared::value::Value;

pub mod binary;

#[derive(Debug)]
pub struct Generator {
    constants: Vec<Value>,
    constants_index: HashMap<Value, usize>,

    instructions: Vec<I>,
}

impl Generator {
    fn add_const(&mut self, val: Value) -> usize {
        self.constants_index
            .get(&val)
            .map(|v| *v)
            .unwrap_or_else(|| {
                let i = self.constants.len();
                self.constants.push(val.clone());
                self.constants_index.insert(val, i);
                i
            })
    }
    fn load_const(&mut self, val: Value, a: Address) -> I {
        let i = self.add_const(val);
        I::LoadK(a, i as Bits17)
    }
    fn emit(&mut self, ins: I) {
        self.instructions.push(ins);
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: vec![],
            constants_index: HashMap::new(),

            instructions: vec![],
        }
    }

    fn do_chunk(&mut self, program: DukaChunk) {
        for Stmt(stmt, _) in program.chunk.0 {
            match stmt {
                StmtKind::Empty => continue,
                StmtKind::Define(..) => todo!(),
                StmtKind::Continue => todo!(),
                StmtKind::Break => todo!(),
                StmtKind::Label(..) => todo!(),
                StmtKind::Expr(_) => todo!(),
                StmtKind::Call(_, items) => todo!(),
                StmtKind::Goto(_) => todo!(),
                StmtKind::Return(items) => todo!(),

                StmtKind::If(_) => todo!(),
                StmtKind::ForNumberic(path, _, _, _, block) => todo!(),
                StmtKind::ForGeneric(paths, items, block) => todo!(),
                StmtKind::While(_, block) => todo!(),
                StmtKind::Do(block) => todo!(),
                StmtKind::Assign(paths, items) => todo!(),
                StmtKind::Function(path, items, func_body, _) => todo!(),

                sk if sk.is_sugar() => unimplemented!(),
                _ => unreachable!(),
            }
        }
    }

    fn do_expr(&mut self, expr: Expr) {
        match expr.0 {
            ExprKind::Literal(val) => todo!(),
            _ => todo!(),
        }
    }
    fn do_val(&mut self, val: Value) {
        match val {
            Value::Bool(b) => self.emit(if b { I::LoadTrue(0) } else { I::LoadFalse(0) }),
            Value::Nil => self.emit(I::LoadNil(0)),
            Value::Int(i) => {
                if let Ok(n) = SignedBits17::try_from(i) {
                    self.emit(I::LoadI(0, n))
                } else {
                    let c = self.load_const(val, 0);
                    self.emit(c)
                }
            }
            Value::Float(_) => {
                let c = self.load_const(val, 0);
                self.emit(c);
            }
            _ if val.is_string() => {
                let c = self.load_const(val, 0);
                self.emit(c);
            }
            _ => unimplemented!(),
        }
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = DukaChunk;

    fn generate(mut self, chunk: DukaChunk) -> DukaProto {
        self.do_chunk(chunk);
        DukaProto {
            constants: self.constants,
            instructions: self.instructions,
            upvalue_count: 1, // _ENV
            param_count: 1,   // ...
            nested_protos: vec![],
            max_stack_size: usize::MAX,
            debug_name: None,
        }
    }
}
