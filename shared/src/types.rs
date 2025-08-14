use std::ops::Mul;

use crate::ast::Block;
use crate::error::{DukaError, Span};
use crate::value::DukaInt;

pub trait Visit {
    fn visit<V: Visitor>(&self, visitor: &mut V);
}
pub trait VisitMut {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V);
}

impl<T: Visit> Visit for Option<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        if let Some(self_) = self {
            self_.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Option<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        if let Some(self_) = self {
            self_.visit_mut(visitor);
        }
    }
}

impl<T: Visit> Visit for Box<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        (*self).visit(visitor);
    }
}
impl<T: VisitMut> VisitMut for Box<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        (*self).visit_mut(visitor);
    }
}

impl<T: Visit> Visit for Vec<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        for self_ in self {
            self_.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Vec<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        for self_ in self {
            self_.visit_mut(visitor);
        }
    }
}

pub trait Visitor {}
pub trait VisitorMut {}

pub type Spanned<T> = (T, Span);
impl<T> Mul<T> for Span {
    type Output = Spanned<T>;
    fn mul(self, rhs: T) -> Self::Output {
        (rhs, self)
    }
}

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
