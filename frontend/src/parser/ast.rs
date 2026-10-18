use std::{
    fmt::Display,
    ops::{Add, BitAnd, BitOr, Div, Mul, Sub},
};

use duka_macros::{Info, Visitor, VisitorMut, binops};
use serde::{Deserialize, Serialize};

use crate::analyzer::{Visit, VisitMut, Visitor, VisitorMut};
use crate::lexer::token::{Token, TokenKind};
use duka_shared::{
    constants::{ccallish, cpar},
    dtype::{FunctionType, Type},
    errors::Span,
    types::{BinOp, LogicDatabase, LogicOp, Pipeline, SourceInfo, Spanned, SysCall, UnOp},
    value::{AttrValue, ConstValue},
};

#[derive(Debug, PartialEq, Clone)]
pub enum ExprOrStmt {
    Expr(Node),
    Stmt(Node),
}
impl ExprOrStmt {
    pub fn get_span(&self) -> Span {
        match self {
            Self::Expr(Node(_, sp)) => *sp,
            Self::Stmt(Node(_, sp)) => *sp,
        }
    }
    pub fn into_block(self) -> Block {
        match self {
            Self::Expr(Node(ek, sp)) => {
                Block([Node(NodeKind::Return([Node(ek, sp)].into(), false), sp)].into())
            }
            Self::Stmt(s) => Block([s].into()),
        }
    }
}

