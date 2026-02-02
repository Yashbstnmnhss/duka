use std::{error::Error, fmt::Display, ops::Add};

use duka_macros::ThatError;

#[derive(
    Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Copy, serde::Serialize, serde::Deserialize,
)]
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
impl Add<(usize, usize)> for Position {
    type Output = Span;
    fn add(self, rhs: (usize, usize)) -> Self::Output {
        Span {
            start: self,
            end: Position {
                line: self.line + rhs.0,
                column: self.column + rhs.1,
            },
        }
    }
}

impl Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(Ln: {}, Col: {})", self.line, self.column)
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
    pub const fn offset(&self) -> (usize, usize) {
        let Position {
            line: l1,
            column: c1,
        } = self.start;
        let Position {
            line: l2,
            column: c2,
        } = self.end;
        (l2 - l1, c2 - c1)
    }
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
    #[error("[Macro Expander] {}")]
    Macro(DukaMacroError),
    #[error("[Parser] {}")]
    Parser(DukaParserError),
    #[error("[Analyzer] {}")]
    Semantic(DukaSemanticError),
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
    #[error("Unknown variable in splicer: {}")]
    UnknownTokensVariable(String),
    #[error("Unexpected end")]
    UnexpectedEnd,
    // wtf typo
    #[error("Unexpected token {}, expected {}")]
    UnexpectedToken(String, String),
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
pub enum DukaMacroError {
    #[error("Invalid macro body")]
    InvalidMacroBody,
    #[error("Invalid parameters count: expected {}")]
    InvalidInputParameters(usize),
    #[error("Failed to load built-in macro")]
    FailedLoadBuiltin,
    #[error("Unknown parameter defined: named {}")]
    UnknownParameterDefined(String),
    #[error("Reached max depth of macro expanding: happened in {}")]
    ReachMaxDepth(String),
    #[error("Unknown macro: named {}")]
    UnknownMacro(String),
    #[error("Unknown built-in macro: named {}")]
    UnknownBuiltinMacro(String),
    #[error("Unexpected token in macro: {}")]
    UnexpectedToken(String),
}
impl Into<DukaErrorKind> for DukaMacroError {
    fn into(self) -> DukaErrorKind {
        DukaErrorKind::Macro(self)
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
pub struct DukaSpannedError {
    pub kind: DukaErrorKind,
    pub span: Span,
}
impl Error for DukaSpannedError {}
impl Display for DukaSpannedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[DukaError] {} in {}", self.kind, self.span)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaCodegenError {
    pub kind: DukaCodegenErrorKind,
}
impl Error for DukaCodegenError {}
impl Display for DukaCodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[DukaCodegenError] {}", self.kind)
    }
}
impl From<DukaCodegenErrorKind> for DukaCodegenError {
    fn from(value: DukaCodegenErrorKind) -> Self {
        Self { kind: value }
    }
}

impl From<&'static str> for DukaCodegenError {
    fn from(value: &'static str) -> Self {
        Self {
            kind: DukaCodegenErrorKind::Custom(value),
        }
    }
}

#[derive(Debug, ThatError, Clone, PartialEq)]
pub enum DukaCodegenErrorKind {
    #[error("{}")]
    Custom(&'static str),
    #[error("Trying to assign a constant: {}")]
    TryAssignConst(String),
    #[error("Got invalid syntax: {} is a variable with attribute <const>")]
    InvalidAST(String),
    #[error("Found unsolved goto: invalid label {}")]
    UnsolvedGoto(String),
    #[error("Undefined variable: {}")]
    UndefinedVariable(String),
    #[error("Unsupported feature read: {}, try to use \"DukaAdapter\" to desugar it first")]
    UnsupportedFeature(String),
    #[error("Exprs used too many register: {}")]
    TooManyRegister(usize),
    #[error(
        "Expr must be a constant expr, which can be a number, string, boolean or constant table"
    )]
    NotConstExpr,
}
