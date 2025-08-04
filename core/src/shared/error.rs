use std::{fmt::Display, ops::Add};

use duka_macros::ThatError;

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Copy)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}
pub const START_LINE: usize = 1;
pub const START_COLUMN: usize = 1;

impl Default for Position {
    fn default() -> Self {
        Self::START
    }
}
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
impl Add<usize> for Position {
    type Output = Span;
    fn add(self, rhs: usize) -> Self::Output {
        Span {
            start: self,
            end: Position {
                line: self.line,
                column: self.column + rhs,
            },
        }
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(Ln: {}, Col: {})", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq, Copy, Default)]
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
    #[error("[Semantic] {}")]
    Semantic(DukaSemanticError),
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
pub enum DukaSemanticError {
    #[error("Cannot use 'break' or 'continue' outside of a loop")]
    InvalidLoopFlowControl,
    #[error("Duplicated {} found: '{}' ")]
    DuplicatedItem(String, String),
    #[error("Invisible label '{}' for goto")]
    InvisibleGotoLabel(String),
    #[error("Cannot use vararg here")]
    InvalidVarArg,
}

impl Into<DukaErrorKind> for DukaSemanticError {
    fn into(self) -> DukaErrorKind {
        DukaErrorKind::Semantic(self)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaParserError {
    // wtf typo
    #[error("Unexpected token, expected {}")]
    UnexpectedToken(String),
    #[error("Duplicated name used: {}")]
    DuplicatedName(String),
    #[error("Found unknown operator: {}")]
    UnknownOperator(String),
    #[error("Invalid operator used: {}")]
    InvalidOperator(String),
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
    InvalidInteger(String),
    #[error("Invalid float format: {}")]
    InvalidFloat(String),
    #[error("Unfinshed string, {}")]
    UnfinishedString(String),
    #[error("Invalid escaped format: {}")]
    InvalidEscaped(String),
    #[error("Invalid unicode escaped: {}")]
    InvalidUnicodeEscaped(String),
    #[error("Invalid escaped format, expected {}")]
    UnexpectedEnd(String),
    #[error("Unexpected character: {}")]
    UnexpectedCharacter(char),
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
