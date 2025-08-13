use crate::ast::Block;
use crate::error::{DukaError, Span};
use crate::value::DukaInt;

pub type Spanned<T> = (T, Span);

pub trait DukaLexer<TokenType> {
    fn next(&mut self) -> Result<TokenType, DukaError>;
    fn span(&self) -> Span;
}

pub trait DukaParser {
    type ChunkType;

    fn parse(&mut self) -> Result<Self::ChunkType, DukaError>;
}

pub trait DukaAnalyzer {
    type InputType;

    fn analyze(self, chunk: &Self::InputType) -> Vec<DukaError>;
}
pub trait DukaAdapter {
    type InputType;

    fn adapt(self, chunk: &mut Self::InputType);
}

pub trait DukaGenerator<OutputType> {
    type InputType;

    fn generate(self, chunk: Self::InputType) -> OutputType;
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
