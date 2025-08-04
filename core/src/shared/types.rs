use crate::backend::vm::instructions::Instruction;
use crate::frontend::ast::Block;
use crate::shared::error::{DukaError, Span};
use crate::shared::value::{DukaInt, Value};

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

#[derive(Debug, Default)]
pub struct LogicDatabase {
    pub facts: Vec<Fact>,
    pub rules: Vec<Rule>,
}

#[derive(Debug)]
pub struct Fact(pub String, pub Vec<Term>);

#[derive(Debug)]
pub struct Rule(pub String, pub Vec<Term>, pub Vec<Goal>);

#[derive(Debug)]
pub enum Term {
    Atom(String),
    Number(DukaInt),
    Var(String),
    Compound(String, Vec<Term>),
    // List, Op ...
}

#[derive(Debug)]
pub enum Goal {
    Term(Term),
    And(Vec<Goal>),
    Or(Vec<Goal>),
    If(Box<Goal>, Box<Goal>, Option<Box<Goal>>),
    Not(Box<Goal>),
    Cut,
    // Unify, Compare ...
}

#[derive(Debug)]
pub struct DukaChunk {
    pub chunk: Block,
    pub span: Span,
    pub logic: LogicDatabase,
}

#[derive(Debug)]
pub struct DukaProto {
    pub constants: Vec<Value>,
    pub instructions: Vec<Instruction>,
}
