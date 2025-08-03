use std::ops::Add;

use duka_macros::{Info, binops};

use crate::{
    frontend::token::{Token, TokenKind},
    shared::{error::Span, types::Spanned, value::Value},
};

pub type Stmt = Spanned<StmtKind>;

#[derive(Debug, PartialEq, Default)]
pub enum StmtKind {
    #[default]
    Empty,

    Logic(LogicDef),

    Expr(Expr),
    Call(Expr, Vec<Expr>),

    Label(String),
    Goto(String),
    Break,
    Continue,
    Return(Vec<Expr>),

    If(IfClause, Vec<IfClause>, Option<Block>),
    /// var, start value, condition, step, body
    ForNumberic(Path, Expr, Expr, Option<Expr>, Block),
    ForGeneric(Vec<Path>, Vec<Expr>, Block),
    While(Expr, Block),
    /// ```lua
    /// do
    /// ...
    /// end
    /// ```
    Do(Block),

    ///```lua
    /// var = 1
    /// ```
    Assign(Vec<Path>, Vec<Expr>),
    ///```lua
    /// local var = 1
    /// ```
    Local(Vec<AttrName>, Vec<Expr>),
    ///```lua
    /// [local] function a(b)
    /// ...
    /// end
    /// ```
    Function(Path, FuncBody, bool),
}

#[derive(Debug, PartialEq)]
pub struct FuncBody(pub Vec<Param>, pub Block);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_vararg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
    }
}

#[derive(Debug, PartialEq)]
pub struct IfClause(pub Block, pub Expr);

#[derive(Debug, PartialEq)]
pub struct Block(pub Vec<Stmt>, pub Option<Box<Stmt>>);
impl Block {
    pub const EMPTY: Self = Self(vec![], None);
}

pub type Expr = Spanned<ExprKind>;

#[derive(Debug, PartialEq, Default)]
pub enum ExprKind {
    #[default]
    Empty,

    LogicQuery(),

    VarArg,
    Literal(Value),

    Access(Path),
    Call(Box<Expr>, Vec<Expr>),

    Table(Vec<Field>),
    Function(FuncBody),

    Unary(Box<Expr>, UnOp),
    Binary(Box<Expr>, Box<Expr>, BinOp),
}

impl ExprKind {
    #[inline]
    pub fn is_const(&self) -> bool {
        matches!(self, ExprKind::Literal(lit) if lit.is_const())
    }
}

#[derive(Debug, PartialEq)]
pub enum Field {
    Value(Expr),
    KeyValue(Expr, Expr),
    NameValue(Name, Expr),
}

impl Field {
    #[inline]
    pub fn is_const(&self) -> bool {
        match self {
            Self::Value(e) => e.0.is_const(),
            Self::KeyValue(k, v) => k.0.is_const() && v.0.is_const(),
            Self::NameValue(_, v) => v.0.is_const(),
        }
    }
}

pub type Attr = Spanned<String>;
pub type Name = Spanned<String>;
pub type AttrName = Spanned<(Name, Option<Attr>)>;

#[derive(Debug, PartialEq)]
pub enum Param {
    Var(Span),
    Name(Name),
}

#[derive(Debug, PartialEq)]
pub enum PathSuffix {
    /// `path.name`
    Dot(Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(Name),
}

#[derive(Debug, PartialEq)]
/// kore wa chain desu
pub enum Path {
    /// `(expr)`
    Expr(Box<Expr>),
    /// `name`
    Base(Name),
    Chain(Box<Path>, PathSuffix),
}

impl Into<Path> for Token {
    fn into(self) -> Path {
        match self.0 {
            TokenKind::Ident(name) => Path::Base((name, self.1)),
            _ => panic!("only support ident"),
        }
    }
}
impl Add<PathSuffix> for Path {
    type Output = Path;
    fn add(self, rhs: PathSuffix) -> Self::Output {
        Path::Chain(Box::new(self), rhs)
    }
}

#[derive(Debug, PartialEq, Info)]
pub enum UnOp {
    Length,
    Not,
    BitNot,
    Minus,
}
#[derive(Debug, PartialEq, Info)]
pub enum BinOp {
    Add,
    Sub,
    Multiply,
    Divide,
    IDivide,
    Mod,
    Pow,

    And,
    Or,
    Xor,

    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,

    BitAnd,
    BitOr,
    BitXor,
    ShiftL,
    ShiftR,

    Concat,
    Pipeline,
}

// macro_rules! binfo {
//     ($op: ident, $n: literal, right) => {
//         (BinOp::$op, ($n + 1, $n))
//     };
//     ($op: ident, $n: literal) => {
//         (BinOp::$op, ($n, $n))
//     };
// }

pub type BinOpInfo = (BinOp, (u8, u8));

binops! {
    as get_binop_info
    type TokenKind -> BinOp = BinOpInfo:

    Or;

    And;

    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual;

    Pipeline;

    BitOr,
    BitTilde => BitXor,
    BitAnd;

    ShiftL,
    ShiftR;

    Concat right;

    Plus => Add,
    Minus => Sub;

    Multiply,
    Divide,
    IDivide,
    Mod;

    Pow right

    递增
}

#[derive(Debug, PartialEq)]
pub struct LogicDef {
    pub facts: Vec<i32>,
    pub rules: Vec<i32>,
}
