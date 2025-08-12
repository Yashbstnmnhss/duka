use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::{Block, Expr, ExprKind, StmtKind};
use crate::shared::types::{DukaChunk, DukaCodegen, DukaProto};
use crate::shared::value::Value;

pub mod binary;

#[derive(Debug)]
pub struct Generator {
    constants: Vec<Value>,
    instructions: Vec<Instruction>,
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: vec![],
            instructions: vec![],
        }
    }

    fn do_chunk(&mut self, program: DukaChunk) {
        for (stmt, _) in program.chunk.0 {
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
            Value::Bool(b) => todo!(),
            _ => todo!(),
        }
    }
}

impl DukaCodegen for Generator {
    fn generate(mut self, chunk: DukaChunk) -> DukaProto {
        self.do_chunk(chunk);
        DukaProto {
            constants: self.constants,
            instructions: self.instructions,
        }
    }
}
