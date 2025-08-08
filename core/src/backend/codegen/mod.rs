use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::{Block, StmtKind};
use crate::shared::types::{DukaChunk, DukaCodegen, DukaProto};
use crate::shared::value::Value;

pub mod binary;

#[derive(Debug)]
pub struct Generator {
    constants: Vec<Value>,
    instructions: Vec<Instruction>,
}

impl Generator {
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
                StmtKind::Match(_) => todo!(),
                StmtKind::Object => todo!(),
                StmtKind::If(_) => todo!(),
                StmtKind::ForNumberic(path, _, _, _, block) => todo!(),
                StmtKind::ForGeneric(paths, items, block) => todo!(),
                StmtKind::While(_, block) => todo!(),
                StmtKind::Do(block) => todo!(),
                StmtKind::Assign(paths, items) => todo!(),
                StmtKind::Function(path, items, func_body, _) => todo!(),
            }
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

pub fn generate(program: Block) -> DukaProto {
    let mut constants: Vec<Value> = vec![];
    let mut instructions: Vec<Instruction> = vec![];

    DukaProto {
        constants,
        instructions,
    }
}
