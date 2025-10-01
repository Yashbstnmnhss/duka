use crate::ast::{Block, Expr, ExprKind, FuncBody, IfClause, Match, MatchClause, Stmt, StmtKind};
use crate::error::{DukaCodegenError, DukaSpannedError, Span};
use crate::token::TokenKind;
use crate::value::{ConstValue, DukaInt};

pub use duka_macros::{Visitor, VisitorMut, binops};

pub trait Printer {
    fn print();
}

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
        (**self).visit(visitor);
    }
}
impl<T: VisitMut> VisitMut for Box<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        (**self).visit_mut(visitor);
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

impl<A: Visit, B: Visit> Visit for (A, B) {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        self.0.visit(visitor);
        self.1.visit(visitor);
    }
}
impl<A: VisitMut, B: VisitMut> VisitMut for (A, B) {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        self.0.visit_mut(visitor);
        self.1.visit_mut(visitor);
    }
}

impl<A, B, C: Visit> Visit for (A, B, C) {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        self.2.visit(visitor);
    }
}
impl<A, B, C: VisitMut> VisitMut for (A, B, C) {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        self.2.visit_mut(visitor);
    }
}

pub trait Visitor {
    fn visit_stmt(&mut self, _stmt: &Stmt) {}
    fn visit_expr(&mut self, _expr: &Expr) {}
    fn visit_if_clause_block(&mut self, _block: &IfClause, _enter: bool) {}
    fn visit_match_else_block(&mut self, _block: &Match, _enter: bool) {}
    fn visit_match_clause_block(&mut self, _block: &MatchClause, _enter: bool) {}
    fn visit_func_block(&mut self, _block: &FuncBody, _enter: bool) {}
    fn visit_do_stmt_block(&mut self, _block: &StmtKind, _enter: bool) {}
    fn visit_do_expr_block(&mut self, _block: &ExprKind, _enter: bool) {}
    fn visit_loop_stmt_block(&mut self, _block: &StmtKind, _enter: bool) {}

    fn report(&self) -> Vec<DukaSpannedError> {
        vec![]
    }
}
pub trait VisitorMut {
    fn visit_stmt(&mut self, _stmt: &mut Stmt) {}
    fn visit_expr(&mut self, _expr: &mut Expr) {}

    fn visit_block(&mut self, _enter: bool) {}
}

pub type Spanned<T> = (T, Span);

pub trait DukaLexer<TokenType> {
    fn next(&mut self) -> Result<TokenType, DukaSpannedError>;
    fn span(&self) -> Span;
}

pub trait DukaParser {
    type ChunkType;

    fn parse(self) -> Result<Self::ChunkType, DukaSpannedError>;
}

pub trait DukaAnalyzer {
    type InputType;

    fn analyze(self, chunk: &Self::InputType) -> Vec<DukaSpannedError>;
}
pub trait DukaAdapter {
    type InputType;

    fn adapt(self, chunk: &mut Self::InputType);
}

pub trait DukaGenerator<OutputType> {
    type InputType;

    fn generate(self, chunk: Self::InputType) -> Result<OutputType, DukaCodegenError>;
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
    Bool(bool),
    String(String),
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

#[derive(Debug, PartialEq, Clone)]
pub enum LogicOp {
    Or,
    And,
}

binops! {
    as get_logicop_info
    type TokenKind -> LogicOp = LogicOpInfo:

    SemiColon => Or;

    Comma => And

    这里是logic的op_同样是递增的
}

#[derive(Debug, Clone)]
pub struct DukaChunk {
    pub chunk: Block,
    pub span: Span,
    pub logic: LogicDatabase,
}

// Runtime environment only for duka vm backend, excepts for other compiling targets
// pub trait DukaRuntime {
//     type ValueType;
//     fn get_stack(&mut self, ad: u8) -> &Self::ValueType;
//     fn set_stack(&mut self, ad: u8, val: Self::ValueType);
// }
