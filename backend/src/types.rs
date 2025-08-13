/// # TODO: MOVE TO SHARED
use duka_shared::value::Value;

use crate::vm::instructions::Instruction;

#[derive(Debug, Clone)]
pub struct DukaProto {
    pub constants: Vec<Value>,
    pub instructions: Vec<Instruction>,
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto);
}
