use std::{
    fmt::Display,
    ops::{Add, BitAnd, BitOr, Mul, Sub},
};

use duka_macros::{Info, binops};
use serde::Serialize;

use crate::{
    error::Span,
    token::{Token, TokenKind},
    types::{Spanned, Visit, VisitMut, Visitor, VisitorMut},
    value::ConstValue,
};

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize)]
#[ast(stmt)]
pub struct Stmt(pub StmtKind, #[nonvisiting] pub Span);
impl Mul<StmtKind> for Span {
    type Output = Stmt;
    fn mul(self, rhs: StmtKind) -> Self::Output {
        Stmt(rhs, self)
    }
}

#[derive(Debug, PartialEq, Default, Info, Clone, Visitor, VisitorMut, Serialize)]
pub enum StmtKind {
    #[default]
    #[tag(empty)]
    Empty,
    #[tag(empty)]
    Extern,

    Expr(Expr),
    Call(Expr, Vec<Expr>),

    Label(#[nonvisiting] String),
    Goto(#[nonvisiting] String),
    Break,
    Continue,
    Return(Vec<Expr>),

    #[tag(sugar)]
    Match(Match),
    #[tag(sugar)]
    Object(ObjectDef),

    If(If),
    /// var, start value, condition, step, body
    ForNumberic(Path, Expr, Expr, Option<Expr>, #[block(loop_stmt)] Block),
    ForGeneric(Vec<Path>, Vec<Expr>, #[block(loop_stmt)] Block),
    While(Expr, #[block(loop_stmt)] Block),
    /// ```lua
    /// do
    /// ...
    /// end
    /// ```
    Do(#[block(do_stmt)] Block),

    ///```lua
    /// var = 1
    /// ```
    Assign(Vec<Path>, Vec<Expr>),
    ///```lua
    /// local var = 1
    /// global var = 2
    /// ```
    Define(
        #[nonvisiting] Vec<AttrName>,
        Vec<Expr>,
        #[nonvisiting] bool, /* is global? */
    ),
    ///```lua
    /// [global] function a(b)
    /// ...
    /// end
    /// ```
    Function(Path, #[nonvisiting] Attrs, FuncBody, #[nonvisiting] bool),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub struct FuncBody(#[nonvisiting] pub Vec<Param>, #[block(func)] pub Block);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_vararg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
    }
}

#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize)]
pub struct If(pub IfClause, pub Vec<IfClause>, pub Option<Block>);
#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize)]
pub struct IfClause(#[block(if_clause)] pub Block, pub Box<Expr>);

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize)]
pub struct Block(pub Vec<Stmt>, pub Option<Box<Stmt>>);
impl Block {
    pub const EMPTY: Self = Self(vec![], None);
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty() && self.1.is_none()
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub struct Match(
    pub Box<Expr>,
    pub Vec<MatchClause>,
    #[block(match_else)] pub Option<Block>,
);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub struct MatchClause(pub Pattern, #[block(match_clause)] pub Block);

/// guard mode
pub type Pattern = (PatternTerm, Option<Expr>);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum PatternTerm {
    /// `123`
    Constant(Box<Expr>),
    /// `local name`
    Bind(#[nonvisiting] Name),
    /// `|> func()`
    Call(Box<Expr>),
    /// `> 2`
    Compare(#[nonvisiting] BinOp, Box<Expr>),
    /// `{ 1, ..., 5, _, _, a = local var, [true] = |> func }`
    Table(Vec<FieldPattern>),
    /// `> 2 and < 5`
    Compound(Box<PatternTerm>, Box<PatternTerm>, #[nonvisiting] PatternOp),
    /// `not ...`
    Not(Box<PatternTerm>),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum FieldPattern {
    Array(PatternArrayTerm),
    Named(#[nonvisiting] Name, PatternTerm),
    Expr(Expr, PatternTerm),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum PatternArrayTerm {
    /// `_ * n`
    Discard(#[nonvisiting] usize),
    /// `...`           
    DiscardMany,
    /// term       
    Term(PatternTerm),
}

#[derive(Debug, PartialEq, Clone, Serialize)]
pub enum PatternOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub struct ObjectDef {
    #[nonvisiting]
    name: Name,
    #[nonvisiting]
    base: Option<Name>,
    constructor: Option<FuncBody>,
    methods: Vec<(Name, Attrs, FuncBody)>,
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
/// (clauses)
/// select (expr)
pub struct Linq(pub Vec<LinqClause>, pub Box<Expr>);

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum LinqClause {
    /// where (expr) -> if ...
    Where(Box<Expr>),
    /// from (name) in (expr) -> for ... in ...
    From(#[nonvisiting] Name, Box<Expr>),
}

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize)]
#[ast(expr)]
pub struct Expr(pub ExprKind, #[nonvisiting] pub Span);
impl Mul<ExprKind> for Span {
    type Output = Expr;
    fn mul(self, rhs: ExprKind) -> Self::Output {
        Expr(rhs, self)
    }
}

macro_rules! compile_time_binary {
    ($opp: ident use $op: ident impl $func: ident) => {
        impl $op for Expr {
            type Output = Expr;
            fn $func(self, rhs: Self) -> Self::Output {
                let span = self.1 + rhs.1;
                Expr(
                    ExprKind::Binary(Box::new(self), Box::new(rhs), BinOp::$opp),
                    span,
                )
            }
        }
    };
}

compile_time_binary!(Add use Add impl add);
compile_time_binary!(Sub use Sub impl sub);
compile_time_binary!(And use BitAnd impl bitand);
compile_time_binary!(Or use BitOr impl bitor);

#[derive(Debug, PartialEq, Default, Info, Clone, Visitor, VisitorMut, Serialize)]
pub enum ExprKind {
    #[default]
    Empty,

    #[tag(sugar)]
    Linq(Linq),
    #[tag(sugar)]
    Match(Match),

    VarArg,
    Literal(#[nonvisiting] ConstValue),
    Do(#[block(do_expr)] Block),

    Access(Path),
    Call(Box<Expr>, Vec<Expr>),

    Table(Vec<Field>),
    Function(FuncBody),

    Unary(Box<Expr>, #[nonvisiting] UnOp),
    Binary(Box<Expr>, Box<Expr>, #[nonvisiting] BinOp),
    If(If),
}

impl ExprKind {
    #[inline]
    pub fn is_const(&self) -> bool {
        matches!(self, ExprKind::Literal(lit) if lit.is_const())
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum Field {
    Value(Expr),
    KeyValue(Expr, Expr),
    NameValue(#[nonvisiting] Name, Expr),
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
pub type Attrs = Vec<Attr>;
pub type Name = Spanned<String>;
pub type AttrName = Spanned<(Name, Attrs)>;

#[derive(Debug, PartialEq, Clone, Serialize)]
pub enum Param {
    Var(Span),
    Name(Name),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
pub enum PathSuffix {
    /// `path.name`
    Dot(#[nonvisiting] Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(#[nonvisiting] Name),
}
impl Display for PathSuffix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSuffix::Dot((name, _)) => write!(f, ".{name}"),
            PathSuffix::Index(_) => write!(f, "[(expr)]"),
            PathSuffix::Colon((name, _)) => write!(f, ":{name}"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize)]
/// これはチェインです
pub enum Path {
    /// `(expr)`
    Expr(Box<Expr>),
    /// `name`
    Base(#[nonvisiting] Name),
    Chain(Box<Path>, PathSuffix),
}
impl Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Path::Expr(_) => write!(f, "(expr)")?,
            Path::Base((name, _)) => write!(f, "{name}")?,
            Path::Chain(path, path_suffix) => write!(f, "{path}{path_suffix}")?,
        }
        Ok(())
    }
}
/// Only used in crate
impl Into<Path> for Token {
    /// ATTETION, this will panic, but I don't care
    fn into(self) -> Path {
        assert!(matches!(self.0, TokenKind::Ident(..)));
        match self.0 {
            TokenKind::Ident(name) => Path::Base((name, self.1)),
            _ => unimplemented!(),
        }
    }
}
impl Add<PathSuffix> for Path {
    type Output = Path;
    fn add(self, rhs: PathSuffix) -> Self::Output {
        Path::Chain(Box::new(self), rhs)
    }
}
#[derive(Debug, PartialEq, Info, Clone, Serialize)]
pub enum UnOp {
    Length,
    Not,
    BitNot,
    Minus,
}
#[derive(Debug, PartialEq, Info, Clone, Serialize)]
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

    BitAnd,
    BitOr,
    BitXor,
    ShiftL,
    ShiftR,

    Concat,
    Pipeline,
    PipelineL,
}

binops! {
    as get_binop_info
    type TokenKind -> BinOp = BinOpInfo:

    Or;

    And;

    Xor;

    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual;

    Pipeline,
    PipelineL right;

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

    这里是expression的op_优先级是递增的
}

binops! {
    as get_patop_info
    type TokenKind -> PatternOp = PatOpInfo:

    Or;

    And;

    Xor

    这里是pattern的op_也是递增的
}
