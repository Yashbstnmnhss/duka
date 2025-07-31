use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::{Block, Path, StmtKind};
use crate::shared::types::DukaProto;
use crate::shared::value::Value;

pub mod binary;

pub fn generate(program: Block) -> DukaProto {
    let mut constants: Vec<Value> = vec![];
    let mut instructions: Vec<Instruction> = vec![];

    for (stmts, _) in program.0 {
        match stmts {
            // StmtKind::Expr(expr) => match expr {
            //     ExprKind::Call(callee, _) => {
            //         if let ExprKind::Access(Path::Base((name, _))) = (*callee).0 {
            //             constants.push(name.into());
            //             instructions.push(Instruction::GetGlobal(0, (constants.len() - 1) as u32));
            //         }
            //         constants.push(Value::Nil);
            //         instructions.push(Instruction::LoadConst(1, (constants.len() - 1) as u32));
            //         instructions.push(Instruction::Call(0, 1));
            //     }
            //     _ => todo!(),
            // },
            _ => todo!(),
        }
    }

    DukaProto {
        constants,
        instructions,
    }
}
