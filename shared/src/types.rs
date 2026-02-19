use crate::constants::{MetaMethod, MetaMethodAction};
use crate::errors::{DukaErrorKind, DukaIRError, DukaSpannedError, Span};
use crate::utils::UniqueVec;
use crate::value::DukaInt;
use duka_macros::Info;
pub use duka_macros::{Visitor, VisitorMut, binops};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::fmt::Display;
use std::io::Read;
use std::ops::{Add, Range, Sub};
use std::sync::Arc;
use std::time::Instant;

pub type BangName = String;
pub type BangData = HashMap<BangName, Box<dyn Any>>;

#[derive(Debug, PartialEq, Eq, Info, Clone, Serialize, Deserialize)]
pub enum UnOp {
    Length,
    Not,
    BitNot,
    Minus,
}
#[derive(Debug, PartialEq, Eq, Info, Clone, Serialize, Deserialize)]
pub enum BinOp {
    #[tag(ari)]
    Add,
    #[tag(ari)]
    Sub,
    #[tag(ari)]
    Multiply,
    #[tag(ari)]
    Divide,
    #[tag(ari)]
    IDivide,
    #[tag(ari)]
    Mod,
    #[tag(ari)]
    Pow,

    #[tag(logic)]
    #[tag(short)]
    And,
    #[tag(logic)]
    #[tag(short)]
    Or,
    #[tag(logic)]
    Xor,

    #[tag(compare)]
    #[tag(single)]
    Equal,
    #[tag(compare)]
    #[tag(single)]
    NotEqual,
    #[tag(compare)]
    #[tag(single)]
    Greater,
    #[tag(compare)]
    #[tag(single)]
    Less,
    #[tag(compare)]
    #[tag(single)]
    GreaterEqual,
    #[tag(compare)]
    #[tag(single)]
    LessEqual,

    #[tag(bits)]
    BitAnd,
    #[tag(bits)]
    BitOr,
    #[tag(bits)]
    BitXor,
    #[tag(bits)]
    ShiftL,
    #[tag(bits)]
    ShiftR,

    #[tag(concat)]
    Concat,
    #[tag(sugar)]
    Pipeline,
    #[tag(sugar)]
    PipelineL,
}

impl BinOp {
    pub fn get_meta_method(&self) -> Option<(MetaMethod, MetaMethodAction)> {
        use MetaMethod::*;
        Some((
            match self {
                BinOp::Add => Add,
                BinOp::Sub => Sub,
                BinOp::Multiply => Mul,
                BinOp::Divide => Div,
                BinOp::IDivide => IDiv,
                BinOp::Mod => Mod,
                BinOp::Pow => Pow,
                BinOp::BitAnd => BAnd,
                BinOp::BitOr => BOr,
                BinOp::BitXor => BXor,
                BinOp::ShiftL => ShL,
                BinOp::ShiftR => ShR,
                BinOp::Concat => Concat,

                BinOp::Less => LT,
                BinOp::LessEqual => LE,
                BinOp::Equal => Eq,

                BinOp::NotEqual => return Some((Eq, MetaMethodAction::Inverse)),
                BinOp::Greater => return Some((LE, MetaMethodAction::Swap)),
                BinOp::GreaterEqual => return Some((LT, MetaMethodAction::Swap)),

                _ => return None,
            },
            MetaMethodAction::Default,
        ))
    }
}

pub type Spanned<T> = (T, Span);

pub type RawToken<T> = Result<T, DukaSpannedError>;

/// Stream of token produced by DukaLexer
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TokenStream<T> {
    pub tokens: Box<[T]>,
    #[serde(skip)]
    pub source_info: SourceInfo,
}

pub trait DukaLexer<Source: Read> {
    type TokenType;

    fn from_source(source: Source, source_name: Option<String>) -> Self;
    fn tokenize(self) -> Result<TokenStream<Self::TokenType>, DukaSpannedError>;
}

pub trait DukaParser<T> {
    type ChunkType;

    fn parse(stream: TokenStream<T>) -> Result<Self::ChunkType, DukaSpannedError>;
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
pub fn Complete<T, S, E>(val: T) -> DukaResult<T, S, E> {
    DukaResult::Ok(DukaResumable::Complete(val))
}
#[allow(non_snake_case)]
#[inline(always)]
pub fn Incomplete<T, S, E>(
    val: S,
    info: SourceInfo,
    expected: Box<str>,
    span: Span,
) -> DukaResult<T, S, E> {
    DukaResult::Ok(DukaResumable::Incomplete(val, info, expected, span))
}

#[derive(Debug)]
pub enum DukaResumable<T, S> {
    Complete(T),
    Incomplete(S, SourceInfo, Box<str>, Span),
}
pub type DukaResult<T, S, E = DukaSpannedError> = Result<DukaResumable<T, S>, E>;

impl<T, S> From<DukaResumable<T, S>> for Result<T, DukaSpannedError> {
    fn from(value: DukaResumable<T, S>) -> Self {
        match value {
            DukaResumable::Complete(e) => Ok(e),
            DukaResumable::Incomplete(_, source_info, expected, at) => Err(DukaSpannedError::new(
                DukaErrorKind::Incomplete(expected),
                at,
                source_info,
            )),
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
    pub inst_spans: Box<[(Range<usize>, Span)]>,
    pub all_span: Span,
    pub debug_name: Option<Box<str>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub name: Option<Arc<str>>,
    pub source: Arc<[u8]>,
    pub time: Instant,
}
impl Default for SourceInfo {
    fn default() -> Self {
        SourceInfo {
            name: None,
            source: vec![].into(),
            time: Instant::now(),
        }
    }
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
