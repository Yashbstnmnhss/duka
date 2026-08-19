use std::{
    fmt::Display,
    ops::{Add, BitAnd, BitOr, Div, Mul, Sub},
};

use duka_macros::{Info, Visitor, VisitorMut, binops};
use serde::{Deserialize, Serialize};

use crate::analyzer::{Visit, VisitMut, Visitor, VisitorMut};
use crate::lexer::token::{Token, TokenKind};
use duka_shared::{
    constants::ccallish,
    dtype::Type,
    errors::Span,
    types::{BinOp, LogicDatabase, LogicOp, SourceInfo, Spanned, SysCall, UnOp},
    value::ConstValue,
};

#[derive(Debug, PartialEq, Clone)]
pub enum ExprOrStmt {
    Expr(Expr),
    Stmt(Stmt),
}
impl ExprOrStmt {
    pub fn get_span(&self) -> Span {
        match self {
            Self::Expr(Expr(_, sp)) => *sp,
            Self::Stmt(Stmt(_, sp)) => *sp,
        }
    }
    pub fn into_block(self) -> Block {
        match self {
            Self::Expr(Expr(ek, sp)) => Block(
                [].into(),
                Some(Box::new(Stmt(StmtKind::Return([Expr(ek, sp)].into()), sp))),
            ),
            Self::Stmt(s) => Block([s].into(), None),
        }
    }
}

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
#[ast(stmt)]
pub struct Stmt(pub StmtKind, #[nonvisiting] pub Span);
impl Mul<StmtKind> for Span {
    type Output = Stmt;
    fn mul(self, rhs: StmtKind) -> Self::Output {
        Stmt(rhs, self)
    }
}

#[derive(Debug, PartialEq, Default, Info, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum StmtKind {
    #[default]
    #[tag(empty)]
    Empty,
    #[tag(empty)]
    Extern,

    Expr(Box<Expr>),
    Call(Box<Expr>, Box<[Expr]>),

    Label(#[nonvisiting] String),
    Goto(#[nonvisiting] String),
    Break,
    Continue,
    Return(Box<[Expr]>),

    #[tag(sugar)]
    Match(Match),
    #[tag(sugar)]
    Object(Box<ObjectDef>),
    #[tag(sugar)]
    Export(Box<Stmt>),

    If(If),
    /// var, start value, condition, step, body
    ForNumeric(
        Path,
        Box<Expr>,
        Box<Expr>,
        Option<Box<Expr>>,
        #[block(loop_stmt)] Box<Block>,
    ),
    ForGeneric(Box<[Path]>, Box<[Expr]>, #[block(loop_stmt)] Box<Block>),
    While(Box<Expr>, #[block(loop_stmt)] Box<Block>),
    /// ```lua
    /// do
    /// ...
    /// end
    /// ```
    Do(#[block(do_stmt)] Box<Block>),

    ///```lua
    /// var = 1
    /// ```
    Assign(Box<[Path]>, Box<[Expr]>),
    ///```lua
    /// local var = 1
    /// global var = 2
    /// ```
    Define(
        #[nonvisiting] Box<[AttrName]>,
        Box<[Expr]>,
        #[nonvisiting] bool, /* is global? */
    ),
    ///```lua
    /// [global] function a(b)
    /// ...
    /// end
    /// ```
    Function(
        Path,
        #[nonvisiting] Attrs,
        Box<FuncBody>,
        #[nonvisiting] bool,
    ),
    #[tag(typesys)]
    ///```ts
    /// type Alias = int | string
    /// ```
    TypeAlias(#[nonvisiting] Name, #[nonvisiting] Box<Type>),
    #[tag(typesys)]
    ///```ts
    /// type function F(a, b) -> type
    ///     return a | b
    /// end
    /// ```
    TypeFunction(#[nonvisiting] Name, #[nonvisiting] Box<FuncBody>),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct FuncBody(
    #[nonvisiting] pub Box<[Param]>,
    #[nonvisiting] pub Box<[TypeParam]>,
    #[nonvisiting] pub Option<Type>, // Return Type
    #[block(func)] pub Box<Block>,
);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_var_arg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
    }
}

#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct If(pub IfClause, pub Box<[IfClause]>, pub Option<Box<Block>>);
#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct IfClause(#[block(if_clause)] pub Box<Block>, pub Box<Expr>);

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct Block(pub Box<[Stmt]>, pub Option<Box<Stmt>>);
impl Block {
    pub fn empty() -> Self {
        Self(Box::new([]), None)
    }
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty() && self.1.is_none()
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct Match(
    pub Box<Expr>,
    pub Box<[MatchClause]>,
    #[block(match_else)] pub Option<Box<Block>>,
);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct MatchClause(pub Pattern, #[block(match_clause)] pub Box<Block>);

/// guard mode
pub type Pattern = (PatternTerm, Option<Box<Expr>>);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternTerm {
    /// `123`
    Constant(Box<Expr>),
    /// `local name: type`
    Bind(#[nonvisiting] Name, #[nonvisiting] Option<Type>),
    /// `|> func()`
    Call(Box<Expr>),
    /// `> 2`
    Compare(#[nonvisiting] BinOp, Box<Expr>),
    /// `{ 1, ..., 5, _, _, a = local var, [true] = |> func }`
    /// also for array `[ 1, ..., 5, [expr] = 1]`
    Table(Box<[FieldPattern]>),
    /// `> 2 and < 5`
    Compound(Box<PatternTerm>, Box<PatternTerm>, #[nonvisiting] PatternOp),
    /// `not ...`
    Not(Box<PatternTerm>),
    /// `Array(inner)`, `Table(k, v)` (type-context only)
    Type(#[nonvisiting] Name, Box<[PatternTerm]>),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum FieldPattern {
    Array(PatternArrayTerm),
    Named(#[nonvisiting] Name, PatternTerm),
    Expr(Expr, PatternTerm),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternArrayTerm {
    /// `_ * n`
    Discard(#[nonvisiting] usize),
    /// `...`           
    DiscardMany,
    /// term       
    Term(PatternTerm),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum PatternOp {
    And,
    Or,
    Xor,
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct ObjectDef {
    #[nonvisiting]
    pub global: bool,
    #[nonvisiting]
    pub attrs: Attrs,
    #[nonvisiting]
    pub name: Name,
    #[nonvisiting]
    pub base: Option<Name>,
    #[nonvisiting]
    pub type_params: Box<[TypeParam]>,
    pub properties: Box<[ObjectProperty]>,
    pub static_methods: Box<[(Name, Attrs, FuncBody)]>,
    pub methods: Box<[(Name, Attrs, FuncBody)]>,
}
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum ObjectProperty {
    NameValue(
        #[nonvisiting] Name,
        Option<Box<Expr>>,
        #[nonvisiting] Option<Type>,
    ),
    KeyValue(Box<Expr>, Option<Box<Expr>>, #[nonvisiting] Option<Type>),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
/// (clauses)
/// select (expr)
pub struct Linq(pub Box<[LinqClause]>, pub Box<Expr>);

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum LinqClause {
    /// where (expr) -> if ...
    Where(Box<Expr>),
    /// from (name) in (expr) -> for ... in ...
    From(#[nonvisiting] Name, Box<Expr>),
}

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
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
compile_time_binary!(Multiply use Mul impl mul);
compile_time_binary!(Divide use Div impl div);
compile_time_binary!(And use BitAnd impl bitand);
compile_time_binary!(Or use BitOr impl bitor);

#[derive(Debug, PartialEq, Default, Info, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum ExprKind {
    #[default]
    Empty,

    #[tag(sugar)]
    Linq(Linq),
    #[tag(sugar)]
    Match(Match),

    VarArg,
    Literal(#[nonvisiting] ConstValue),
    Do(#[block(do_expr)] Box<Block>),

    Access(Box<Path>),
    Call(Box<Expr>, Box<[Expr]>),

    SysCall(#[nonvisiting] SysCall),

    Table(Box<[Field]>),
    Array(Box<[Expr]>),
    Function(FuncBody),

    Unary(Box<Expr>, #[nonvisiting] UnOp),
    Binary(Box<Expr>, Box<Expr>, #[nonvisiting] BinOp),
    If(Box<If>),
    #[tag(typesys)]
    TypeLit(#[nonvisiting] Type),
}

impl ExprKind {
    #[inline]
    pub fn is_const(&self) -> bool {
        matches!(self, ExprKind::Literal(lit) if lit.is_const())
    }
    #[inline]
    pub const fn is_self_call(&self) -> bool {
        matches!(self, ExprKind::Access(path) if path.is_self_call())
    }
    #[inline]
    pub fn is_callable_keyword(&self) -> Option<&'static str> {
        if let ExprKind::Access(path) = self {
            path.is_callable_keyword()
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
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
            // Self::Expand => false,
        }
    }
}

pub type Attr = (Spanned<String>, Box<[(Name, ConstValue)]>);
pub type Attrs = Box<[Attr]>;
pub type Name = Spanned<String>;
/// 可选的类型注时节存放在`.2`
pub type AttrName = Spanned<(Name, Attrs, Option<Type>)>;

pub fn get_attr(attrs: &Attrs, who: &str) -> Option<Box<[(Name, ConstValue)]>> {
    attrs
        .iter()
        .find_map(|i| (i.0.0 == who).then_some(i.1.clone()))
}
pub fn has_attr(attrs: &Attrs, who: &str) -> bool {
    attrs.iter().any(|(n, _)| n.0 == who)
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TypeParam(pub Name, pub Option<Type>);

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Param {
    Var(Span),
    Name(Name),
    /// 带类型标注的参数
    Typed(Name, Type),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PathSuffix {
    /// `path.name`
    Dot(#[nonvisiting] Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(#[nonvisiting] Name),
    /// `path.<T1, T2>` 值层泛型实例化, 编译期擦除 -> See docs/type.md
    TypeArgs(#[nonvisiting] Box<[Type]>, #[nonvisiting] Span),
}
impl PathSuffix {
    pub fn get_span(&self) -> Span {
        match self {
            PathSuffix::Dot(n) => n.1,
            PathSuffix::Index(expr) => (**expr).1,
            PathSuffix::Colon(n) => n.1,
            PathSuffix::TypeArgs(_, span) => *span,
        }
    }
}
impl Display for PathSuffix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathSuffix::Dot((name, _)) => write!(f, ".{name}"),
            PathSuffix::Index(_) => write!(f, "[(expr)]"),
            PathSuffix::Colon((name, _)) => write!(f, ":{name}"),
            PathSuffix::TypeArgs(..) => write!(f, ".<...>"),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
/// これはチェインです
pub enum Path {
    /// `(expr)`
    Expr(Box<Expr>),
    /// `name`
    Base(#[nonvisiting] Name),
    Chain(Box<Path>, PathSuffix),
}
impl Path {
    pub fn get_span(&self) -> Span {
        match self {
            Self::Expr(e) => (**e).1,
            Self::Base(n) => n.1,
            Self::Chain(p, s) => p.get_span() + s.get_span(),
        }
    }
    #[inline]
    pub const fn is_self_call(&self) -> bool {
        matches!(self, Path::Chain(_, PathSuffix::Colon(..)))
    }
    #[inline]
    pub fn is_callable_keyword(&self) -> Option<&'static str> {
        if let Path::Base((name, _)) = self {
            ccallish::CALLISHES
                .iter()
                .find_map(|n| (name == n).then_some(*n))
        } else {
            None
        }
    }
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
impl From<Token> for Path {
    /// ATTENTION, this will panic, but I don't care
    fn from(value: Token) -> Self {
        assert!(matches!(value.0, TokenKind::Ident(..)));
        match value.0 {
            TokenKind::Ident(name) => Path::Base((name, value.1)),
            _ => unreachable!(),
        }
    }
}
impl Add<PathSuffix> for Path {
    type Output = Path;
    fn add(self, rhs: PathSuffix) -> Self::Output {
        Path::Chain(Box::new(self), rhs)
    }
}

#[derive(Debug, Clone)]
pub enum TypeOp {
    Union,
    Intersect,
}

binops! {
    as get_typeop_info
    type TokenKind -> TypeOp = TypeOpInfo:

    BitOr => Union;
    BitAnd => Intersect

    Priority_Increasing
}

binops! {
    as get_patop_info
    type TokenKind -> PatternOp = PatOpInfo:

    Or;

    And;

    Xor

    Priority_Increasing
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

    Priority_Increasing
}

binops! {
    as get_logicop_info
    type TokenKind -> LogicOp = LogicOpInfo:

    SemiColon => Or;

    Comma => And

    Priority_Increasing
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaChunk {
    pub block: Block,
    pub span: Span,
    pub source_info: SourceInfo,
    pub logic: Box<LogicDatabase>,
}

impl Visit for DukaChunk {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        visitor.before();
        self.block.visit(visitor);
        visitor.after();
    }
}
impl VisitMut for DukaChunk {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        visitor.before();
        self.block.visit_mut(visitor);
        visitor.after();
    }
}
