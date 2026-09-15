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
                    StmtKind::Return([Expr(ek, sp)].into(), false),
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
    Empty,
    #[tag(empty)]
    Extern,

    Expr(Box<Expr>),
    Call(Box<Expr>, Box<[Expr]>),

    Label(#[nonvisiting] String),
    Goto(#[nonvisiting] String),
    Break,
    Continue,
    /* (values, banged) */
    Return(Box<[Expr]>, #[nonvisiting] bool),

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
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
    ),
    ForGeneric(
        Box<[Path]>,
        Box<[Expr]>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
    ),
    While(
        Box<Expr>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
    ),
    /// ```lua
    /// do
    /// ...
    /// end
    /// ```
    Do(
        #[block(do_stmt)]
        #[block_mut]
        Box<Block>,
        #[nonvisiting] bool, /* banged? */
    ),

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
                | Self::While(.., true)
                | Self::ForGeneric(.., true)
                | Self::Return(.., true)
                | Self::Do(.., true)
        )
    }
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct FuncBody(
    #[nonvisiting] pub Box<[Param]>,
    #[nonvisiting] pub Box<[TypeParam]>,
    #[nonvisiting] pub Option<ReturnAnnotation>,
    #[block(func)]
    #[block_mut]
    pub Box<Block>,
);
impl FuncBody {
    pub const ANONYMOUS: &str = "__anonymous";
    pub fn has_var_arg(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Param::Var(..)))
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
    Empty,

    #[tag(sugar)]
    Linq(Linq),
    #[tag(sugar)]
    Match(Match),

    VarArg,
    Literal(#[nonvisiting] ConstValue),
    Do(
        #[block(do_expr)]
        #[block_mut]
        Box<Block>,
    ),

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
    TypeLit(#[nonvisiting] TypeDesc),

    #[tag(sugar)]
    BangDo(BangDoNode),
    #[tag(sugar)]
    BangMacro(#[nonvisiting] BangMacroNode),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Visitor, VisitorMut)]
pub struct BangDoNode {
    pub context: Box<Expr>,
    pub body: Box<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BangMacroNode {
    pub name: String,
    pub tokens: Vec<Token>,
    pub span: Span,
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
pub struct TypeParam(pub Name, pub Option<TypeDesc>);

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

/// 在AST层面的对于类型的描述符, 供TypeEval使用
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
    TypeTable(Box<[(Box<str>, TypeDesc)]>),
    Function(Option<TypeFnValue>),
    FnLit(Box<FuncBody>),
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
    pub fn typetable_of(items: Box<[(Box<str>, TypeDesc)]>) -> TypeDesc {
        if items.iter().all(|(_, v)| v.is_pure()) {
            TypeDesc::Pure(Type::TypeTable(
                items
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            ConstValue::String(k.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(v.expect_pure().unwrap()),
                        )
                    })
                    .collect(),
            ))
        } else {
            TypeDesc::TypeTable(items)
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
                    .map(|(k, v)| format!("{k}: {v}"))
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
