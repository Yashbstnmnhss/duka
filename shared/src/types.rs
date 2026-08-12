use crate::config::{DukaLexerConfig, DukaParserConfig};
use crate::errors::{DukaErrorKind, DukaIRError, DukaSpannedError, Span};
use crate::utils::UniqueVec;
use crate::value::DukaInt;
use duka_macros::Info;
pub use duka_macros::{Visitor, VisitorMut, binops};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

pub type Spanned<T> = (T, Span);

pub type RawToken<T> = Result<T, DukaSpannedError>;

/// Stream of token produced by DukaLexer
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TokenStream<T> {
    pub tokens: Box<[T]>,
    pub source_info: SourceInfo,
}

impl<T> TokenStream<T> {
    pub fn new(tokens: Box<[T]>, source_info: SourceInfo) -> Self {
        Self {
            tokens,
            source_info,
        }
    }
}

/// Common lexer trait for duka, generic type indicates the source (implementing `Read` trait)
pub trait DukaLexer<Source: Read> {
    type TokenType;

    /// Accept source and its name (optional), return a lexer instance
    fn from_source(source: Source, source_name: Option<String>, config: DukaLexerConfig) -> Self;
    /// Consume lexer itself, return the stream of tokens
    fn tokenize(self) -> Result<TokenStream<Self::TokenType>, DukaSpannedError>;
}

/// Common parser trait for duka, generic type indicates its token type
pub trait DukaParser<T> {
    type ChunkType;

    /// Accept a stream of tokens (completely), return parsed chunk
    fn parse(
        stream: TokenStream<T>,
        config: DukaParserConfig,
    ) -> Result<Self::ChunkType, DukaSpannedError>;
}
/// Common analyzer trait for duka. This is used to analyze errors in static code
pub trait DukaAnalyzer: Sized {
    type InputType;
    type InputData;
    type OutputData;

    /// Accept a chunk, apply analyzing rules on it, return iterator of reports (if it has)
    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>);
    /// Chain two analyzer, apply them orderly
    fn chain<N>(self, next: N) -> AnalyzerChain<Self, N>
    where
        N: DukaAnalyzer<InputType = Self::InputType, InputData = Self::OutputData>,
    {
        AnalyzerChain(self, next)
    }
}
pub struct AnalyzerChain<A: DukaAnalyzer, B: DukaAnalyzer>(A, B);
impl<I, A: DukaAnalyzer<InputType = I>, B: DukaAnalyzer<InputType = I, InputData = A::OutputData>>
    DukaAnalyzer for AnalyzerChain<A, B>
{
    type InputType = I;
    type InputData = A::InputData;
    type OutputData = B::OutputData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        let (data, reports) = self.0.analyze(chunk, data);
        let (data, reports2) = self.1.analyze(chunk, data);
        (data, reports.chain(reports2))
    }
}

/// Common syntax adapter trait for duka. This is used to modify, remove and add nodes in parsed AST
pub trait DukaAdapter: Sized {
    type InputType;

    /// Accept mutable reference of chunk, this will modify it
    fn adapt(&self, chunk: &mut Self::InputType);
    /// Chain two adapters, apply them orderly
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

/// Common generator for duka. This can be used to generate IR from AST, and generate target code from IR
pub trait DukaGenerator<OutputType, E = DukaIRError> {
    type InputType;
    type ConfigType;

    /// Consume parsed chunk, return generated code
    fn generate(input: Self::InputType, config: Self::ConfigType) -> Result<OutputType, E>;
}

#[allow(non_snake_case)]
#[inline(always)]
pub fn Complete<T, S, E>(val: T) -> DukaResult<T, S, E> {
    Ok(DukaResumable::Complete(val))
}
#[allow(non_snake_case)]
#[inline(always)]
pub fn Incomplete<T, S, E>(
    val: S,
    info: SourceInfo,
    expected: Box<str>,
    span: Span,
) -> DukaResult<T, S, E> {
    Ok(DukaResumable::Incomplete(val, info, expected, span))
}

#[derive(Debug)]
pub enum DukaResumable<T, S> {
    Complete(T),
    Incomplete(S, SourceInfo, Box<str>, Span),
}
impl<T, S> DukaResumable<T, S> {
    pub fn map_to_result<E, F>(self, op: F) -> Result<T, E>
    where
        F: FnOnce(S, SourceInfo, Box<str>, Span) -> E,
    {
        match self {
            Self::Complete(t) => Ok(t),
            Self::Incomplete(a, b, c, d) => Err(op(a, b, c, d)),
        }
    }
    pub fn map<U, F>(self, op: F) -> DukaResumable<U, S>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Complete(t) => DukaResumable::Complete(op(t)),
            Self::Incomplete(a, b, c, d) => DukaResumable::Incomplete(a, b, c, d),
        }
    }
    pub fn into_result(self) -> Result<T, DukaSpannedError> {
        match self {
            DukaResumable::Complete(t) => Ok(t),
            DukaResumable::Incomplete(_, source_info, expected, span) => Err(
                DukaSpannedError::new(DukaErrorKind::Incomplete(expected), span, source_info),
            ),
        }
    }
}

pub type DukaResult<T, S, E = DukaSpannedError> = Result<DukaResumable<T, S>, E>;

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
    pub source_info: SourceInfo,
}

mod serde_opt_arc_str {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &Option<Arc<str>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => serializer.serialize_none(),
            Some(string) => serializer.serialize_str(string.as_ref()),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Arc<str>>, D::Error> {
        let opt = Option::<String>::deserialize(deserializer)?;
        Ok(opt.map(Arc::from))
    }
}
mod serde_arc_slice {
    use super::*;
    pub fn serialize<S: Serializer>(value: &Arc<[u8]>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(value.as_ref())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<[u8]>, D::Error> {
        let vec = Vec::<u8>::deserialize(deserializer)?;
        Ok(Arc::from(vec))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    #[serde(with = "serde_opt_arc_str")]
    pub name: Option<Arc<str>>,
    #[serde(with = "serde_arc_slice")]
    pub source: Arc<[u8]>,
    #[serde(skip, default = "Instant::now")]
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
