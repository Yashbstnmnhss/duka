use std::any::Any;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::Read;
use std::ops::{Add, Range, Sub};

use crate::ast::{Block, Expr, ExprKind, FuncBody, IfClause, Match, MatchClause, Stmt, StmtKind};
use crate::error::{DukaErrorKind, DukaIRError, DukaSpannedError, Span};
use crate::token::{Token, TokenKind};
use crate::utils::UniqueVec;
use crate::value::DukaInt;
pub use duka_macros::{Visitor, VisitorMut, binops};
use serde::{Deserialize, Serialize};

pub type BangName = String;
pub type BangData = HashMap<BangName, Box<dyn Any>>;

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
impl<T: Visit> Visit for Box<[T]> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        for el in self {
            el.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Box<[T]> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        for el in self {
            el.visit_mut(visitor);
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

    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
}
pub trait VisitorMut {
    fn visit_stmt(&mut self, _stmt: &mut Stmt) {}
    fn visit_expr(&mut self, _expr: &mut Expr) {}

    fn visit_block(&mut self, _enter: bool) {}
}

pub type Spanned<T> = (T, Span);

pub type RawToken<T> = Result<T, DukaSpannedError>;

pub trait DukaLexer<Source: Read> {
    type TokenType;

    fn from_source(source: Source) -> Self;

    fn next_token(&mut self) -> RawToken<Self::TokenType>;
    fn span(&self) -> Span;
    fn source(&self) -> &str;
}

pub trait DukaParser<I: Iterator<Item = RawToken<Token>>> {
    type ChunkType;

    fn parse(stream: I) -> Result<Self::ChunkType, DukaSpannedError>;
}

pub trait DukaAnalyzer: Sized {
    type InputType;

    fn analyze(&self, chunk: &Self::InputType) -> impl Iterator<Item = DukaSpannedError>;
    fn chain<N: DukaAnalyzer<InputType = Self::InputType>>(
        self,
        next: N,
    ) -> AnalyzerChain<Self, N> {
        AnalyzerChain(self, next)
    }
}
pub struct AnalyzerChain<A: DukaAnalyzer, B: DukaAnalyzer>(A, B);
impl<I, A: DukaAnalyzer<InputType = I>, B: DukaAnalyzer<InputType = I>> DukaAnalyzer
    for AnalyzerChain<A, B>
{
    type InputType = I;
    fn analyze(&self, chunk: &Self::InputType) -> impl Iterator<Item = DukaSpannedError> {
        self.0.analyze(chunk).chain(self.1.analyze(chunk))
    }
}

pub trait DukaAdapter: Sized {
    type InputType;

    fn adapt(&self, chunk: &mut Self::InputType);
    fn chain<N: DukaAdapter<InputType = Self::InputType>>(self, next: N) -> AdapterChain<Self, N> {
        AdapterChain(self, next)
    }
}
pub struct AdapterChain<A: DukaAdapter, B: DukaAdapter>(A, B);
impl<I, A: DukaAdapter<InputType = I>, B: DukaAdapter<InputType = I>> DukaAdapter
    for AdapterChain<A, B>
{
    type InputType = I;
    fn adapt(&self, chunk: &mut Self::InputType) {
        self.0.adapt(chunk);
        self.1.adapt(chunk);
    }
}

pub trait DukaGenerator<OutputType, E = DukaIRError> {
    type InputType;

