use std::{error::Error, fmt::Display, ops::Add};

use duka_macros::ThatError;

use crate::{constants::MAX_EXPANDING_DEPTH, types::SourceInfo};

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Copy, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
    pub at_char: u32,
}
pub const START_LINE: u32 = 1;
pub const START_COLUMN: u32 = 1;
pub const START_CHAR: u32 = 0;

impl Ord for Position {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.at_char.cmp(&other.at_char)
    }
}

impl Default for Position {
    fn default() -> Self {
        Self::START
    }
}
impl Position {
    pub const START: Self = Self {
        line: START_LINE,
        column: START_COLUMN,
        at_char: START_CHAR,
    };

    pub fn is_start(&self) -> bool {
        *self == Position::START
    }
    pub fn new_line(&mut self) {
        self.line += 1;
        self.column = START_COLUMN;
    }
    pub fn step(&mut self) {
        self.column += 1;
        self.at_char += 1;
    }
}
impl Add<usize> for Position {
    type Output = Span;
    fn add(self, rhs: usize) -> Self::Output {
        Span {
            start: self,
            end: Position {
                line: self.line,
                column: self.column + rhs as u32,
                at_char: self.at_char + rhs as u32,
            },
        }
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Copy, Default, serde::Serialize, serde::Deserialize)]
/** 左闭右开 */
pub struct Span {
    pub start: Position,
    pub end: Position,
}

impl Span {
    pub const EMPTY: Self = Self {
        start: Position::START,
        end: Position::START,
    };
    #[inline]
    pub const fn offset(&self) -> (u32, u32) {
        let Position {
            line: l1,
            column: c1,
            ..
        } = self.start;
        let Position {
            line: l2,
            column: c2,
            ..
        } = self.end;
        (l2 - l1, c2 - c1)
    }
    pub fn char_len(&self) -> u32 {
        let from = self.start.at_char.min(self.end.at_char);
        let to = self.start.at_char.max(self.end.at_char);
        to - from
    }
}

impl Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.start, self.end)
    }
}
impl Add for Span {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            start: self.start.min(rhs.start),
            end: self.end.max(rhs.end),
        }
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaErrorKind {
    #[error("[Lexer] {}")]
    Lexer(DukaLexerError),
    #[error("[Macro Expander] {}")]
    Macro(DukaMacroError),
    #[error("[Parser] {}")]
    Parser(DukaParserError),
    #[error("[Analyzer] {}")]
    Semantic(DukaSemanticError),
    #[error("[Incomplete Input] expected {}")]
    Incomplete(Box<str>),
}

