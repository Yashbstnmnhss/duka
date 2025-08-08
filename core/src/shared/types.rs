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
    fn generate(self, chunk: DukaChunk) -> DukaProto;
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto);
}

#[derive(Debug, Default, Clone)]
pub struct LogicDatabase {
    pub facts: Vec<Fact>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Fact(pub String, pub Vec<Term>);

#[derive(Debug, Clone)]
pub struct Rule(pub String, pub Vec<Term>, pub Goal);

#[derive(Debug, Clone)]
pub enum Term {
    Atom(String), // abc "abc" 'abc'
    Number(DukaInt),
    Var(String),                          // Abc _abc
    Anonymous,                            // _
    Compound(String, Vec<Term>),          // father(a, b)
    List(Vec<Term>, Option<Box<Term>>),   // [a, b, c] [head|tail]
    Binary(Box<Term>, Box<Term>, String), // X + Y
}

#[derive(Debug, Clone)]
pub enum Goal {
    Term(Term),
    And(Vec<Goal>), // ,
    Or(Vec<Goal>),  // ;
    If(Box<Goal>, Box<Goal>, Option<Box<Goal>>),
    Not(Box<Goal>), // not
    Cut,            // !
    Unify(Term, Term),
    Compare(Term, Term, String),
    Meta(String, Vec<Term>),
}

#[derive(Debug, Clone)]
pub struct DukaChunk {
    pub chunk: Block,
    pub span: Span,
    pub logic: LogicDatabase,
}

#[derive(Debug, Clone)]
pub struct DukaProto {
    pub constants: Vec<Value>,
    pub instructions: Vec<Instruction>,
}
