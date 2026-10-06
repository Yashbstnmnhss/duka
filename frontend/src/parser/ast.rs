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
    dtype::{FunctionType, Type},
    errors::Span,
    types::{BinOp, LogicDatabase, LogicOp, Pipeline, SourceInfo, Spanned, SysCall, UnOp},
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
                Some(Box::new(Stmt(
                    StmtKind::Return([Expr(ek, sp)].into(), false, None),
                    sp,
                ))),
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
    #[tag(tcx)]
    Empty,

    #[tag(sugar)]
    #[tag(user)]
    BangCollected(#[nonvisiting] BangCollected),

    #[tag(tcx)]
    Expr(Box<Expr>),
    #[tag(tcx)]
    Call(Box<Expr>, Box<[Expr]>),

    Label(#[nonvisiting] Name),
    Goto(#[nonvisiting] Name),
    #[tag(tcx)]
    Break(#[nonvisiting] Option<Name>),
    #[tag(tcx)]
    Continue(#[nonvisiting] Option<Name>),
    /* (values, banged) */
    #[tag(tcx)]
    Return(
        Box<[Expr]>,
        #[nonvisiting] bool,
        #[nonvisiting] Option<Name>,
    ),

    #[tag(sugar)]
    #[tag(tcx)]
    Match(Match),
    #[tag(sugar)]
    Object(Box<ObjectDef>),
    #[tag(sugar)]
    Export(Box<Stmt>),

    #[tag(tcx)]
    If(If),
    /// var, start value, condition, step, body
    #[tag(tcx)]
    ForNumeric(
        Path,
        Box<Expr>,
        Box<Expr>,
        Option<Box<Expr>>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] Option<Name>,
    ),
    #[tag(tcx)]
    ForGeneric(
        Box<[Path]>,
        Box<[Expr]>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
        #[nonvisiting] Option<Name>,
    ),
    #[tag(tcx)]
    While(
        Box<Expr>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
        #[nonvisiting] Option<Name>,
    ),
    /// ```lua
    /// do ::name::
    /// ...
    /// end
    /// ```
    #[tag(tcx)]
    Do(
        #[block(do_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
        #[nonvisiting] Option<Name>,
    ),

    ///```lua
    /// var = 1
    /// ```
    #[tag(tcx)]
    Assign(Box<[Path]>, Box<[Expr]>),
    ///```lua
    /// local var = 1
    /// global var = 2
    /// ```
    #[tag(tcx)]
    Define(
        #[nonvisiting] Box<[AttrName]>,
        Box<[Expr]>,
        #[nonvisiting] bool, /* is global? */
        #[nonvisiting] bool, /* banged ? */
    ),
    #[tag(sugar)]
    ///```lua
    /// local { a, b, c = [a, b, c] } = table
    /// ```
    Destructing(
        Destructing,
        Box<Expr>,
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
    #[tag(tcx)]
    ///```ts
    /// type Alias = int | string
    /// ```
    TypeAlias(#[nonvisiting] Name, #[nonvisiting] Box<TypeDesc>),
    #[tag(typesys)]
    ///```ts
    /// type function F(a, b) -> type
    ///     return a | b
    /// end
    /// ```
    TypeFunction(#[nonvisiting] Name, #[nonvisiting] Box<FuncBody>),
    #[tag(typesys)]
    ///```ts
    /// type function List(t) = { head: t, tail: List(t)? }
    /// ```
    InlineTypeFunction(
        #[nonvisiting] Name,
        #[nonvisiting] Box<[Param]>,
        #[nonvisiting] Box<TypeDesc>,
    ),
}
impl StmtKind {
    pub const fn is_banged(&self) -> bool {
        matches!(
            self,
            Self::Define(.., true)
                | Self::While(.., true, _)
                | Self::ForGeneric(.., true, _)
                | Self::Return(.., true, _)
                | Self::Do(.., true, _)
        )
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct FuncBody(
    #[nonvisiting] pub Box<[Param]>,
    #[nonvisiting] pub Box<[TypeParam]>,
    #[nonvisiting] pub Option<ReturnAnnotation>,
    /// `where U: Point, Sized(T), type V = T`
    ///
    /// Between the signature and the body, and read top to bottom in that order,
    /// which is what makes a concept able to see a binding written above it.
    ///
    /// Not `#[nonvisiting]`, unlike the fields above it: a concept is an
    /// expression and has to be walked, because the module dependency walk and
    /// generic name normalisation both need the names inside it.
    pub Box<[WhereClause]>,
    #[block(func)]
    #[block_mut]
    pub Box<Block>,
);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_var_arg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
    }
    /// The parameters that name a value, with the annotation each one carries.
    /// The rest parameter is deliberately absent: it says how many arguments
    /// may follow rather than what one of them is, so it belongs to
    /// `var_arg`. Anything that builds a parameter list has to go through here,
    /// or a signature grows a phantom slot for every `...` it declares.
    pub fn named_params(&self) -> impl Iterator<Item = (&Name, Option<&TypeDesc>)> {
        self.0.iter().filter_map(|p| match p {
            Param::Typed(name, ty) => Some((name, Some(ty))),
            Param::Name(name) => Some((name, None)),
            Param::Var(_) => None,
        })
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ReturnAnnotation {
    pub tys: Box<[TypeDesc]>,
    pub var_arg: bool,
}

#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct If(
    pub IfClause,
    pub Box<[IfClause]>,
    #[block_mut] pub Option<Box<Block>>,
);
#[derive(Debug, PartialEq, Clone, Default, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct IfClause(
    #[block(if_clause)]
    #[block_mut]
    pub Box<Block>,
    pub Box<Expr>,
);

#[derive(Debug, PartialEq, Default, Clone, Visitor, Serialize, Deserialize)]
pub struct Block(pub Box<[Stmt]>, pub Option<Box<Stmt>>);
impl VisitMut for Block {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        visitor.visit_block(self);
    }
}
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
    #[block(match_else)]
    #[block_mut]
    pub Option<Box<Block>>,
);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct MatchClause(
    pub Pattern,
    #[block(match_clause)]
    #[block_mut]
    pub Box<Block>,
);

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum Destructing {
    Table(Box<[DestructingTableTerm]>),
    Array(Box<[DestructingTerm]>),
}
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct DestructingTableTerm(#[nonvisiting] pub Name, pub DestructingTerm);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum DestructingTerm {
    Bind(#[nonvisiting] Name),
    Term(Destructing),
}

/// guard mode
pub type Pattern = (PatternTerm, Option<Box<Expr>>);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternTerm {
    /// `123`
    Constant(Box<Expr>),
    /// `local name: type`
    Bind(#[nonvisiting] Name, #[nonvisiting] Option<TypeDesc>),
    /// `|> func() then (subs)`
    Call(#[nonvisiting] Pipeline, Box<Expr>, Option<Box<PatternTerm>>),
    /// `> 2`
    Compare(#[nonvisiting] BinOp, Box<Expr>),
    /// `{ 1, ..., 5, _, _, a = local var, [true] = |> func }`
    Table(Box<[PatternFieldTerm]>),
    /// also for array `[ 1, ..., 5 ]`
    Array(Box<[PatternArrayTerm]>),
    /// `> 2 and < 5`
    Compound(Box<PatternTerm>, Box<PatternTerm>, #[nonvisiting] PatternOp),
    /// `not ...`
    Not(Box<PatternTerm>),

    /// `Array(inner)`, `Table(k, v)` (type-context only)
    Type(#[nonvisiting] Name, Box<[PatternTerm]>),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternFieldTerm<T = PatternTerm>
where
    T: Visit + VisitMut,
{
    Array(PatternArrayTerm<T>),
    Named(#[nonvisiting] Name, T),
    Expr(Expr, PatternTerm),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternArrayTerm<T = PatternTerm>
where
    T: Visit + VisitMut,
{
    /// `_ * n`
    Discard(#[nonvisiting] usize),
    /// `...`           
    DiscardMany,
    /// term       
    Term(T),
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
        #[nonvisiting] Option<TypeDesc>,
    ),
    KeyValue(
        Box<Expr>,
        Option<Box<Expr>>,
        #[nonvisiting] Option<TypeDesc>,
    ),
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
    #[tag(tcx)]
    Empty,

    #[tag(sugar)]
    Linq(Linq),
    #[tag(sugar)]
    #[tag(tcx)]
    Match(Match),

    #[tag(tcx)]
    VarArg,
    #[tag(tcx)]
    Literal(#[nonvisiting] ConstValue),
    #[tag(tcx)]
    Do(
        #[block(do_expr)]
        #[block_mut]
        Box<Block>,
    ),

    #[tag(tcx)]
    Access(Box<Path>),
    #[tag(tcx)]
    Call(Box<Expr>, Box<[Expr]>),

    SysCall(#[nonvisiting] SysCall),

    #[tag(tcx)]
    Table(Box<[Field]>),
    #[tag(tcx)]
    Array(Box<[Expr]>),
    Function(FuncBody),

    #[tag(tcx)]
    Unary(Box<Expr>, #[nonvisiting] UnOp),
    #[tag(tcx)]
    Binary(Box<Expr>, Box<Expr>, #[nonvisiting] BinOp),
    #[tag(tcx)]
    If(Box<If>),
    #[tag(typesys)]
    #[tag(tcx)]
    TypeLit(#[nonvisiting] TypeDesc),

    #[tag(sugar)]
    BangDo(BangDoNode),
    #[tag(sugar)]
    #[tag(user)]
    BangCollected(#[nonvisiting] BangCollected),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Visitor, VisitorMut)]
pub struct BangDoNode {
    pub context: Box<Expr>,
    pub body: Box<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BangCollectedSource {
    Tokens(Vec<Token>, Span),
    Raw(String),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BangCollected {
    pub name: String,
    pub source: BangCollectedSource,
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
pub type AttrName = Spanned<(Name, Attrs, Option<TypeDesc>)>;

pub fn get_attr(attrs: &Attrs, who: &str) -> Option<Box<[(Name, ConstValue)]>> {
    attrs
        .iter()
        .find_map(|i| (i.0.0 == who).then_some(i.1.clone()))
}
pub fn has_attr(attrs: &Attrs, who: &str) -> bool {
    attrs.iter().any(|(n, _)| n.0 == who)
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TypeParam(
    pub Name,
    /// `T: bound`
    pub Option<TypeDesc>,
    /// `T = int`, used when inference determines nothing
    pub Option<TypeDesc>,
);

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Param {
    Var(Span),
    Name(Name),
    /// 带类型标注的参数
    Typed(Name, TypeDesc),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PathSuffix {
    /// `path.name`
    Dot(#[nonvisiting] Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(#[nonvisiting] Name),
    /// `path.<types,...>`
    TypeArgs(#[nonvisiting] Box<[TypeDesc]>, #[nonvisiting] Span),
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
            PathSuffix::TypeArgs(..) => write!(f, ".<type(s)>"),
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

    Pipeline param;
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

/// `where` 语句
///
/// The right-hand sides are `TypeDesc`, the same shape a type parameter's bound
/// and default are written in, and for the same reason: a where clause is part
/// of a signature and is read by the type grammar, which does not treat the
/// bracket that closes a type argument list as an operator. A position that
/// genuinely wants an expression holds it as `TypeDesc::Expr`, which is the one
/// escape that shape already has.
///
/// A concept is the exception and is held as an expression outright, because that
/// is what a concept is: something that computes an answer rather than a type.
/// `T == int` and `Sized(T) or T == any` are not type shapes.
///
/// The visitor is derived, and the `#[nonvisiting]` fields are the reason the
/// bound and the binding do not need one of their own: a name is a name and a
/// `TypeDesc` is deliberately opaque to the derived walk, so only the concept is
/// something to walk into. Consumers reach a bound or a binding through
/// `TypeDesc::expressions` and `TypeDesc::type_children`, the same way they
/// already reach into a type parameter's bound.
#[derive(Debug, Clone, PartialEq, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum WhereClause {
    /// `U: Point` -- `U` has to be a subtype of `Point`.
    ///
    /// This is a subtype and nothing more. It says what `U` may be, and the same
    /// bound is what the body of the function is checked against, so a member
    /// read off `U` has something to stand on.
    Bound(
        #[nonvisiting] Name,
        #[nonvisiting] Box<TypeDesc>,
        #[nonvisiting] Span,
    ),
    /// `Sized(T)` -- a concept: a type-level expression read for its truth.
    ///
    /// The reading is Duka's own: `nil` and `false` fail and everything else
    /// answers, because a concept is written over types and almost no type is a
    /// boolean. A concept that cannot be decided yet is an obligation, not a
    /// failure.
    Concept(Box<Expr>, #[nonvisiting] Span),
    /// `type V = T` -- a type-level binding, whose right-hand side is an
    /// expression and may be anything one.
    ///
    /// The name is the one being introduced. A `Destructing` rather than a `Name`
    /// because `type {A, B} = T` is the same statement with more than one name
    /// on the left, exactly as `local {a, b} = e` is.
    Bind(
        #[nonvisiting] Destructing,
        #[nonvisiting] Box<TypeDesc>,
        #[nonvisiting] Span,
    ),
}

/// 在AST层面的对于类型的描述符, 供TypeEval使用
///
/// A type position is an `Expr`, and this is the grammar of the plain type
/// shapes that can sit inside one, wrapped in `ExprKind::TypeLit`. Every slot
/// here is an `Expr` rather than a `TypeDesc` so that a type position can hold
/// a computation wherever it holds a name: `array<IsList(T)>` is the same shape
/// as `array<int>`, and the difference is only what the leaf evaluates to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeDesc {
    Pure(Type),
    Named(Box<str>, Span),
    Generic {
        name: Box<str>,
        args: Box<[TypeDesc]>,
        span: Span,
    },
    TypeCall {
        name: Box<str>,
        args: Box<[TypeDesc]>,
        span: Span,
    },
    Access {
        base: Box<TypeDesc>,
        member: Box<TypeDesc>,
        args: Option<Box<[TypeDesc]>>,
        span: Span,
    },
    TypeOf {
        expr: Box<Expr>,
        span: Span,
    },
    Array(Option<Box<TypeDesc>>),
    Table(Option<Box<TypeDesc>>, Option<Box<TypeDesc>>),
    Union(Box<[TypeDesc]>),
    TypeTuple(Box<[TypeDesc]>),
    /// A record. The key keeps its span so that declaring the record can turn
    /// each field into a symbol of the alias rather than leaving a language
    /// server to guess where a field was written.
    TypeTable(Box<[(Box<str>, Span, TypeDesc)]>),
    Function(Option<TypeFnValue>),
    FnLit(Box<FuncBody>),
    /// A general type-level expression. A type position is not a closed
    /// grammar: it is the language the type functions are written in, so an
    /// annotation may compute, branch and call rather than only name a type.
    /// The shapes above stay the written form of a plain type, and this is the
    /// single place the rest lands. It is evaluated by `EvalCtx`, the same
    /// evaluator that runs a `type function` body.
    Expr(Box<Expr>),
    NonNil(Box<TypeDesc>),
    Nilable(Box<TypeDesc>),
    Rec(Box<TypeDesc>),
}

impl Default for TypeDesc {
    fn default() -> Self {
        Self::Pure(Default::default())
    }
}
impl TypeDesc {
    pub fn base_type(&self) -> Option<&Type> {
        match self {
            TypeDesc::Pure(t) => Some(t),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) => inner.base_type(),
            _ => None,
        }
    }

    /// Every expression this shape holds, so a type position can hold
    /// something other than a name. The derived visitor does not reach into a
    /// `TypeDesc` (its non-`Visit` leaves), so consumers start from here, and
    /// the match stays exhaustive: a shape added later must say where its
    /// expressions are.
    pub fn expressions(&self) -> Vec<&Expr> {
        let mut out = vec![];
        match self {
            TypeDesc::Pure(_) | TypeDesc::Named(..) | TypeDesc::FnLit(_) => {}
            TypeDesc::Generic { args, .. } | TypeDesc::TypeCall { args, .. } => {
                out.extend(args.iter().flat_map(TypeDesc::expressions))
            }
            TypeDesc::Access {
                base, member, args, ..
            } => {
                out.extend(base.expressions());
                out.extend(member.expressions());
                out.extend(
                    args.as_deref()
                        .unwrap_or_default()
                        .iter()
                        .flat_map(TypeDesc::expressions),
                );
            }
            TypeDesc::TypeOf { expr, .. } => out.push(expr),
            TypeDesc::Array(inner) => {
                if let Some(inner) = inner.as_deref() {
                    out.extend(inner.expressions());
                }
            }
            TypeDesc::Table(k, v) => {
                for td in [k, v].into_iter().flatten() {
                    out.extend(td.expressions());
                }
            }
            TypeDesc::Union(items) | TypeDesc::TypeTuple(items) => {
                out.extend(items.iter().flat_map(TypeDesc::expressions))
            }
            TypeDesc::TypeTable(fields) => {
                for (_, _, td) in fields.iter() {
                    out.extend(td.expressions());
                }
            }
            TypeDesc::Function(None) => {}
            TypeDesc::Function(Some(ft)) => {
                out.extend(ft.params.iter().flat_map(TypeDesc::expressions));
                out.extend(ft.returns.iter().flat_map(TypeDesc::expressions));
            }
            TypeDesc::Expr(expr) => out.push(expr),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                out.extend(inner.expressions())
            }
        }
        out
    }

    /// Every type this shape holds, the counterpart of `expressions`: a
    /// consumer that has to look at what a type is made of starts here rather
    /// than matching the shapes itself, and the match stays exhaustive, so a
    /// shape added later has to say where its types are.
    ///
    /// This is deliberately shallow. A shape can hold an expression as well as a
    /// type, and the types inside such an expression are reached through the
    /// derived visitor, which is what reaches them today.
    pub fn type_children(&self) -> Vec<&TypeDesc> {
        let mut out = vec![];
        match self {
            TypeDesc::Pure(_) | TypeDesc::Named(..) | TypeDesc::TypeOf { .. } => {}
            TypeDesc::Generic { args, .. } | TypeDesc::TypeCall { args, .. } => {
                out.extend(args.iter())
            }
            TypeDesc::Access {
                base, member, args, ..
            } => {
                out.push(base.as_ref());
                out.push(member.as_ref());
                out.extend(args.as_deref().unwrap_or_default().iter());
            }
            TypeDesc::Array(inner) => out.extend(inner.as_deref()),
            TypeDesc::Table(k, v) => out.extend([k, v].into_iter().flatten().map(|b| b.as_ref())),
            TypeDesc::Union(items) | TypeDesc::TypeTuple(items) => out.extend(items.iter()),
            TypeDesc::TypeTable(fields) => out.extend(fields.iter().map(|(_, _, td)| td)),
            TypeDesc::Function(None) => {}
            TypeDesc::Function(Some(ft)) => {
                out.extend(ft.params.iter());
                out.extend(ft.returns.iter());
            }
            TypeDesc::FnLit(body) => {
                let FuncBody(params, _, ret, _, _) = body.as_ref();
                out.extend(params.iter().filter_map(|p| match p {
                    Param::Typed(_, t) => Some(t),
                    _ => None,
                }));
                out.extend(ret.iter().flat_map(|r| r.tys.iter()));
            }
            // the types this one holds are inside the expression, and the
            // derived visitor does not descend into a `TypeDesc`
            TypeDesc::Expr(_) => {}
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                out.push(inner.as_ref())
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeFnValue {
    pub params: Box<[TypeDesc]>,
    pub var_arg: bool,
    pub returns: Box<[TypeDesc]>,
    pub return_var_arg: bool,
}

impl TypeDesc {
    pub fn is_pure(&self) -> bool {
        matches!(self, TypeDesc::Pure(_))
    }
    pub fn expect_pure(self) -> Option<Type> {
        match self {
            TypeDesc::Pure(t) => Some(t),
            _ => None,
        }
    }
    pub fn union(self, rhs: TypeDesc) -> TypeDesc {
        match (self, rhs) {
            (TypeDesc::Pure(a), TypeDesc::Pure(b)) => TypeDesc::Pure(a | b),
            (a, b) if a == b => a,
            (a, b) => TypeDesc::Union([a, b].into()),
        }
    }
    pub fn intersect(self, rhs: TypeDesc) -> TypeDesc {
        match (self, rhs) {
            (TypeDesc::Pure(a), TypeDesc::Pure(b)) => TypeDesc::Pure(a & b),
            _ => TypeDesc::Pure(Type::Never),
        }
    }
    pub fn nilable(self) -> TypeDesc {
        TypeDesc::Nilable(Box::new(self))
    }
    pub fn nonnilable(self) -> TypeDesc {
        TypeDesc::NonNil(Box::new(self))
    }
    pub fn array_of(elem: Option<TypeDesc>) -> TypeDesc {
        match elem {
            None => TypeDesc::Pure(Type::Array(None)),
            Some(TypeDesc::Pure(t)) => TypeDesc::Pure(Type::Array(Some(Box::new(t)))),
            Some(tv) => TypeDesc::Array(Some(Box::new(tv))),
        }
    }
    pub fn table_of(k: Option<TypeDesc>, v: Option<TypeDesc>) -> TypeDesc {
        match (&k, &v) {
            (None, None) => TypeDesc::Pure(Type::Table(None, None)),
            (Some(TypeDesc::Pure(k)), Some(TypeDesc::Pure(v))) => TypeDesc::Pure(Type::Table(
                Some(Box::new(k.clone())),
                Some(Box::new(v.clone())),
            )),
            _ => TypeDesc::Table(k.map(Box::new), v.map(Box::new)),
        }
    }
    pub fn function_of(ft: Option<TypeFnValue>) -> TypeDesc {
        let Some(ft) = ft else {
            return TypeDesc::Pure(Type::Function(None));
        };
        if ft.params.iter().all(TypeDesc::is_pure) && ft.returns.iter().all(TypeDesc::is_pure) {
            TypeDesc::Pure(Type::Function(Some(FunctionType {
                params: ft
                    .params
                    .iter()
                    .map(|t| t.clone().expect_pure().unwrap())
                    .collect(),
                var_arg: ft.var_arg,
                returns: ft
                    .returns
                    .iter()
                    .map(|t| t.clone().expect_pure().unwrap())
                    .collect(),
                return_var_arg: ft.return_var_arg,
            })))
        } else {
            TypeDesc::Function(Some(ft))
        }
    }
    pub fn tuple_of(items: Box<[TypeDesc]>) -> TypeDesc {
        if items.iter().all(TypeDesc::is_pure) {
            TypeDesc::Pure(Type::TypeTuple(
                items
                    .iter()
                    .map(|t| t.clone().expect_pure().unwrap())
                    .collect(),
            ))
        } else {
            TypeDesc::TypeTuple(items)
        }
    }
    /// A record is never folded into a `Type` here: folding would throw the
    /// spans of its keys away, and those are what makes each field a symbol.
    pub fn typetable_of(items: Box<[(Box<str>, Span, TypeDesc)]>) -> TypeDesc {
        TypeDesc::TypeTable(items)
    }
    /// The type an annotation stands for when it needs no evaluation. A name,
    /// a type call or a `type(...)` does need one, so it reads as `any` here.
    pub fn as_type(&self) -> Type {
        match self {
            TypeDesc::Pure(t) => t.clone(),
            TypeDesc::Array(e) => Type::Array(e.as_deref().map(|e| Box::new(e.as_type()))),
            TypeDesc::Table(k, v) => Type::Table(
                k.as_deref().map(|k| Box::new(k.as_type())),
                v.as_deref().map(|v| Box::new(v.as_type())),
            ),
            TypeDesc::Union(ts) => ts.iter().fold(Type::Never, |acc, t| acc | t.as_type()),
            TypeDesc::TypeTuple(ts) => Type::TypeTuple(ts.iter().map(TypeDesc::as_type).collect()),
            TypeDesc::TypeTable(ts) => Type::TypeTable(
                ts.iter()
                    .map(|(k, _, v)| {
                        (
                            ConstValue::String(k.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(v.as_type()),
                        )
                    })
                    .collect(),
            ),
            TypeDesc::Function(ft) => Type::Function(ft.as_ref().map(|ft| FunctionType {
                params: ft.params.iter().map(TypeDesc::as_type).collect(),
                var_arg: ft.var_arg,
                returns: ft.returns.iter().map(TypeDesc::as_type).collect(),
                return_var_arg: ft.return_var_arg,
            })),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) => inner.as_type(),
            TypeDesc::Rec(inner) => Type::Rec(Box::new(inner.as_type())),
            TypeDesc::Named(..)
            | TypeDesc::Generic { .. }
            | TypeDesc::TypeCall { .. }
            | TypeDesc::Access { .. }
            | TypeDesc::TypeOf { .. }
            | TypeDesc::Expr(_)
            | TypeDesc::FnLit(_) => Type::Any,
        }
    }
}

impl Display for TypeDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeDesc::Pure(t) => write!(f, "{t}"),
            TypeDesc::Named(name, _) => write!(f, "{name}"),
            TypeDesc::Generic { name, args, .. } => write!(
                f,
                "{name}<{}>",
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDesc::TypeCall { name, args, .. } => write!(
                f,
                "{name}({})",
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDesc::Access {
                base, member, args, ..
            } => match args {
                Some(args) => write!(
                    f,
                    "{base}.{member}({})",
                    args.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None => write!(f, "{base}.{member}"),
            },
            TypeDesc::TypeOf { .. } => write!(f, "type(...)"),
            TypeDesc::Array(inner) => match inner {
                Some(inner) => write!(f, "[{inner}]"),
                None => write!(f, "[]"),
            },
            TypeDesc::Table(k, v) => {
                let k = k.as_ref().map(ToString::to_string).unwrap_or_default();
                let v = v.as_ref().map(ToString::to_string).unwrap_or_default();
                write!(f, "table[{k}]({v})")
            }
            TypeDesc::Union(ts) => write!(
                f,
                "{}",
                ts.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            TypeDesc::TypeTuple(ts) => write!(
                f,
                "({})",
                ts.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDesc::TypeTable(ts) => write!(
                f,
                "table[{}]",
                ts.iter()
                    .map(|(k, _, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDesc::Function(ft) => match ft {
                Some(ft) => write!(
                    f,
                    "type function({}) -> ({})",
                    ft.params
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    ft.returns
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None => write!(f, "type function"),
            },
            TypeDesc::FnLit(_) => write!(f, "type fn"),
            TypeDesc::Expr(expr) => write!(f, "{}", crate::analyzer::tcx::render(expr)),
            TypeDesc::NonNil(inner) => write!(f, "{inner}!"),
            TypeDesc::Nilable(inner) => write!(f, "{inner}?"),
            TypeDesc::Rec(inner) => write!(f, "rec {inner}"),
        }
    }
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
        visitor.visit_block(&mut self.block);
        visitor.after();
    }
}
