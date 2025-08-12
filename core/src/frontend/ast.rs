use std::ops::Add;

use duka_macros::{Info, binops};

use crate::{
    frontend::token::{Token, TokenKind},
    shared::{error::Span, types::Spanned, value::Value},
};

pub type Stmt = Spanned<StmtKind>;

#[derive(Debug, PartialEq, Default, Info, Clone)]
pub enum StmtKind {
    #[default]
    Empty,

    Expr(Expr),
    Call(Expr, Vec<Expr>),

    Label(String),
    Goto(String),
    Break,
    Continue,
    Return(Vec<Expr>),

    #[tag(sugar)]
    Match(Match),
    #[tag(sugar)]
    Object(ObjectDef),

    If(If),
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
    /// global var = 2
    /// ```
    Define(Vec<AttrName>, Vec<Expr>, bool),
    ///```lua
    /// [global] function a(b)
    /// ...
    /// end
    /// ```
    Function(Path, Attrs, FuncBody, bool),
}

#[derive(Debug, PartialEq, Clone)]
pub struct FuncBody(pub Vec<Param>, pub Block);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_vararg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct If(pub IfClause, pub Vec<IfClause>, pub Option<Block>);
#[derive(Debug, PartialEq, Clone)]
pub struct IfClause(pub Block, pub Box<Expr>);

#[derive(Debug, PartialEq, Default, Clone)]
pub struct Block(pub Vec<Stmt>, pub Option<Box<Stmt>>);
impl Block {
    pub const EMPTY: Self = Self(vec![], None);
}

#[derive(Debug, PartialEq, Clone)]
pub struct Match(pub Box<Expr>, pub Vec<MatchClause>, pub Option<Block>);
#[derive(Debug, PartialEq, Clone)]
pub struct MatchClause(pub Pattern, pub Block);

/// guard mode
pub type Pattern = (PatternTerm, Option<Expr>);
#[derive(Debug, PartialEq, Clone)]
pub enum PatternTerm {
    /// `123`
    Constant(Box<Expr>),
    /// `local name`
    Bind(Name),
    /// `|> func()`
    Call(Box<Expr>),
    /// `> 2`
    Compare(BinOp, Box<Expr>),
    /// `{ 1, ..., 5, _, _, a = local var, [true] = |> func }`
    Table(Vec<FieldPattern>),
    /// `> 2 and < 5`
    Compound(Box<PatternTerm>, Box<PatternTerm>, PatternOp),
    /// `not ...`
    Not(Box<PatternTerm>),
}

#[derive(Debug, PartialEq, Clone)]
pub enum FieldPattern {
    Array(PatternArrayTerm),
    Named(Name, PatternTerm),
    Expr(Expr, PatternTerm),
}

#[derive(Debug, PartialEq, Clone)]
pub enum PatternArrayTerm {
    /// `_ * n`
    Discard(usize),
    /// `...`           
    DiscardMany,
    /// term       
    Term(PatternTerm),
}

#[derive(Debug, PartialEq, Clone)]
pub enum PatternOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, PartialEq, Clone)]
pub struct ObjectDef {
    name: Name,
    base: Option<Name>,
    constructor: Option<FuncBody>,
    methods: Vec<(Name, Attrs, FuncBody)>,
}

pub type Expr = Spanned<ExprKind>;

#[derive(Debug, PartialEq, Default, Info, Clone)]
pub enum ExprKind {
    #[default]
    Empty,

    #[tag(sugar)]
    Linq(),
    #[tag(sugar)]
    Match(Match),

    VarArg,
    Literal(Value),
    Block(Block),

    Access(Path),
    Call(Box<Expr>, Vec<Expr>),

    Table(Vec<Field>),
    Function(FuncBody),

    Unary(Box<Expr>, UnOp),
    Binary(Box<Expr>, Box<Expr>, BinOp),
    If(If),
}

impl ExprKind {
    #[inline]
    pub fn is_const(&self) -> bool {
        matches!(self, ExprKind::Literal(lit) if lit.is_const())
    }
}

#[derive(Debug, PartialEq, Clone)]
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
pub type Attrs = Vec<Attr>;
pub type Name = Spanned<String>;
pub type AttrName = Spanned<(Name, Attrs)>;

#[derive(Debug, PartialEq, Clone)]
pub enum Param {
    Var(Span),
    Name(Name),
}

#[derive(Debug, PartialEq, Clone)]
pub enum PathSuffix {
    /// `path.name`
    Dot(Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(Name),
}

#[derive(Debug, PartialEq, Clone)]
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

#[derive(Debug, PartialEq, Info, Clone)]
pub enum UnOp {
    Length,
    Not,
    BitNot,
    Minus,
}
#[derive(Debug, PartialEq, Info, Clone)]
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

// macro_rules! binfo {
//     ($op: ident, $n: literal, right) => {
//         (BinOp::$op, ($n + 1, $n))
//     };
//     ($op: ident, $n: literal) => {
//         (BinOp::$op, ($n, $n))
//     };
// }

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
