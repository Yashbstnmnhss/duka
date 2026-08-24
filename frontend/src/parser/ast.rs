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
    ),
    While(
        Box<Expr>,
        #[block(loop_stmt)]
        #[block_mut]
        Box<Block>,
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
    TypeAlias(#[nonvisiting] Name, #[nonvisiting] Box<TypeDescriptor>),
    #[tag(typesys)]
    ///```ts
    /// type function F(a, b) -> type
    ///     return a | b
    /// end
    /// ```
    TypeFunction(#[nonvisiting] Name, #[nonvisiting] Box<FuncBody>),
    #[tag(typesys)]
    ///```ts
    /// type function inline List(t) = { head: t, tail: List(t)? }
    /// ```
    InlineTypeFunction(
        #[nonvisiting] Name,
        #[nonvisiting] Box<[Param]>,
        #[nonvisiting] Box<TypeDescriptor>,
    ),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub struct FuncBody(
    #[nonvisiting] pub Box<[Param]>,
    #[nonvisiting] pub Box<[TypeParam]>,
    #[nonvisiting] pub Option<TypeDescriptor>, // Return Type
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

/// guard mode
pub type Pattern = (PatternTerm, Option<Box<Expr>>);
#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PatternTerm {
    Custom(#[nonvisiting] Name, Box<[Expr]>, Box<[PatternTerm]>),
    /// `123`
    Constant(Box<Expr>),
    /// `local name: type`
    Bind(#[nonvisiting] Name, #[nonvisiting] Option<TypeDescriptor>),
    /// `|> func()`
    Call(Box<Expr>),
    /// `> 2`
    Compare(#[nonvisiting] BinOp, Box<Expr>),
    /// `{ 1, ..., 5, _, _, a = local var, [true] = |> func }`
    Table(Box<[FieldPattern]>),
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
        #[nonvisiting] Option<TypeDescriptor>,
    ),
    KeyValue(
        Box<Expr>,
        Option<Box<Expr>>,
        #[nonvisiting] Option<TypeDescriptor>,
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
    TypeLit(#[nonvisiting] TypeDescriptor),
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
pub type AttrName = Spanned<(Name, Attrs, Option<TypeDescriptor>)>;

pub fn get_attr(attrs: &Attrs, who: &str) -> Option<Box<[(Name, ConstValue)]>> {
    attrs
        .iter()
        .find_map(|i| (i.0.0 == who).then_some(i.1.clone()))
}
pub fn has_attr(attrs: &Attrs, who: &str) -> bool {
    attrs.iter().any(|(n, _)| n.0 == who)
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct TypeParam(pub Name, pub Option<TypeDescriptor>);

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Param {
    Var(Span),
    Name(Name),
    /// 带类型标注的参数
    Typed(Name, TypeDescriptor),
}

#[derive(Debug, PartialEq, Clone, Visitor, VisitorMut, Serialize, Deserialize)]
pub enum PathSuffix {
    /// `path.name`
    Dot(#[nonvisiting] Name),
    /// `path[expr]`
    Index(Box<Expr>),
    /// `path:name`
    Colon(#[nonvisiting] Name),
    TypeArgs(#[nonvisiting] Box<[TypeDescriptor]>, #[nonvisiting] Span),
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

/// 在AST层面的对于类型的描述符, 供TypeEval使用
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeDescriptor {
    Pure(Type),
    Named(Box<str>, Span),
    Generic {
        name: Box<str>,
        args: Box<[TypeDescriptor]>,
        span: Span,
    },
    TypeCall {
        name: Box<str>,
        args: Box<[TypeDescriptor]>,
        span: Span,
    },
    Access {
        base: Box<TypeDescriptor>,
        member: Box<TypeDescriptor>,
        args: Option<Box<[TypeDescriptor]>>,
        span: Span,
    },
    TypeOf {
        expr: Box<Expr>,
        span: Span,
    },
    Array(Option<Box<TypeDescriptor>>),
    Table(Option<Box<TypeDescriptor>>, Option<Box<TypeDescriptor>>),
    Union(Box<[TypeDescriptor]>),
    TypeTuple(Box<[TypeDescriptor]>),
    TypeTable(Box<[(Box<str>, TypeDescriptor)]>),
    Function(Option<TypeFnValue>),
    FnLit(Box<FuncBody>),
    NonNil(Box<TypeDescriptor>),
    Nilable(Box<TypeDescriptor>),
    Rec(Box<TypeDescriptor>),
}

impl Default for TypeDescriptor {
    fn default() -> Self {
        Self::Pure(Default::default())
    }
}
impl TypeDescriptor {
    pub fn base_type(&self) -> Option<&Type> {
        match self {
            TypeDescriptor::Pure(t) => Some(t),
            TypeDescriptor::NonNil(inner) | TypeDescriptor::Nilable(inner) => inner.base_type(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeFnValue {
    pub params: Box<[TypeDescriptor]>,
    pub var_arg: bool,
    pub returns: Box<[TypeDescriptor]>,
    pub return_var_arg: bool,
}

impl TypeDescriptor {
    pub fn is_pure(&self) -> bool {
        matches!(self, TypeDescriptor::Pure(_))
    }
    pub fn expect_pure(self) -> Option<Type> {
        match self {
            TypeDescriptor::Pure(t) => Some(t),
            _ => None,
        }
    }
    pub fn union(self, rhs: TypeDescriptor) -> TypeDescriptor {
        match (self, rhs) {
            (TypeDescriptor::Pure(a), TypeDescriptor::Pure(b)) => TypeDescriptor::Pure(a | b),
            (a, b) if a == b => a,
            (a, b) => TypeDescriptor::Union([a, b].into()),
        }
    }
    pub fn intersect(self, rhs: TypeDescriptor) -> TypeDescriptor {
        match (self, rhs) {
            (TypeDescriptor::Pure(a), TypeDescriptor::Pure(b)) => TypeDescriptor::Pure(a & b),
            _ => TypeDescriptor::Pure(Type::Never),
        }
    }
    pub fn nilable(self) -> TypeDescriptor {
        TypeDescriptor::Nilable(Box::new(self))
    }
    pub fn nonnilable(self) -> TypeDescriptor {
        TypeDescriptor::NonNil(Box::new(self))
    }
    pub fn array_of(elem: Option<TypeDescriptor>) -> TypeDescriptor {
        match elem {
            None => TypeDescriptor::Pure(Type::Array(None)),
            Some(TypeDescriptor::Pure(t)) => TypeDescriptor::Pure(Type::Array(Some(Box::new(t)))),
            Some(tv) => TypeDescriptor::Array(Some(Box::new(tv))),
        }
    }
    pub fn table_of(k: Option<TypeDescriptor>, v: Option<TypeDescriptor>) -> TypeDescriptor {
        match (&k, &v) {
            (None, None) => TypeDescriptor::Pure(Type::Table(None, None)),
            (Some(TypeDescriptor::Pure(k)), Some(TypeDescriptor::Pure(v))) => TypeDescriptor::Pure(
                Type::Table(Some(Box::new(k.clone())), Some(Box::new(v.clone()))),
            ),
            _ => TypeDescriptor::Table(k.map(Box::new), v.map(Box::new)),
        }
    }
    pub fn function_of(ft: Option<TypeFnValue>) -> TypeDescriptor {
        let Some(ft) = ft else {
            return TypeDescriptor::Pure(Type::Function(None));
        };
        if ft.params.iter().all(TypeDescriptor::is_pure)
            && ft.returns.iter().all(TypeDescriptor::is_pure)
        {
            TypeDescriptor::Pure(Type::Function(Some(FunctionType {
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
            TypeDescriptor::Function(Some(ft))
        }
    }
    pub fn tuple_of(items: Box<[TypeDescriptor]>) -> TypeDescriptor {
        if items.iter().all(TypeDescriptor::is_pure) {
            TypeDescriptor::Pure(Type::TypeTuple(
                items
                    .iter()
                    .map(|t| t.clone().expect_pure().unwrap())
                    .collect(),
            ))
        } else {
            TypeDescriptor::TypeTuple(items)
        }
    }
    pub fn typetable_of(items: Box<[(Box<str>, TypeDescriptor)]>) -> TypeDescriptor {
        if items.iter().all(|(_, v)| v.is_pure()) {
            TypeDescriptor::Pure(Type::TypeTable(
                items
                    .into_iter()
                    .map(|(k, v)| (k, Box::new(v.expect_pure().unwrap())))
                    .collect(),
            ))
        } else {
            TypeDescriptor::TypeTable(items)
        }
    }
}

impl Display for TypeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeDescriptor::Pure(t) => write!(f, "{t}"),
            TypeDescriptor::Named(name, _) => write!(f, "{name}"),
            TypeDescriptor::Generic { name, args, .. } => write!(
                f,
                "{name}<{}>",
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDescriptor::TypeCall { name, args, .. } => write!(
                f,
                "{name}({})",
                args.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDescriptor::Access {
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
            TypeDescriptor::TypeOf { .. } => write!(f, "type(...)"),
            TypeDescriptor::Array(inner) => match inner {
                Some(inner) => write!(f, "[{inner}]"),
                None => write!(f, "[]"),
            },
            TypeDescriptor::Table(k, v) => {
                let k = k.as_ref().map(ToString::to_string).unwrap_or_default();
                let v = v.as_ref().map(ToString::to_string).unwrap_or_default();
                write!(f, "table[{k}]({v})")
            }
            TypeDescriptor::Union(ts) => write!(
                f,
                "{}",
                ts.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
            TypeDescriptor::TypeTuple(ts) => write!(
                f,
                "({})",
                ts.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDescriptor::TypeTable(ts) => write!(
                f,
                "table[{}]",
                ts.iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeDescriptor::Function(ft) => match ft {
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
            TypeDescriptor::FnLit(_) => write!(f, "type fn"),
            TypeDescriptor::NonNil(inner) => write!(f, "{inner}!"),
            TypeDescriptor::Nilable(inner) => write!(f, "{inner}?"),
            TypeDescriptor::Rec(inner) => write!(f, "rec {inner}"),
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