#[derive(Debug, PartialEq, Default, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
#[ast(node)]
pub struct Node(pub NodeKind, #[nonvisiting] pub Span);
impl Mul<NodeKind> for Span {
    type Output = Node;
    fn mul(self, rhs: NodeKind) -> Self::Output {
        Node(rhs, self)
    }
}
/// A statement and an expression are one node. What a position accepts is
/// `#[tag(stmt_only)]` rather than a different type.
pub type Expr = Node;
pub type Stmt = Node;

#[derive(Debug, PartialEq, Default, Info, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum NodeKind {
    #[default]
    #[tag(empty)]
    #[tag(tcx)]
    Empty,

    #[tag(sugar)]
    #[tag(user)]
    BangCollected(#[nonvisiting] BangCollected),

    #[tag(stmt_only)]
    #[tag(tcx)]
    Expr(Box<Node>),
    #[tag(tcx)]
    Call(Box<Node>, Box<[Node]>),

    #[tag(stmt_only)]
    Label(#[nonvisiting] Name),
    #[tag(stmt_only)]
    Goto(#[nonvisiting] Name),
    /// (label, values, banged)
    ///
    /// `break` is the exit of the nearest breakable body -- a loop, or a
    /// `do`-expression. Carrying values makes it the value of that body, which
    /// is the only thing a `do`-expression has; inside a loop there is nowhere
    /// to put them and `LoopChecker` rejects the form.
    #[tag(stmt_only)]
    #[tag(tcx)]
    Break(
        #[nonvisiting] Option<Name>,
        Box<[Node]>,
        #[nonvisiting] bool,
    ),
    #[tag(stmt_only)]
    #[tag(tcx)]
    Continue(#[nonvisiting] Option<Name>),
    /* (values, banged) */
    #[tag(stmt_only)]
    #[tag(tcx)]
    Return(Box<[Node]>, #[nonvisiting] bool),

    #[tag(stmt_only)]
    #[tag(sugar)]
    Object(Box<ObjectDef>),
    #[tag(stmt_only)]
    #[tag(sugar)]
    Export(Box<Node>),

    #[tag(tcx)]
    If(If),

    #[tag(tcx)]
    VarArg,
    #[tag(tcx)]
    Literal(#[nonvisiting] ConstValue),
    #[tag(tcx)]
    Access(Box<Path>),
    SysCall(#[nonvisiting] SysCall),

    #[tag(tcx)]
    Table(Box<[Field]>),
    #[tag(tcx)]
    Array(Box<[Node]>),
    /// ```lua
    /// fn(a) -> a
    /// ```
    Function(FuncBody),

    #[tag(tcx)]
    Unary(Box<Node>, #[nonvisiting] UnOp),
    #[tag(tcx)]
    Binary(Box<Node>, Box<Node>, #[nonvisiting] BinOp),
    #[tag(typesys)]
    #[tag(tcx)]
    TypeLit(#[nonvisiting] TypeDesc),

    #[tag(sugar)]
    BangDo(BangDoNode),
    #[tag(sugar)]
    Linq(Linq),
    #[tag(sugar)]
    #[tag(tcx)]
    Match(Match),

    /// var, start value, condition, step, body
    #[tag(stmt_only)]
    #[tag(tcx)]
    ForNumeric(
        Path,
        Box<Node>,
        Box<Node>,
        Option<Box<Node>>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] Option<Name>,
    ),
    #[tag(stmt_only)]
    #[tag(tcx)]
    ForGeneric(
        Box<[Path]>,
        Box<[Node]>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
        #[nonvisiting] Option<Name>,
    ),
    #[tag(stmt_only)]
    #[tag(tcx)]
    While(
        Box<Node>,
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
    #[tag(stmt_only)]
    #[tag(tcx)]
    Assign(Box<[Path]>, Box<[Node]>),
    ///```lua
    /// local var = 1
    /// global var = 2
    /// ```
    #[tag(stmt_only)]
    #[tag(tcx)]
    Define(
        #[nonvisiting] Box<[AttrName]>,
        Box<[Node]>,
        #[nonvisiting] bool, /* is global? */
        #[nonvisiting] bool, /* banged ? */
    ),
    #[tag(stmt_only)]
    #[tag(sugar)]
    ///```lua
    /// local { a, b, c = [a, b, c] } = table
    /// ```
    Destructing(
        Destructuring,
        Box<Node>,
        #[nonvisiting] bool, /* is global? */
    ),
    ///```lua
    /// [global] function a(b)
    /// ...
    /// end
    /// ```
    #[tag(stmt_only)]
    FunctionDecl(
        Path,
        #[nonvisiting] Attrs,
        Box<FuncBody>,
        #[nonvisiting] bool,
    ),
    #[tag(stmt_only)]
    #[tag(typesys)]
    #[tag(tcx)]
    ///```ts
    /// type Alias = int | string
    /// ```
    TypeAlias(#[nonvisiting] Name, #[nonvisiting] Box<TypeDesc>),
    #[tag(stmt_only)]
    #[tag(typesys)]
    ///```ts
    /// type function F(a, b) -> type
    ///     return a | b
    /// end
    /// ```
    TypeFunction(#[nonvisiting] Name, #[nonvisiting] Box<FuncBody>),
    #[tag(stmt_only)]
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
impl NodeKind {
    pub const fn is_banged(&self) -> bool {
        matches!(
            self,
            Self::Define(.., true)
                | Self::While(.., true, _)
                | Self::ForGeneric(.., true, _)
                | Self::Return(.., true)
                | Self::Do(.., true, _)
        )
    }

    #[inline]
    pub fn is_breakable_do(&self) -> bool {
        !matches!(self, Self::Do(_, _, Some((name, _))) if name == cpar::NOLAB)
    }

    #[inline]
    pub fn is_const(&self) -> bool {
        matches!(self, Self::Literal(lit) if lit.is_const())
    }

    #[inline]
    pub const fn is_self_call(&self) -> bool {
        matches!(self, Self::Access(path) if path.is_self_call())
    }
    #[inline]
    pub fn is_fake_call(&self) -> bool {
        matches!(self, Self::Access(path) if path.is_fake_call())
    }
    #[inline]
    pub fn is_callish(&self) -> Option<&'static str> {
        if let Self::Access(path) = self {
            path.is_callish()
        } else {
            None
        }
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
    pub span: Span,
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
pub struct Block(pub Box<[Stmt]>);
impl VisitMut for Block {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        visitor.visit_block(self);
    }
}
impl Block {
    pub fn empty() -> Self {
        Self(Box::new([]))
    }
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn always_returns(&self) -> bool {
        self.0.last().is_some_and(|last| node_exits(&last.0))
    }
}

fn node_exits(node: &NodeKind) -> bool {
    match node {
        NodeKind::Return(..) | NodeKind::Break(..) => true,
        NodeKind::Expr(inner) => node_exits(&inner.0),
        NodeKind::Do(body, ..) => body.always_returns(),
        NodeKind::If(if_) => {
            if_.2.is_some()
                && if_.0.0.always_returns()
                && if_.1.iter().all(|clause| clause.0.always_returns())
        }
        NodeKind::Match(m) => m.2.is_some() && m.1.iter().all(|clause| clause.1.always_returns()),
        NodeKind::While(_, _, banged, _) => *banged,
        _ => false,
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
pub enum Destructuring {
    /// A bare name, which takes the whole value: `type C = T`.
    ///
    /// Its own variant rather than a one-element pattern, because a pattern reads
    /// its value apart by position or by field -- so an `Array` holding a single
    /// name would mean the first element, and `type C = T` would bind `C` to one
    /// part of `T` instead of all of it.
    #[nonvisiting]
    Name(Name), // FIXME
    Table(Box<[DestructingTableTerm]>),
    Array(Box<[DestructingTerm]>),
}
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct DestructingTableTerm(#[nonvisiting] pub Name, pub DestructingTerm);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum DestructingTerm {
    Bind(#[nonvisiting] Name),
    /// `_` -- read and thrown away. Its own variant rather than a name, because a
    /// name is in scope after the statement and `_` is not.
    Ignored(#[nonvisiting] Span),
    /// `...` -- everything the pattern did not reach, bound to the name beside it
    /// where there is one. `...` alone collects nothing.
    Rest(#[nonvisiting] Option<Name>, #[nonvisiting] Span),
    Term(Destructuring),
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

macro_rules! compile_time_binary {
    ($opp: ident use $op: ident impl $func: ident) => {
        impl $op for Expr {
            type Output = Expr;
            fn $func(self, rhs: Self) -> Self::Output {
                let span = self.1 + rhs.1;
                Node(
                    NodeKind::Binary(Box::new(self), Box::new(rhs), BinOp::$opp),
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

pub type Attr = (Name, Box<[(Name, AttrValue)]>);
pub type Attrs = Box<[Attr]>;
pub type Name = Spanned<String>;
/// 可选的类型注时节存放在`.2`
pub type AttrName = Spanned<(Name, Attrs, Option<TypeDesc>)>;

pub fn get_attr(attrs: &Attrs, who: &str) -> Option<Box<[(Name, AttrValue)]>> {
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
    /// `<...Ts>` against `<T>`
    pub ParamShape,
);

/// Whether a type parameter names one type or a list of them.
///
/// `...Ts` binds `Ts` to whatever list of types the call turned out to be, and
/// that list is a `Type::TypeTuple` like any other. Nothing is added to the type
/// language for this: a pack parameter is an ordinary parameter whose type
/// happens to be a tuple of types, so `Ts` reads with the same `Len` and `++`
/// and indexing that any other tuple does.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
pub enum ParamShape {
    /// One type, `T`.
    #[default]
    Fixed,
    /// A list of types, `...Ts`.
    Pack,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Param {
    Var(Span),
    Name(Name),
    /// 带类型标注的参数
    Typed(Name, TypeDesc),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PathSuffix {
    /// `path?`
    NilCheck(#[nonvisiting] Span),
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
            PathSuffix::NilCheck(span) => *span,
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
            PathSuffix::NilCheck(_) => write!(f, "?"),
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
    pub fn is_fake_call(&self) -> bool {
        matches!(self, Path::Base((n, _)) if ccallish::FAKES.contains(&n.as_str()))
    }
    #[inline]
    pub fn is_callish(&self) -> Option<&'static str> {
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
        #[nonvisiting] Destructuring,
        #[nonvisiting] Box<TypeDesc>,
        #[nonvisiting] Span,
    ),
}

/// 在AST层面的对于类型的描述符, 供TypeEval使用
///
/// A type position is an `Expr`, and this is the grammar of the plain type
/// shapes that can sit inside one, wrapped in `NodeKind::TypeLit`. Every slot
/// here is an `Expr` rather than a `TypeDesc` so that a type position can hold
/// a computation wherever it holds a name: `array<IsList(T)>` is the same shape
/// as `array<int>`, and the difference is only what the leaf evaluates to.
/// A type written as a computation is held as one expression, and these are how
/// the two spellings of one are built.
///
/// `F(a)`, `x.f(a)` and `x[i]` are all applications, and the evaluator applies a
/// path to arguments in one place for all of them. Holding them as three separate
/// shapes meant the same computation had to be recognised three times over -- in
/// the name collector, in the display, in the checker -- while a fourth spelling
/// (`F(a) + 1`) was only understood by the third. One shape means there is one
/// thing to be right about.
///
/// The arguments go in wrapped as `TypeLit`, because they are types written where
/// the expression grammar wants a value; that is what `eval_expr_to_type` unwraps
/// on the way in.
///
/// `type_access` is where this is easy to get wrong. The base is not a name, it is
/// whatever came before -- very often an application in its own right, as in
/// `require("mod").thing`. Encoding it as a bare name loses the arguments that
/// named the module, and a module that cannot be named cannot be read from, so the
/// base is rebuilt as a path here rather than flattened.
pub fn type_application(name: Box<str>, args: Box<[TypeDesc]>, span: Span) -> TypeDesc {
    let call = application_call(name.into_string(), args, span);
    TypeDesc::Expr(Box::new(call))
}

fn application_call(name: String, args: Box<[TypeDesc]>, span: Span) -> Expr {
    let path = Path::Base((name, span));
    let args: Box<[Expr]> = args
        .iter()
        .map(|t| Node(NodeKind::TypeLit(t.clone()), span))
        .collect();
    Node(
        NodeKind::Call(Box::new(Node(NodeKind::Access(Box::new(path)), span)), args),
        span,
    )
}

/// `x.f` or `x[i]`, with or without an argument list.
pub fn type_access(
    base: TypeDesc,
    member: TypeDesc,
    args: Option<Box<[TypeDesc]>>,
    span: Span,
) -> TypeDesc {
    let base_path = path_of(&base, span);
    // `.name` and `[i]` are the two ways to read past something, and they are one
    // suffix in the path either way. Which one it is has to survive: a dot is a
    // name and a bracket is an expression, and the reader treats them differently.
    let suffix = match &member {
        TypeDesc::Pure(Type::Literal(ConstValue::String(name))) => {
            PathSuffix::Dot((String::from_utf8_lossy(name).into_owned(), span))
        }
        other => PathSuffix::Index(Box::new(Node(NodeKind::TypeLit(other.clone()), span))),
    };
    let path = Path::Chain(Box::new(base_path), suffix);
    let accessed = Node(NodeKind::Access(Box::new(path)), span);
    match args {
        // `x.f` is a read; `x.f(a)` is an application of one. The evaluator reads
        // them differently -- a read goes through member lookup, an application
        // binds arguments -- so which one it is has to survive the encoding.
        None => TypeDesc::Expr(Box::new(accessed)),
        Some(args) => {
            let args: Box<[Expr]> = args
                .iter()
                .map(|t| Node(NodeKind::TypeLit(t.clone()), span))
                .collect();
            TypeDesc::Expr(Box::new(Node(
                NodeKind::Call(Box::new(accessed), args),
                span,
            )))
        }
    }
}

/// The path `base` is read through.
///
/// The base is already an application or a type, both of which are one shape now,
/// so there is nothing to unwrap: `require("m")` arrives here as the expression it
/// is, and the path reader recognises it as naming a module. Flattening it to a
/// bare name here is what would lose the `"m"`.
pub fn path_of(base: &TypeDesc, span: Span) -> Path {
    Path::Expr(Box::new(Node(NodeKind::TypeLit(base.clone()), span)))
}

/// One field of a record.
///
/// This is a struct rather than a tuple because the derived visitor skips whole
/// fields, not parts of one. As a tuple the key and the span were visited
/// alongside the field type, which is both meaningless -- a key is a name, not a
/// node -- and unworkable, since neither implements `Visit`. The field type is the
/// part a walk has to reach, so it is the only part that is marked for visiting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Visitor, VisitorMut)]
pub struct TypeField {
    #[nonvisiting]
    pub key: Box<str>,
    #[nonvisiting]
    pub span: Span,
    pub ty: TypeDesc,
}

impl TypeField {
    pub fn new(key: Box<str>, span: Span, ty: TypeDesc) -> Self {
        Self { key, span, ty }
    }
}

/// The derived visitor walks a type the way it walks anything else, which is what
/// makes `F(a)` and `F<b>` reachable by the same code.
///
/// The walk is written by hand rather than derived because the derived visitor
/// skips whole fields, and this type is mostly made of fields that are not nodes:
/// a name, a span, a literal, a function signature. A derived walk of it would
/// either not compile or reach nothing, which is the reason `type_children` and
/// `expressions` existed -- two hand-written lists of children to be kept in step
/// by hand, one of which had been left out of entirely for the shapes that
/// compute. Every type position is reached from the one `match` below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeDesc {
    Pure(Type),
    Named(Box<str>, Span),
    Generic(Box<str>, Box<[TypeDesc]>, Span),
    TypeOf(Box<Expr>, Span),
    Array(Option<Box<TypeDesc>>),
    Table(Option<Box<TypeDesc>>, Option<Box<TypeDesc>>),
    Union(Box<[TypeDesc]>),
    TypeTuple(Box<[TypeDesc]>),
    /// A record. The key keeps its span so that declaring the record can turn
    /// each field into a symbol of the alias rather than leaving a language
    /// server to guess where a field was written.
    TypeTable(Box<[TypeField]>),
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
    /// Where the annotation was written.
    ///
    /// A type is not only what it denotes but where it was written, and a
    /// diagnostic that talks about a type has to be able to point at it. Only
    /// the forms that are written as something on the page carry a span of
    /// their own; the rest are built around one that does.
    pub fn span(&self) -> Option<Span> {
        match self {
            TypeDesc::Named(_, span)
            | TypeDesc::Generic(_, _, span)
            | TypeDesc::TypeOf(_, span) => Some(*span),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                inner.span()
            }
            TypeDesc::Array(inner) => inner.as_deref().and_then(TypeDesc::span),
            TypeDesc::Table(key, value) => key
                .as_deref()
                .and_then(TypeDesc::span)
                .or_else(|| value.as_deref().and_then(TypeDesc::span)),
            TypeDesc::Union(types) | TypeDesc::TypeTuple(types) => {
                types.iter().find_map(TypeDesc::span)
            }
            _ => None,
        }
    }

    pub fn base_type(&self) -> Option<&Type> {
        match self {
            TypeDesc::Pure(t) => Some(t),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) => inner.base_type(),
            _ => None,
        }
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
    pub fn typetable_of(items: Box<[TypeField]>) -> TypeDesc {
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
                    .map(|f| {
                        let (k, v) = (&f.key, &f.ty);
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
            | TypeDesc::Generic(..)
            | TypeDesc::TypeOf(..)
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
            TypeDesc::Generic(name, args, _) => write!(
                f,
                "{name}<{}>",
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            // `F(a)`, `x.f(a)` and `x[i]` are applications now, and they are rendered by
            // `tcx::render` -- the same renderer a type-level expression written
            // with operators goes through. That is what there being one shape buys:
            // `F(a)` and `F(a) + 1` read back the same way whether or not the
            // second one needed a parser of its own.
            TypeDesc::Expr(expr) => write!(f, "{}", crate::analyzer::tcx::render(expr)),
            TypeDesc::TypeOf(..) => write!(f, "type(...)"),
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
            // A tuple is written `[a, b]`, parsed from `[a, b]`, and `Type::TypeTuple`
            // renders it that way too. This arm drew it `(a, b)` -- the shape a
            // grouping or a union takes -- so `where T: [V]` came back as
            // `where T: (V)`: a bound nobody wrote, reading as something else.
            //
            // A bound has to read the way it was written, or a reader cannot tell
            // the type the author wrote from the checker's opinion of it.
            TypeDesc::TypeTuple(ts) => write!(
                f,
                "[{}]",
                ts.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDesc::TypeTable(ts) => write!(
                f,
                "table[{}]",
                ts.iter()
                    .map(|f| format!("{}: {}", f.key, f.ty))
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
    /// What a `---` documented, keyed by the span of what it documented.
    ///
    /// Carried here rather than on each statement because the document is not
    /// part of the language: a declaration is the same declaration whether or not
    /// anyone wrote a line above it, and this is only what a reader has to say
    /// about it.
    pub docs: std::collections::HashMap<Span, String>,
}

/// The one walk over a type.
///
/// Every arm that can hold another type reaches it, and an arm that cannot is
/// absent rather than matched-and-ignored -- so adding a shape that holds a type
/// and forgetting this is a change that does not compile, which is the property
/// the two hand-written lists this replaces did not have.
impl Visit for TypeDesc {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        match self {
            TypeDesc::Named(name, span) => visitor.visit_type_name(name, span),
            TypeDesc::Generic(_, args, _) => args.visit(visitor),
            TypeDesc::TypeOf(expr, _) => expr.visit(visitor),
            TypeDesc::Array(Some(inner)) => inner.visit(visitor),
            TypeDesc::Table(k, v) => {
                for inner in [k, v].into_iter().flatten() {
                    inner.visit(visitor);
                }
            }
            TypeDesc::Union(items) | TypeDesc::TypeTuple(items) => items.visit(visitor),
            TypeDesc::TypeTable(fields) => {
                for f in fields.iter() {
                    f.ty.visit(visitor);
                }
            }
            TypeDesc::Function(None) => {}
            TypeDesc::Function(Some(ft)) => {
                ft.params.visit(visitor);
                ft.returns.visit(visitor);
            }
            TypeDesc::FnLit(body) => body.visit(visitor),
            TypeDesc::Expr(expr) => expr.visit(visitor),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                inner.visit(visitor)
            }
            // `Pure` is a `Type` and `Array(None)` is an array of nothing: both
            // hold no type to reach.
            TypeDesc::Pure(_) | TypeDesc::Array(None) => {}
        }
    }
}

impl VisitMut for TypeDesc {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        match self {
            TypeDesc::Named(name, span) => visitor.visit_type_name(name, span),
            TypeDesc::Generic(_, args, _) => args.visit_mut(visitor),
            TypeDesc::TypeOf(expr, _) => expr.visit_mut(visitor),
            TypeDesc::Array(Some(inner)) => inner.visit_mut(visitor),
            TypeDesc::Table(k, v) => {
                if let Some(k) = k.as_deref_mut() {
                    k.visit_mut(visitor);
                }
                if let Some(v) = v.as_deref_mut() {
                    v.visit_mut(visitor);
                }
            }
            TypeDesc::Union(items) | TypeDesc::TypeTuple(items) => items.visit_mut(visitor),
            TypeDesc::TypeTable(fields) => {
                for f in fields.iter_mut() {
                    f.ty.visit_mut(visitor);
                }
            }
            TypeDesc::Function(None) => {}
            TypeDesc::Function(Some(ft)) => {
                ft.params.visit_mut(visitor);
                ft.returns.visit_mut(visitor);
            }
            TypeDesc::FnLit(body) => body.visit_mut(visitor),
            TypeDesc::Expr(expr) => expr.visit_mut(visitor),
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                inner.visit_mut(visitor)
            }
            TypeDesc::Pure(_) | TypeDesc::Array(None) => {}
        }
    }
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
