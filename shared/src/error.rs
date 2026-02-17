use std::{error::Error, fmt::Display, ops::Add};

use duka_macros::ThatError;

#[derive(
    Debug, Clone, PartialEq, PartialOrd, Eq, Ord, Copy, serde::Serialize, serde::Deserialize,
)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}
pub const START_LINE: u32 = 1;
pub const START_COLUMN: u32 = 1;

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
                column: self.column + rhs as u32,
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
                line: self.line + rhs.0 as u32,
                column: self.column + rhs.1 as u32,
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
    Incomplete(String),
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

impl From<DukaSemanticError> for DukaErrorKind {
    fn from(value: DukaSemanticError) -> Self {
        DukaErrorKind::Semantic(value)
    }
}

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaParserError {
    #[error("Unknown variable in splicer: {}")]
    UnknownTokensVariable(String),
    #[error("Unexpected end")]
    UnexpectedEnd,
    #[error("Found unknown bang keyword: {}")]
    UnknownBang(String),
    #[error("Unexpected token {}, expected {}")]
    UnexpectedToken { got: String, expected: String },
    #[error("Duplicated name used: {}")]
    DuplicatedName(String),
    #[error("Found unknown operator: {}")]
    UnknownOperator(String),
    #[error("Invalid operator used: {}")]
    InvalidOperator(String),
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
impl From<DukaMacroError> for DukaErrorKind {
    fn from(value: DukaMacroError) -> Self {
        DukaErrorKind::Macro(value)
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
    #[error("Unfinished string, {}")]
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

impl From<DukaLexerError> for DukaErrorKind {
    fn from(value: DukaLexerError) -> Self {
        DukaErrorKind::Lexer(value)
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
            kind: DukaIRErrorKind::Custom(value),
        }
    }
}

#[derive(Debug, ThatError, Clone, PartialEq)]
pub enum DukaIRErrorKind {
    #[error("{}")]
    Custom(&'static str),
    #[error("Trying to modify a readonly item: {}")]
    TryModifyReadonly(String),
    #[error("Trying to assign a constant: {}")]
    TryAssignConst(String),
    #[error("Got invalid syntax: {} is a variable with attribute <const>")]
    InvalidAST(String),
    #[error("Found unsolved goto: invalid label {}")]
    UnsolvedGoto(String),
    #[error("Found invalid control keyword out of any loop: {}")]
    OutOfLoop(String),
    #[error("Undefined variable: {}")]
    UndefinedVariable(String),
    #[error("Unsupported feature read: {}, try to use \"DukaAdapter\" to desugar it first")]
    UnsupportedFeature(String),
    #[error("Exprs used too many register: {} > {}")]
    TooManyRegisters { got: usize, limit: usize },
    #[error("Exprs used too many local variables: {} > {}")]
    TooManyLocals { got: usize, limit: usize },
    #[error("Got invalid address: {}")]
    InvalidAddress(usize),
    #[error("Invalid params for {}: expected {}, got {}")]
    InvalidParams(String, usize, usize),
    #[error(
        "Expr must be a constant expr, which can be a number, string, boolean or constant table"
    )]
    NotConstExpr,
}
