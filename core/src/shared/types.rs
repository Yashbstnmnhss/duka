use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::Block;
use crate::shared::error::{DukaError, Span};
use crate::shared::value::Value;

pub type Spanned<T> = (T, Span);

pub trait DukaLexer<Token> {
    fn next(&mut self) -> Result<Token, DukaError>;
    fn span(&self) -> Span;
}

pub trait DukaParser {
    fn parse(&mut self) -> Result<Block, DukaError>;
}

pub trait DukaAnalyzer {
    fn analyze(&mut self, chunk: &Block) -> Result<(), Vec<DukaError>>;
}

pub trait DukaCodegen {
    fn generate(&mut self, chunk: &DukaChunk) -> DukaProto;
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto);
}

#[derive(Debug)]
pub struct DukaChunk {
    pub chunk: Block,
    pub span: Span,
    pub globals: Vec<String>,
}

#[derive(Debug)]
pub struct DukaProto {
    pub constants: Vec<Value>,
    pub instructions: Vec<Instruction>,
}
