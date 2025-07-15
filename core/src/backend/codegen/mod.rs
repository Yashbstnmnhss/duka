use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::{BlockStmt, Expr, Stmt};
use crate::shared::types::DukaProto;
use crate::shared::value::Value;

pub mod types;

pub fn generate(program: BlockStmt) -> DukaProto {
    let mut constants: Vec<Value> = vec![];
    let mut instructions: Vec<Instruction> = vec![];

    for stmts in program.stmts {
        match stmts {
            Stmt::Expr(expr) => match expr {
                Expr::Call { callee, args: _ } => {
                    if let Expr::Ident { name } = *callee {
                        constants.push(name.into());
                        instructions.push(Instruction::GetGlobal(0, (constants.len() - 1) as u32));
                    }
                    constants.push(Value::Nil);
                    instructions.push(Instruction::LoadConst(1, (constants.len() - 1) as u32));
                    instructions.push(Instruction::Call(0, 1));
                }
                _ => todo!(),
            },
            _ => todo!(),
        }
    }

    DukaProto {
        constants,
        instructions,
    }
}