impl DukaErrorKind {
    pub fn get_help(&self) -> String {
        match self {
            DukaErrorKind::Lexer(e) => e.get_help(),
            DukaErrorKind::Macro(e) => e.get_help(),
            DukaErrorKind::Parser(e) => e.get_help(),
            DukaErrorKind::Semantic(e) => e.get_help(),
            DukaErrorKind::Incomplete(e) => format!("Complete it with {e}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaSemanticError {
    #[error("Cannot use 'break' or 'continue' outside of a loop")]
    InvalidLoopFlowControl,
    #[error("Duplicated {} found: '{}' ")]
    DuplicatedItem(Box<str>, Box<str>),
    #[error("Invisible label '{}' for goto")]
    InvisibleGotoLabel(Box<str>),
    #[error("Cannot use var arg here")]
    InvalidVarArg,
}

impl DukaSemanticError {
    pub fn get_help(&self) -> String {
        match self {
            DukaSemanticError::InvalidLoopFlowControl => {
                format!("Move it inside a 'for' or 'while' loop")
            }
            DukaSemanticError::DuplicatedItem(what, who) => {
                format!("Duplicated {what} isn't supported, remove one of the {who}")
            }
            DukaSemanticError::InvisibleGotoLabel(label) => {
                format!(
                    "There is no available label {label} in nearest function scope, declare one in the same function scope or remove its goto"
                )
            }
            DukaSemanticError::InvalidVarArg => format!(
                "Var arg can only be used in top or a function which delcares '...' in its parameters list"
            ),
        }
    }
}

impl From<DukaSemanticError> for DukaErrorKind {
    fn from(value: DukaSemanticError) -> Self {
        DukaErrorKind::Semantic(value)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaParserError {
    #[error("Expected end")]
    ShouldBeEnd,
    #[error("Unexpected end")]
    UnexpectedEnd,
    #[error("Found unknown bang keyword: {}")]
    UnknownBang(Box<str>),
    #[error("Unexpected token {}, expected {}")]
    UnexpectedToken { got: Box<str>, expected: Box<str> },
    #[error("Duplicated name used: {}")]
    DuplicatedName(Box<str>),
    #[error("Found unknown operator: {}")]
    UnknownOperator(Box<str>),
    #[error("Invalid operator used: {}")]
    InvalidOperator(Box<str>),
}
impl DukaParserError {
    pub fn get_help(&self) -> String {
        match self {
            DukaParserError::ShouldBeEnd => format!("Something useless was also here, remove it"),
            DukaParserError::UnexpectedEnd => format!("Complete it"),
            DukaParserError::UnknownBang(name) => {
                format!("Check typo or register a custom bang handler with name {name}")
            }
            DukaParserError::UnexpectedToken { got, expected } => {
                format!("This should be {expected} or other valid tokens, remove {got} and fix it")
            }
            DukaParserError::DuplicatedName(name) => {
                format!("Duplicated {name} here isn't supported, change the name of any of them")
            }
            DukaParserError::UnknownOperator(op) => {
                format!("Remove invalid operator {op}, which isn't supported")
            }
            DukaParserError::InvalidOperator(op) => {
                format!("Check the usage of {op}, such as its associativity, priority and so on")
            }
        }
    }
}
impl From<DukaParserError> for DukaErrorKind {
    fn from(value: DukaParserError) -> Self {
        DukaErrorKind::Parser(value)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaMacroError {
    #[error("Invalid macro body")]
    InvalidMacroBody,
    #[error("Invalid parameters count: expected {}")]
    InvalidInputParameters(usize),
    #[error("Failed to load built-in macro")]
    FailedLoadBuiltin,
    #[error("Unknown parameter defined: named {}")]
    UnknownParameterDefined(Box<str>),
    #[error("Reached max depth of macro expanding: happened in {}")]
    ReachMaxDepth(Box<str>),
    #[error("Unknown macro: named {}")]
    UnknownMacro(Box<str>),
    #[error("Unknown built-in macro: named {}")]
    UnknownBuiltinMacro(Box<str>),
    #[error("Unexpected token in macro: expected {}")]
    UnexpectedToken(Box<str>),
}
impl DukaMacroError {
    pub fn get_help(&self) -> String {
        match self {
            DukaMacroError::InvalidMacroBody => format!(
                "Starts with '->' and ends with ';' to define single line macro, or use '#^enifed' to end the multiple line macro"
            ),
            DukaMacroError::InvalidInputParameters(count) => {
                format!("This macro requires at least {count} parameters")
            }
            DukaMacroError::FailedLoadBuiltin => {
                format!("This wouldn't happen technically, bro...")
            }
            DukaMacroError::UnknownParameterDefined(name) => format!(
                "Define a parameter named {name}, or just remove the `$` references to {name}"
            ),
            DukaMacroError::ReachMaxDepth(in_name) => format!(
                "Check macro {in_name}, infinite recursion may happened there, the limitation is {}",
                MAX_EXPANDING_DEPTH
            ),
            DukaMacroError::UnknownMacro(name) => {
                format!("Define the {name} macro before calling it, or remove the calling")
            }
            DukaMacroError::UnknownBuiltinMacro(name) => format!(
                "No such built-in macro named {name}, check typo or remove the '!' marker if you want to call user's macro"
            ),
            DukaMacroError::UnexpectedToken(e) => format!("Use {e}"),
        }
    }
}
impl From<DukaMacroError> for DukaErrorKind {
    fn from(value: DukaMacroError) -> Self {
        DukaErrorKind::Macro(value)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaLexerError {
    #[error("Reader error: {}")]
    ReaderError(Box<str>),
    #[error("Invalid integer format: {}")]
    InvalidInteger(Box<str>),
    #[error("Invalid float format: {}")]
    InvalidFloat(Box<str>),
    #[error("Unfinished string, {}")]
    UnfinishedString(Box<str>),
    #[error("Invalid escaped format: {}")]
    InvalidEscaped(Box<str>),
    #[error("Invalid unicode escaped: {}")]
    InvalidUnicodeEscaped(Box<str>),
    #[error("Invalid escaped format, expected {}")]
    UnexpectedEnd(Box<str>),
    #[error("Unexpected character: {}")]
    UnexpectedCharacter(char),
    #[error("Multiple line comment isn't finished, {}")]
    UnfinishedComment(Box<str>),
    #[error("Unknown character has been read: {}")]
    UnknownCharacter(Box<str>),
    #[error("Invalid input: Input is not valid utf8")]
    InvalidUtf8,
}
impl DukaLexerError {
    pub fn get_help(&self) -> String {
        match self {
            DukaLexerError::ReaderError(_) => format!("See error message"),
            DukaLexerError::InvalidInteger(_) => format!(
                "The format of given integer is invalid, ensure you have used radix prefix or other things correctly"
            ),
            DukaLexerError::InvalidFloat(_) => format!(
                "The format of given float is invalid, ensure you have used e/E, point or other things correctly"
            ),
            DukaLexerError::UnfinishedString(_) => format!(
                "To finish the string, complete its terminator with the same pattern of its start"
            ),
            DukaLexerError::InvalidEscaped(_) => format!("Check your escaped character in string"),
            DukaLexerError::InvalidUnicodeEscaped(_) => {
                format!("Badly use in \\u/\\U, ensure the code point is valid")
            }
            DukaLexerError::UnexpectedEnd(_) => format!("Complete the escaped pattern in string"),
            DukaLexerError::UnexpectedCharacter(c) => format!("Remove the invalid character: {c}"),
            DukaLexerError::UnfinishedComment(_) => format!(
                "To finish the comment, complete its terminator with the same pattern of its start"
            ),
            DukaLexerError::UnknownCharacter(_) => {
                format!("This shouldn't happen technically, bro...")
            }
            DukaLexerError::InvalidUtf8 => {
                format!("Check the encode of your input, duka only accept UTF-8 input")
            }
        }
    }
}

impl From<DukaLexerError> for DukaErrorKind {
    fn from(value: DukaLexerError) -> Self {
        DukaErrorKind::Lexer(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaSpannedError {
    pub kind: DukaErrorKind,
    pub span: Span,
    pub source_info: SourceInfo,
    pub related: Box<[(Box<str>, Span)]>,
}
impl DukaSpannedError {
    pub fn new(kind: DukaErrorKind, span: Span, source_info: SourceInfo) -> Self {
        Self {
            kind,
            span,
            source_info,
            related: Box::new([]),
        }
    }
    pub fn related(self, related: impl Into<Box<[(Box<str>, Span)]>>) -> Self {
        Self {
            related: [self.related, related.into()].concat().into(),
            ..self
        }
    }
}

impl Error for DukaSpannedError {}
impl Display for DukaSpannedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[DukaError] {} in {}", self.kind, self.span)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaIRError {
    pub kind: DukaIRErrorKind,
}
impl Error for DukaIRError {}
impl Display for DukaIRError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[DukaCodegenError] {}", self.kind)
    }
}
impl From<DukaIRErrorKind> for DukaIRError {
    fn from(value: DukaIRErrorKind) -> Self {
        Self { kind: value }
    }
}

impl From<&'static str> for DukaIRError {
    fn from(value: &'static str) -> Self {
        Self {
            kind: DukaIRErrorKind::Custom(value.into()),
        }
    }
}

#[derive(Debug, ThatError, Clone, PartialEq)]
pub enum DukaIRErrorKind {
    #[error("{}")]
    Custom(String),
    #[error("Trying to modify a readonly item: {}")]
    TryModifyReadonly(Box<str>),
    #[error("Trying to assign a constant: {}")]
    TryAssignConst(Box<str>),
    #[error("Got invalid syntax: {} is a variable with attribute <const>")]
    InvalidAST(Box<str>),
    #[error("Found unsolved goto: invalid label {}")]
    UnsolvedGoto(Box<str>),
    #[error("Found invalid control keyword out of any loop: {}")]
    OutOfLoop(Box<str>),
    #[error("Undefined variable: {}")]
    UndefinedVariable(Box<str>),
    #[error("Unsupported feature read: {}, try to use \"DukaAdapter\" to desugar it first")]
    UnsupportedFeature(Box<str>),
    #[error("Exprs used too many register: {} > {}")]
    TooManyRegisters { got: usize, limit: usize },
    #[error("Exprs used too many local variables: {} > {}")]
    TooManyLocals { got: usize, limit: usize },
    #[error("Got invalid address: {}")]
    InvalidAddress(usize),
    #[error("Invalid params for {}: expected {}, got {}")]
    InvalidParams(Box<str>, usize, usize),
    #[error(
        "Expr must be a constant expr, which can be a number, string, boolean or constant table"
    )]
    NotConstExpr,
    #[error("Found duplicated label: #{}")]
    DuplicatedLabel(usize),
}