    fn generate(input: Self::InputType) -> Result<OutputType, E>;
}

#[allow(non_snake_case)]
#[inline(always)]
pub const fn Complete<T, S, E>(val: T) -> DukaResult<T, S, E> {
    DukaResult::Ok(DukaResumable::Complete(val))
}
#[allow(non_snake_case)]
#[inline(always)]
pub const fn Incomplete<T, S, E>(val: S, expected: String, span: Span) -> DukaResult<T, S, E> {
    DukaResult::Ok(DukaResumable::Incomplete(val, expected, span))
}

#[derive(Debug)]
pub enum DukaResumable<T, S> {
    Complete(T),
    Incomplete(S, String, Span),
}
pub type DukaResult<T, S, E = DukaSpannedError> = Result<DukaResumable<T, S>, E>;

impl<T, S> From<DukaResumable<T, S>> for Result<T, DukaSpannedError> {
    fn from(value: DukaResumable<T, S>) -> Self {
        match value {
            DukaResumable::Complete(e) => Ok(e),
            DukaResumable::Incomplete(_, expected, at) => Err(DukaSpannedError {
                kind: DukaErrorKind::Incomplete(expected),
                span: at,
            }),
        }
    }
}

/* FOR LOGIC PROGRAMMING */

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum QueryCount {
    Binding(String),
    Exact(usize),
    All,
}
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum SysCall {
    Query(usize, QueryCount),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DebugInfo {
    pub inst_spans: Vec<(Range<usize>, Span)>,
    pub all_span: Span,
    pub debug_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// # 值的数量
pub enum ValueCount {
    /// `VarArg`: *`0` in number representing*
    VarArg,
    /// `Exact(n)`: *`n + 1` in number representing*
    Exact(usize),
}
impl PartialEq<usize> for ValueCount {
    fn eq(&self, other: &usize) -> bool {
        match self {
            Self::Exact(n) => n.eq(other),
            _ => false,
        }
    }
}
impl PartialOrd<usize> for ValueCount {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        match self {
            Self::Exact(n) => Some(n.cmp(other)),
            _ => Some(std::cmp::Ordering::Greater),
        }
    }
}
impl Add<usize> for ValueCount {
    type Output = Self;
    fn add(self, rhs: usize) -> Self::Output {
        match self {
            ValueCount::VarArg => ValueCount::VarArg,
            ValueCount::Exact(n) => ValueCount::Exact(n + rhs),
        }
    }
}
impl Sub<usize> for ValueCount {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self::Output {
        match self {
            ValueCount::VarArg => ValueCount::VarArg,
            ValueCount::Exact(n) => ValueCount::Exact(n.saturating_sub(rhs)),
        }
    }
}
impl Display for ValueCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueCount::Exact(n) => write!(f, "{n}"),
            ValueCount::VarArg => write!(f, "..."),
        }
    }
}
impl ValueCount {
    pub fn format_register(&self, from: usize) -> String {
        match self {
            Self::Exact(0) => "empty".to_owned(),
            Self::Exact(n) => format!("R[{from}] to R[{}]", from + n - 1),
            Self::VarArg => format!("R[{from}] to ..."),
        }
    }
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Exact(0))
    }
    /// Convert `ValueCount` to its index in given stack
    pub const fn to_index(&self, stack_len: usize) -> usize {
        match self {
            ValueCount::VarArg => stack_len,
            ValueCount::Exact(n) => *n,
        }
    }
}
// only used for instruction
impl From<usize> for ValueCount {
    #[inline]
    fn from(val: usize) -> Self {
        if val == 0 {
            ValueCount::VarArg
        } else {
            ValueCount::Exact(val - 1)
        }
    }
}
impl From<ValueCount> for usize {
    #[inline]
    fn from(val: ValueCount) -> Self {
        match val {
            ValueCount::VarArg => 0,
            ValueCount::Exact(n) => n + 1,
        }
    }
}
impl From<ValueCount> for u8 {
    fn from(val: ValueCount) -> Self {
        Into::<u32>::into(val) as u8
    }
}
// only used for API function or coroutine returning
impl From<ValueCount> for u32 {
    #[inline]
    fn from(val: ValueCount) -> Self {
        Into::<usize>::into(val) as u32
    }
}
impl From<u8> for ValueCount {
    #[inline]
    fn from(val: u8) -> Self {
        Into::<ValueCount>::into(val as u32)
    }
}
impl From<u32> for ValueCount {
    #[inline]
    fn from(val: u32) -> Self {
        Into::<ValueCount>::into(val as usize)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogicDatabase {
    pub facts: Vec<Fact>,
    pub rules: Vec<Rule>,
    pub queries: UniqueVec<Query>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact(pub String, pub Vec<Term>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule(pub String, pub Vec<Term>, pub Goal);

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Eq, Hash)]
pub struct Query(pub Goal);

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Eq, Hash)]
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

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Eq, Hash)]
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

    Priority_Increasing
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct DukaChunk {
    pub chunk: Block,
    pub span: Span,
    pub logic: Box<LogicDatabase>,
}
