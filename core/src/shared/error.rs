use std::{
    fmt::Display,
    num::{ParseFloatError, ParseIntError},
    ops::Add,
};

use duka_macros::ThatError;

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Copy)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}
pub const START_LINE: usize = 1;
pub const START_COLUMN: usize = 1;

impl Position {
    pub const START: Self = Self {
        line: START_LINE,
        column: START_COLUMN,
    };

    pub fn is_start(&self) -> bool {
        *self == Position::START
    }
    pub fn new_line(&mut self) {
        self.line += 1;
        self.column = START_COLUMN;
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(Ln: {}, Col: {})", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
/** 左闭右开 */
pub struct Span {
    pub start: Position,
    pub end: Position,
}
impl Span {
    pub const EMPTY: Span = Span {
        start: Position::START,
        end: Position::START,
    };
}
impl Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} to {}", self.start, self.end)
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
    #[error("[Parser] {}")]
    Parser(DukaParserError),
    #[error("[Runtime] {}")]
    Runtime(DukaRuntimeError),
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    None,
}

impl Into<DukaErrorKind> for DukaRuntimeError {
    fn into(self) -> DukaErrorKind {
        DukaErrorKind::Runtime(self)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaParserError {
    // wtf typo
    #[error("Unexpected token, expecting {}")]
    UnexpectedToken(String),
    #[error("Duplicated name used: {}")]
    DuplicatedName(String),
    #[error("Found unknown operator: {}")]
    UnknownOperator(String),
}
impl Into<DukaErrorKind> for DukaParserError {
    fn into(self) -> DukaErrorKind {
        DukaErrorKind::Parser(self)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaLexerError {
    #[error("Reader error: {}")]
    ReaderError(String),
    #[error("Invalid integer format: {}")]
    InvalidInteger(ParseIntError),
    #[error("Invalid float format: {}")]
    InvalidFloat(ParseFloatError),
    #[error("Unfinshed string, {}")]
    UnfinishedString(String),
    #[error("Invalid escaped format: {}")]
    InvalidEscaped(String),
    #[error("Invalid unicode escaped: {}")]
    InvalidUnicodeEscaped(String),
    #[error("Invalid escaped format, expecting {}")]
    UnexpectedEnd(String),
    #[error("Invalid escaped format")]
    UnexpectedCharacter,
    #[error("Multiple line comment aren't finished, {}")]
    UnfinishedComment(String),
    #[error("Unknown character has been read: {}")]
    UnknownCharacter(String),
    #[error("Invalid input: Input is not valid utf8")]
    InvalidUtf8,
}

impl Into<DukaErrorKind> for DukaLexerError {
    fn into(self) -> DukaErrorKind {
        DukaErrorKind::Lexer(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaError {
    pub kind: DukaErrorKind,
    pub span: Span,
}
impl Display for DukaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[DukaError] {} in {}", self.kind, self.span)
    }
}
