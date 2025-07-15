use crate::backend::vm::instructions::Instruction;
use crate::shared::error::{DukaError, Span};
use crate::shared::value::Value;

pub type Spanned<T> = (T, Span);

pub trait DukaLexer<Token> {
    fn next(&mut self) -> Result<Token, DukaError>;
    fn span(&self) -> Span;
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto);
}

#[derive(Debug)]
pub struct DukaProto {
    pub constants: Vec<Value>,
    pub instructions: Vec<Instruction>,
}
