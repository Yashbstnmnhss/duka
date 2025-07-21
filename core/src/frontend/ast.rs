use crate::{
    frontend::token::{Token, TokenKind},
    shared::{types::Spanned, value::Value},
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
    Call(ExprKind),

    Label(String),
    Break,
    Continue,
    Return(Vec<ExprKind>),

    If(IfClause, Vec<IfClause>, Option<Block>),
    /// var, start value, condition, step, body
    ForNumberic(Path, Expr, Expr, Option<Expr>, Block),
    ForGeneric(Vec<Path>, Expr, Block),
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
    Function(Path, Vec<Path>, Block, bool),
}

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

    Unary(Box<Expr>, UnOp),
    Binary(Box<Expr>, Box<Expr>, BinOp),
}

#[derive(Debug, PartialEq)]
pub enum UnOp {
    Length,
    Not,
    BitNot,
    Minus,
}
#[derive(Debug, PartialEq)]
pub enum BinOp {
    Add,
    Minus,
    Multiply,
    Divide,
    IDivide,
    Mod,
    Pow,

    And,
    Or,
    Xor,

    BitAnd,
    BitOr,
    BitXor,
    ShiftL,
    ShiftR,

    Concat,
}

pub type Attr = Spanned<String>;
pub type Name = Spanned<String>;
pub type AttrName = Spanned<(Name, Option<Attr>)>;

#[derive(Debug, PartialEq)]
pub enum Path {
    /// `name`
    Simple(Name),
    /// `path.name`
    Dot(Box<Path>, Name),
    /// `path[expr]`
    Index(Box<Path>, Box<Expr>),
    /// `path:name`
    Colon(Box<Path>, Name),
}

impl Into<Path> for Token {
    fn into(self) -> Path {
        match self.0 {
            TokenKind::Ident(name) => Path::Simple((name, self.1)),
            _ => panic!("only support ident"),
        }
    }
}
