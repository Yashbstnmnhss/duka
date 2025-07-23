use std::ops::Add;

use duka_macros::Info;

use crate::{
    frontend::token::{Token, TokenKind},
    shared::{error::Span, types::Spanned, value::Value},
};

pub trait Visitor<T> {
    fn visit_expr(&mut self, expr: &ExprKind) -> T;
    fn visit_stmt(&mut self, stmt: &StmtKind) -> T;
}

pub type Stmt = Spanned<StmtKind>;

#[derive(Debug, PartialEq)]
pub enum StmtKind {
    Empty,

    Expr(ExprKind),

    Call(Expr, Vec<Expr>),

    Label(String),
    Goto(String),
    Break,
    Continue,
    Return(Vec<ExprKind>),

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

#[derive(Debug, PartialEq)]
pub struct IfClause(pub Block, pub ExprKind);

#[derive(Debug, PartialEq)]
pub struct Block(pub Vec<Stmt>, pub Option<Box<Stmt>>);

pub type Expr = Spanned<ExprKind>;

#[derive(Debug, PartialEq)]
pub enum ExprKind {
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
}

macro_rules! binfo {
    ($op: ident, $n: literal, right) => {
        (BinOp::$op, ($n + 1, $n))
    };
    ($op: ident, $n: literal) => {
        (BinOp::$op, ($n, $n))
    };
}

pub type BinOpInfo = (BinOp, (u8, u8));

#[inline]
pub fn get_binop_info(tk: &TokenKind) -> Option<BinOpInfo> {
    if !tk.is_binop() {
        return None;
    }

    Some(match tk {
        TokenKind::Or => binfo!(Or, 1),
        TokenKind::And => binfo!(And, 2),

        TokenKind::Equal => binfo!(Equal, 3),
        TokenKind::NotEqual => binfo!(NotEqual, 3),
        TokenKind::Greater => binfo!(Greater, 3),
        TokenKind::GreaterEqual => binfo!(GreaterEqual, 3),
        TokenKind::Less => binfo!(Less, 3),
        TokenKind::LessEqual => binfo!(LessEqual, 3),

        TokenKind::BitOr => binfo!(BitOr, 4),
        TokenKind::BitTilde => binfo!(BitXor, 5),
        TokenKind::BitAnd => binfo!(BitAnd, 6),

        TokenKind::ShiftL => binfo!(ShiftL, 7),
        TokenKind::ShiftR => binfo!(ShiftR, 7),

        TokenKind::Concat => binfo!(Concat, 8, right),

        TokenKind::Plus => binfo!(Add, 10),
        TokenKind::Minus => binfo!(Sub, 10),

        TokenKind::Multiply => binfo!(Multiply, 11),
        TokenKind::Mod => binfo!(Mod, 11),
        TokenKind::Divide => binfo!(Divide, 11),
        TokenKind::IDivide => binfo!(IDivide, 11),

        TokenKind::Pow => binfo!(Pow, 13, right),
        _ => unreachable!(),
    })
}
