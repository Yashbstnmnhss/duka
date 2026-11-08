use std::{
    collections::HashMap,
    fmt::Display,
    ops::{BitAnd, BitOr},
    str::Chars,
};

use duka_macros::Info;
use serde::{Deserialize, Serialize};

use crate::{
    constants::ctype,
    effects::EffectSet,
    utils::{MultiPeekable, MultiPeekableExtension},
    value::{ConstValue, DukaFloat, DukaInt},
};

/// The container skeleton both contexts (value-context & type-context) are built from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default, Info)]
#[shy]
pub enum Ty<L> {
    /* Real Type */
    #[tag(literal)]
    Nil,
    #[tag(basic)]
    Bool,
    #[tag(basic)]
    #[tag(numeric)]
    Int,
    #[tag(basic)]
    #[tag(numeric)]
    Float,
    #[tag(basic)]
    String,
    #[tag(array_like)]
    Array(Option<Box<Ty<L>>>),
    #[tag(table_like)]
    /// key/value types, plus the metatable type `set_metatable` attached
    Table(Option<Box<Ty<L>>>, Option<Box<Ty<L>>>, Option<Box<Ty<L>>>),
    #[tag(table_like)]
    Object {
        id: ObjectId,
        name: Box<str>,
        base: Option<ObjectId>,
        args: Box<[Ty<L>]>,
        module: Option<Box<str>>,
        /// metatable attached to the class table itself, not to instances
        mt: Option<Box<Ty<L>>>,
    },
    // A pure function with pure types, see TypeDesc::Function for not pure types
    Function(Option<FunctionType>),
    /// A coroutine handle.
    ///
    /// `spawn` hands one back and `go` takes one, so it is a thing of its own
    /// rather than any of the things it carries.
    #[tag(basic)]
    Coroutine,
    /// A userdata value: a payload the runtime owns.
    ///
    /// The name is what tells one userdata from another -- a file is not a
    /// regular expression -- and an empty name is a userdata of any kind.
    UserData {
        name: Box<str>,
    },
    #[tag(inter)]
    TypeFn {
        id: usize,
        name: Box<str>,
    },
    /* Epyt Laer */

    /* Type Mode */
    /// 值层泛型占位(泛型函数/对象体内的类型参数)
    #[tag(inter)]
    Param(Box<str>),
    /// A type parameter of the enclosing signature, by index into its
    /// `type_vars`. Bound by that list, so -- unlike `Param` -- it is part of a
    /// final type rather than an evaluation intermediate.
    #[tag(inter)]
    Var(usize),
    #[tag(literal)]
    Literal(ConstValue),
    /// `[type, type, type]`: a known prefix and an optional repeated tail.
    /// `None` is a tuple of exactly its members; `Some(t)` is zero or more `t`
    /// after them, which is what `[...t]` writes.
    #[tag(array_like)]
    TypeTuple(Vec<Ty<L>>, Option<Box<Ty<L>>>),
    /// `{ literal: type }` key can be any ConstValue
    #[tag(table_like)]
    TypeTable(
        Vec<(ConstValue, Box<Ty<L>>)>,
        Option<Box<Ty<L>>>, /* metatable */
    ),
    Rec(Box<Ty<L>>),
    /// A nominal label on any type
    Brand(Box<Brand>),
    Pattern(Box<Pattern>),
    /* Edom Epyt */

    /* Special */
    #[default]
    #[tag(top)]
    Any,
    #[tag(top)]
    Unknown,
    #[tag(bottom)]
    Never,
    Union(Box<[Ty<L>]>),
    /// What this instantiation adds past the final world: the evaluator's own
    /// intermediates (closures, placeholders). The value context cannot inhabit
    /// it, so a `Type` can never hold one.
    Ext(L),
    /* Laiceps */
}

/// The value context has nothing past the final forms.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoExt {}

pub type Type = Ty<NoExt>;
/// Rewrites the named parameters of a signature into bound `Var` indices.
pub fn to_vars(t: &Type, names: &[&str]) -> Type {
    map_type(t, &mut |leaf| match leaf {
        Type::Param(name) => names.iter().position(|n| *n == &**name).map(Type::Var),
        _ => None,
    })
}

/// Rewrites every leaf a type contains, keeping the shape of its containers.
///
/// The answer for a node replaces it wholesale and stops the walk, so a node
/// that must not be reached into -- a `Rec` body, an already-decided result --
/// survives by answering `Some` for itself.
pub fn map_type(t: &Type, f: &mut impl FnMut(&Type) -> Option<Type>) -> Type {
    if let Some(replaced) = f(t) {
        return replaced;
    }
    match t {
        Type::Union(ts) => Type::Union(ts.iter().map(|x| map_type(x, f)).collect()),
        Type::TypeTuple(items, rest) => Type::TypeTuple(
            items.iter().map(|x| map_type(x, f)).collect(),
            rest.as_deref().map(|x| Box::new(map_type(x, f))),
        ),
        Type::Array(inner) => Type::Array(inner.as_deref().map(|x| Box::new(map_type(x, f)))),
        Type::Table(k, v, mt) => Type::Table(
            k.as_deref().map(|x| Box::new(map_type(x, f))),
            v.as_deref().map(|x| Box::new(map_type(x, f))),
            mt.as_deref().map(|x| Box::new(map_type(x, f))),
        ),
        Type::TypeTable(items, mt) => Type::TypeTable(
            items
                .iter()
                .map(|(k, v)| (k.clone(), Box::new(map_type(v, f))))
                .collect(),
            mt.as_deref().map(|x| Box::new(map_type(x, f))),
        ),
        Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
            params: ft.params.iter().map(|x| map_type(x, f)).collect(),
            var_arg: ft.var_arg,
            var_arg_bind: ft.var_arg_bind.clone(),
            returns: ft.returns.iter().map(|x| map_type(x, f)).collect(),
            return_var_arg: ft
                .return_var_arg
                .as_deref()
                .map(|x| Box::new(map_type(x, f))),
            return_var_arg_bind: ft.return_var_arg_bind.clone(),
            type_vars: ft
                .type_vars
                .iter()
                .map(|v| TypeVar {
                    name: v.name.clone(),
                    bound: v.bound.as_deref().map(|b| Box::new(map_type(b, f))),
                    default: v.default.as_deref().map(|d| Box::new(map_type(d, f))),
                    literal: v.literal,
                    pack: v.pack,
                })
                .collect(),
            sig: ft.sig,
            effects: ft.effects,
        })),
        Type::Rec(inner) => Type::Rec(Box::new(map_type(inner, f))),
        Type::Brand(b) => Type::Brand(Box::new(Brand {
            base: map_type(&b.base, f),
            label: b.label.clone(),
        })),
        Type::Pattern(p) => Type::Pattern(Box::new(Pattern {
            base: map_type(&p.base, f),
            pred: p.pred.clone(),
        })),
        Type::Object {
            id,
            name,
            base,
            args,
            module,
            mt,
        } => Type::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args.iter().map(|x| map_type(x, f)).collect(),
            module: module.clone(),
            mt: mt.as_deref().map(|x| Box::new(map_type(x, f))),
        },
        other => other.clone(),
    }
}
pub type ObjectId = usize;

/// A label a type carries. `base` is what the label sits on, so unwrapping one
/// leaves the type that was written underneath it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Brand {
    pub base: Type,
    pub label: Label,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Label {
    /// A dimension: a free abelian group over symbols, so `m * s` is `m·s`.
    Unit(Units),
    /// A name with no algebra behind it: `UserId * UserId` is an error.
    Nominal(Box<str>),
}

/// The exponent vector of a unit label, normalized: sorted by symbol, one entry
/// per symbol, no zero exponents. The empty vector is dimensionless, which is
/// the same type as carrying no label at all.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Units(pub Box<[(Box<str>, i32)]>);

/// A value constraint carried by a type. `base` is what it constrains, so the
/// two are always the same scalar.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pattern {
    pub base: Type,
    pub pred: Pred,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pred {
    Num(NumPred),
    Text(TextPred),
}

impl Display for Pred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pred::Num(num_pred) => write!(f, "{num_pred}"),
            Pred::Text(text_pred) => write!(f, "{text_pred}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CmpOp {
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}
impl Display for CmpOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                CmpOp::Gt => ">",
                CmpOp::Lt => "<",
                CmpOp::Ge => ">=",
                CmpOp::Le => "<=",
                CmpOp::Eq => "==",
                CmpOp::Ne => "~=",
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NumPred {
    Cmp(CmpOp, ConstValue),
    And(Box<[NumPred]>),
    Or(Box<[NumPred]>),
    Not(Box<NumPred>),
}

impl Display for NumPred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumPred::Cmp(op, cv) => write!(f, "{op} {cv}"),
            NumPred::And(ps) => write!(
                f,
                "all({})",
                ps.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NumPred::Or(ps) => write!(
                f,
                "any({})",
                ps.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NumPred::Not(ps) => write!(f, "not({ps})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextPred {
    Has(Box<str>),
    Starts(Box<str>),
    Ends(Box<str>),
    Len(CmpOp, DukaInt),
    Regex(Box<str>),
    And(Box<[TextPred]>),
    Or(Box<[TextPred]>),
    Not(Box<TextPred>),
}

impl Display for TextPred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextPred::And(ps) => write!(
                f,
                "all({})",
                ps.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TextPred::Or(ps) => write!(
                f,
                "any({})",
                ps.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TextPred::Not(ps) => write!(f, "not({ps})"),
            TextPred::Has(s) => write!(f, "has(\"{s}\")"),
            TextPred::Starts(s) => write!(f, "starts(\"{s}\")"),
            TextPred::Ends(s) => write!(f, "ends(\"{s}\")"),
            TextPred::Len(op, n) => write!(f, "# {op} {n}"),
            TextPred::Regex(reg) => write!(f, "regex(\"{reg}\")"),
        }
    }
}

fn as_num(value: &ConstValue) -> Option<DukaFloat> {
    match value {
        ConstValue::Int(i) => Some(*i as DukaFloat),
        ConstValue::Float(f) => Some(*f),
        _ => None,
    }
}

impl CmpOp {
    pub fn check(self, left: DukaFloat, right: DukaFloat) -> bool {
        match self {
            CmpOp::Gt => left > right,
            CmpOp::Lt => left < right,
            CmpOp::Ge => left >= right,
            CmpOp::Le => left <= right,
            CmpOp::Eq => left == right,
            CmpOp::Ne => left != right,
        }
    }
}

impl NumPred {
    pub fn check(&self, value: &ConstValue) -> bool {
        match self {
            NumPred::Cmp(op, rhs) => match (as_num(value), as_num(rhs)) {
                (Some(l), Some(r)) => op.check(l, r),
                _ => false,
            },
            NumPred::And(items) => items.iter().all(|i| i.check(value)),
            NumPred::Or(items) => items.iter().any(|i| i.check(value)),
            NumPred::Not(inner) => !inner.check(value),
        }
    }
}

impl TextPred {
    pub fn check(&self, text: &[u8]) -> bool {
        let s = std::str::from_utf8(text).unwrap_or_default();
        match self {
            TextPred::Has(part) => s.contains(&**part),
            TextPred::Starts(part) => s.starts_with(&**part),
            TextPred::Ends(part) => s.ends_with(&**part),
            TextPred::Len(op, n) => op.check(s.chars().count() as f64, *n as f64),
            TextPred::Regex(source) => match crate::regex::compile(source) {
                Ok(compiled) => crate::regex::match_str(&compiled, s)
                    .is_some_and(|m| m.start == 0 && m.end == s.len()),
                Err(_) => false,
            },
            TextPred::And(items) => items.iter().all(|i| i.check(text)),
            TextPred::Or(items) => items.iter().any(|i| i.check(text)),
            TextPred::Not(inner) => !inner.check(text),
        }
    }
}

impl Pattern {
    pub fn check(&self, value: &ConstValue) -> bool {
        match (&self.pred, value) {
            (Pred::Num(pred), _) => pred.check(value),
            (Pred::Text(pred), ConstValue::String(text)) => pred.check(text),
            _ => false,
        }
    }
}

impl Units {
    /// The label `m/s^2` writes. Symbols keep their spelling; only the
    /// exponents are ordered, so two spellings of one dimension compare equal.
    pub fn parse(source: &str) -> Result<Self, String> {
        struct Parser<'a> {
            chars: MultiPeekable<Chars<'a>>,
        }
        impl<'a> Parser<'a> {
            fn new(source: &'a str) -> Self {
                Self {
                    chars: source.chars().multi_peekable(),
                }
            }

            #[inline]
            fn parse(mut self) -> Result<Units, String> {
                let items = self.expr()?;
                if self.next().is_some() {
                    return Err("Invalid input at the tail of unit".to_owned());
                }
                Ok(Units::normalized(items))
            }
            fn normalized(
                base: &mut Vec<(Box<str>, i32)>,
                other: Vec<(Box<str>, i32)>,
                factor: i32,
            ) {
                for (key, pow) in other {
                    if let Some(idx) = base.iter().position(|i| i.0 == key) {
                        base[idx].1 += pow * factor
                    } else {
                        base.push((key, pow * factor))
                    }
                }
            }

            fn peek(&mut self) -> Option<char> {
                while let Some(ch) = self.chars.peek_nth(0) {
                    if ch.is_ascii_whitespace() {
                        self.chars.next();
                        continue;
                    }
                    return Some(*ch);
                }
                None
            }
            fn next(&mut self) -> Option<char> {
                for ch in self.chars.by_ref() {
                    if ch.is_ascii_whitespace() {
                        continue;
                    }
                    return Some(ch);
                }
                None
            }

            fn expr(&mut self) -> Result<Vec<(Box<str>, i32)>, String> {
                let Some(mut base) = self.item()? else {
                    return Ok(vec![]);
                };

                while let Some(op) = self.peek() {
                    let factor = match op {
                        '/' => -1,
                        '*' | '·' => 1,
                        _ => return Ok(base),
                    };
                    self.next();
                    let Some(next) = self.item()? else {
                        return Err(format!("Expected operands after operator {op}"));
                    };

                    Self::normalized(&mut base, next, factor);
                }

                Ok(base)
            }
            fn item(&mut self) -> Result<Option<Vec<(Box<str>, i32)>>, String> {
                let Some(ch) = self.peek() else {
                    return Ok(None);
                };
                let results = match ch {
                    // `1` is the identity of the group: `m/1` is `m`, and `1`
                    // alone is nothing
                    '1' => {
                        self.next();
                        vec![]
                    }
                    '(' => {
                        self.next();

                        let inner = self.expr()?;

                        let Some(')') = self.next() else {
                            return Err("Unfinished \"(\" in unit".to_owned());
                        };

                        inner
                    }
                    ch if ch.is_alphabetic() => {
                        let mut res = String::new();
                        res.push(ch);
                        self.next();

                        while let Some(ch) = self.peek()
                            && ch.is_alphanumeric()
                        {
                            res.push(ch);
                            self.next();
                        }

                        vec![(res.into_boxed_str(), 1)]
                    }
                    who => return Err(format!("Invalid character in unit: {who}")),
                };

                let power = if let Some('^') = self.peek() {
                    self.next();

                    if let Some(head) = self.next()
                        && (head.is_ascii_digit() || head == '-')
                    {
                        let mut buffer = String::new();
                        buffer.push(head);

                        while let Some(ch) = self.peek()
                            && ch.is_ascii_digit()
                        {
                            buffer.push(ch);
                            self.next();
                        }

                        if buffer == "-" {
                            return Err("Invalid integer with only \"-\"".to_owned());
                        }

                        match buffer.parse::<i32>() {
                            Ok(i) => i,
                            Err(e) => {
                                return Err(format!("Invalid integer \"{buffer}\": {e}"));
                            }
                        }
                    } else {
                        return Err("Expected integer after \"^\"".to_owned());
                    }
                } else {
                    1
                };

                Ok(Some(
                    results.into_iter().map(|i| (i.0, i.1 * power)).collect(),
                ))
            }
        }

        Parser::new(source).parse()
    }

    pub fn normalized(mut items: Vec<(Box<str>, i32)>) -> Self {
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out: Vec<(Box<str>, i32)> = Vec::with_capacity(items.len());
        for (name, exp) in items {
            match out.last_mut() {
                Some(last) if last.0 == name => last.1 += exp,
                _ => out.push((name, exp)),
            }
        }
        out.retain(|(_, exp)| *exp != 0);
        Units(out.into_boxed_slice())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn mul(&self, other: &Self) -> Self {
        let mut items: Vec<(Box<str>, i32)> = self.0.to_vec();
        items.extend(other.0.iter().cloned());
        Self::normalized(items)
    }

    pub fn div(&self, other: &Self) -> Self {
        let mut items: Vec<(Box<str>, i32)> = self.0.to_vec();
        items.extend(other.0.iter().map(|(name, exp)| (name.clone(), -exp)));
        Self::normalized(items)
    }

    pub fn pow(&self, n: i32) -> Self {
        Self::normalized(
            self.0
                .iter()
                .map(|(name, exp)| (name.clone(), exp * n))
                .collect(),
        )
    }
}

impl Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Label::Unit(units) => write!(f, "<{units}>"),
            Label::Nominal(name) => write!(f, "::{name}::"),
        }
    }
}

impl Display for Units {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return write!(f, "1");
        }
        let mut num: Vec<String> = Vec::new();
        let mut den: Vec<String> = Vec::new();
        for (name, exp) in self.0.iter() {
            let exp = *exp;
            let side = if exp > 0 { &mut num } else { &mut den };
            let exp = exp.abs();
            side.push(if exp == 1 {
                name.to_string()
            } else {
                format!("{name}^{exp}")
            });
        }
        if num.is_empty() {
            num.push("1".to_owned());
        }
        write!(f, "{}", num.join("·"))?;
        if !den.is_empty() {
            write!(f, "/{}", den.join("·"))?;
        }
        Ok(())
    }
}

pub const REC_PARAM_PREFIX: &str = "__rec_";
#[inline]
pub fn rec_marker(name: &str) -> Box<str> {
    format!("{REC_PARAM_PREFIX}{name}").into_boxed_str()
}
/// The name a reader should see: the marker is an implementation detail of the
/// recursion, never part of the type as written.
#[inline]
pub fn rec_name(marker: &str) -> &str {
    marker.strip_prefix(REC_PARAM_PREFIX).unwrap_or(marker)
}

/// A type parameter a signature declares, the `T` of `as.<T>(v: any): T`.
///
/// It travels with the signature instead of sitting beside it, so that a call
/// site can solve for it however the function was reached: written in a file,
/// or handed over by the runtime as a builtin. A builtin that lives in a module
/// has no name a call site can look it up by, and two modules may have a member
/// of the same name, so the signature is the only place the variables can be.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TypeVar {
    pub name: Box<str>,
    /// `T: bound`, what the variable has to accept once it is filled in
    pub bound: Option<Box<Type>>,
    /// `T = default`, used when nothing else determines the variable
    pub default: Option<Box<Type>>,
    /// `literal T`: the variable wants what the argument *is*, not the type the
    /// argument widens to
    pub literal: bool,
    /// `...Ts`: the variable takes the rest of the type arguments as a list
    pub pack: bool,
}

impl Display for TypeVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.pack {
            write!(f, "...")?;
        }
        if self.literal {
            write!(f, "const ")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(b) = &self.bound {
            write!(f, ": {b}")?;
        }
        if let Some(d) = &self.default {
            write!(f, "= {d}")?;
        }
        Ok(())
    }
}

/// A wider member that really does stand for the narrower one: a scalar base
/// stands for its literals, and anything stands for `never`. Tables, objects
/// and type parameters are left alone: `accepts` says a value *fits*, not that
/// the two describe the same thing, and a metatable or a class id is part of
/// what the narrower one says.
fn folds_into(wider: &Type, narrower: &Type) -> bool {
    if matches!(narrower, Type::Brand(_) | Type::Pattern(_)) {
        return false;
    }
    matches!(
        wider,
        Type::Any
            | Type::Unknown
            | Type::Never
            | Type::Int
            | Type::Float
            | Type::String
            | Type::Bool
            | Type::Nil
    ) || matches!(narrower, Type::Literal(_))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FunctionType {
    pub params: Box<[Type]>,
    pub var_arg: bool,
    /// The name a rest parameter collects its types into, `...: Ts`
    pub var_arg_bind: Option<Box<str>>,
    pub returns: Box<[Type]>,
    /// The type the returned values go on with after `returns`, `Some(t)` for
    /// zero or more `t`; `None` means the list is exactly as long as it says.
    pub return_var_arg: Option<Box<Type>>,
    /// The name the open tail collects into, `fn() -> ...Rs`; the tail is what
    /// sits past the declared slots, so `-> (bool, ...Rs)` names after `bool`
    pub return_var_arg_bind: Option<Box<str>>,
    /// The type parameters this signature declares, empty for a plain one
    pub type_vars: Box<[TypeVar]>,
    /// The declaration beside this signature: its `where` list, postcondition and
    /// returns as written, in the analysis this signature came from
    pub sig: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<EffectSet>,
}
impl FunctionType {
    /// This signature with its bound variables written as names.
    ///
    /// `Var` is the stored, closed form; the solver and the rest of the checker
    /// pair parameters against arguments by name, so they read this view.
    pub fn param_view(&self) -> FunctionType {
        let to_param = |t: &Type| {
            map_type(t, &mut |leaf| match leaf {
                Type::Var(i) => self.type_vars.get(*i).map(|v| Type::Param(v.name.clone())),
                _ => None,
            })
        };
        FunctionType {
            params: self.params.iter().map(to_param).collect(),
            returns: self.returns.iter().map(to_param).collect(),
            return_var_arg: self
                .return_var_arg
                .as_deref()
                .map(|t| Box::new(to_param(t))),
            type_vars: self
                .type_vars
                .iter()
                .map(|v| TypeVar {
                    name: v.name.clone(),
                    bound: v.bound.as_deref().map(|b| Box::new(to_param(b))),
                    default: v.default.as_deref().map(|d| Box::new(to_param(d))),
                    literal: v.literal,
                    pack: v.pack,
                })
                .collect(),
            ..self.clone()
        }
    }

    pub fn accepts(&self, other: &FunctionType) -> bool {
        // the pre-strict direction on parameters: the wanted signature's
        // parameter types take the given one's, which reads the pair the wrong
        // way round but is what `strict: false` asks for
        self.params
            .iter()
            .zip(other.params.iter())
            .all(|(a, b)| a.accepts(b))
            && self.returns_match(other)
            && self.effects_match(other)
    }

    /// The direction a call runs in: a signature can stand where another is
    /// wanted when it takes every argument the wanted one could be given, so
    /// the *given* signature's parameters have to accept the *wanted* one's.
    /// Returns keep their ordinary direction -- what comes back flows out.
    pub fn accepts_strict(&self, other: &FunctionType) -> bool {
        other
            .params
            .iter()
            .zip(self.params.iter())
            .all(|(given, wanted)| given.accepts(wanted))
            && self.returns_match(other)
            && self.effects_match(other)
    }

    pub fn effects_match(&self, other: &FunctionType) -> bool {
        match (&self.effects, &other.effects) {
            (None, _) | (_, None) => true,
            (Some(target), Some(value)) => target.accepts(value),
        }
    }

    /// Parameters use a loose arity (zip); returns are validated item-by-item.
    /// - declaring fn is var-arg returns => accepts
    /// - actual fn has no return annotation => accepts (unknown / no void)
    /// - missing actual returns are filled with `nil`
    /// - an actual var-arg return covers the remainder with `any`
    fn returns_match(&self, other: &FunctionType) -> bool {
        if self.return_var_arg.is_some() {
            return true;
        }
        if other.returns.is_empty() && other.return_var_arg.is_none() {
            return true;
        }
        for (idx, declared) in self.returns.iter().enumerate() {
            let actual = match other.returns.get(idx) {
                Some(t) => t.clone(),
                // a tail may stop before this slot, so the value is absent then
                None => match &other.return_var_arg {
                    Some(t) => (**t).clone() | Type::Nil,
                    None => Type::Nil,
                },
            };
            if !declared.accepts(&actual) {
                return false;
            }
        }
        true
    }
}

impl Display for FunctionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ft = self.param_view();
        let generics = ft
            .type_vars
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>();
        let mut params: Vec<String> = ft.params.iter().map(|i| i.to_string()).collect();
        if ft.var_arg {
            params.push(match &ft.var_arg_bind {
                Some(name) => format!("...: {name}"),
                None => "...".to_owned(),
            });
        }
        let mut returns: Vec<String> = ft.returns.iter().map(|i| i.to_string()).collect();
        if let Some(tail) = &ft.return_var_arg {
            returns.push(match &ft.return_var_arg_bind {
                Some(name) => format!("...{name}"),
                None if **tail == Type::Any => "...".to_owned(),
                None => format!("...{tail}"),
            });
        }
        write!(
            f,
            "{}{}({}){}{}{}",
            ctype::FUN,
            if generics.is_empty() {
                "".to_owned()
            } else {
                format!("<{}>", generics.join(", "))
            },
            params.join(", "),
            if returns.is_empty() { "" } else { " -> " },
            if returns.len() == 1 {
                returns.pop().unwrap() // Ensured
            } else if !returns.is_empty() {
                format!("({})", returns.join(", "))
            } else {
                "".to_owned()
            },
            if let Some(e) = self.effects {
                format!(" {e}")
            } else {
                "".to_owned()
            }
        )
    }
}

/// A record is a table with literal keys, and it nests. The flat one-tab-per
/// field rendering turned `{ a: { b: int } }` into a staircase, so the indent
/// follows the depth.
fn render_table<L: Display + PartialEq>(
    items: &[(ConstValue, Box<Ty<L>>)],
    depth: usize,
) -> String {
    let pad = "    ".repeat(depth + 1);
    let close = "    ".repeat(depth);
    let body = items
        .iter()
        .map(|(key, value)| {
            let key = match key {
                ConstValue::String(s) => String::from_utf8_lossy(s).into_owned(),
                other => other.to_string(),
            };
            format!("{pad}{key}: {}", render_at(value, depth + 1))
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n{body}\n{close}}}")
}

fn render_at<L: Display + PartialEq>(ty: &Ty<L>, depth: usize) -> String {
    match ty {
        Ty::TypeTable(items, _) => render_table(items, depth),
        other => other.to_string(),
    }
}

/// The rendered type, plus its metatable when it carries one. A type with no
/// metatable renders exactly as it did before the slot existed.
fn with_mt<L: Display + PartialEq>(rendered: String, mt: &Option<Box<Ty<L>>>) -> String {
    match mt {
        Some(m) => format!("{rendered} + metatable {m}"),
        None => rendered,
    }
}

impl Display for NoExt {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl<L: Display + PartialEq> Display for Ty<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Ty::TypeTable(t, mt) => with_mt(render_table(t, 0), mt),
                Ty::Rec(inner) => format!("(recursive) {inner}"),
                Ty::TypeTuple(t, rest) => {
                    let head = t
                        .iter()
                        .map(|o| o.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let suffix = if rest.is_some() && !t.is_empty() {
                        ", "
                    } else {
                        ""
                    };
                    let tail = match rest {
                        None => String::new(),
                        Some(t) if **t == Ty::Any => "...".to_owned(),
                        Some(t) => format!("...{t}"),
                    };
                    format!("[{head}{suffix}{tail}]")
                }
                Ty::Array(inner) =>
                    if let Some(inner) = inner {
                        format!("{}<{}>", ctype::ARR, inner)
                    } else {
                        ctype::ARR.to_owned()
                    },
                Ty::Nil => ctype::NIL.to_owned(),
                Ty::Bool => ctype::BOO.to_owned(),
                Ty::Int => ctype::INT.to_owned(),
                Ty::Float => ctype::FLO.to_owned(),
                Ty::String => ctype::STR.to_owned(),
                Ty::Table(k, v, mt) => with_mt(
                    if k.is_some() || v.is_some() {
                        format!(
                            "{}<{}, {}>",
                            ctype::TAB,
                            k.as_deref().unwrap_or(&Ty::Any),
                            v.as_deref().unwrap_or(&Ty::Any)
                        )
                    } else {
                        ctype::TAB.to_owned()
                    },
                    mt,
                ),
                // the name is what a reader can be shown; the id is the
                // implementation's handle on the body and means nothing written
                Ty::TypeFn { name, .. } => format!("type function {name}"),
                Ty::Object { name, args, mt, .. } => with_mt(
                    if args.is_empty() {
                        name.to_string()
                    } else {
                        format!(
                            "{}<{}>",
                            name,
                            args.iter()
                                .map(|a| a.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    },
                    mt,
                ),
                Ty::Param(name) => rec_name(name).to_string(),
                Ty::Var(index) => format!("?{index}"),
                Ty::Literal(v) => match v {
                    ConstValue::String(s) => {
                        let c = str::from_utf8(s).unwrap_or("?");
                        format!("\"{c}\"")
                    }
                    other => other.to_string(),
                },
                Ty::Function(ft) =>
                    if let Some(ft) = ft {
                        ft.to_string()
                    } else {
                        ctype::FUN.to_owned()
                    },
                Ty::Any => ctype::ANY.to_owned(),
                Ty::Unknown => ctype::UNK.to_owned(),
                Ty::Coroutine => ctype::COR.to_owned(),
                Ty::UserData { name } => {
                    if name.is_empty() {
                        "userdata".to_owned()
                    } else {
                        name.to_string()
                    }
                }
                Ty::Union(items) => {
                    let render = |i: &Ty<L>| {
                        if matches!(i, Ty::Function(..) | Ty::Union(..) | Ty::Brand(..)) {
                            format!("({i})")
                        } else {
                            i.to_string()
                        }
                    };
                    // `T | nil` is what a default-nilable annotation means, and
                    // the language already spells that `T?`, so show that
                    let rest: Vec<&Ty<L>> =
                        items.iter().filter(|i| !matches!(i, Ty::Nil)).collect();
                    if rest.len() == 1 && rest.len() < items.len() {
                        format!("{}?", render(rest[0]))
                    } else {
                        items.iter().map(render).collect::<Vec<_>>().join(" | ")
                    }
                }
                Ty::Never => ctype::NEV.to_owned(),
                Ty::Brand(b) => format!("{} {}", b.base, b.label),
                Ty::Pattern(p) => format!("{}*", p.base),
                Ty::Ext(ext) => format!("{ext}"),
            }
        )
    }
}
impl Type {
    #[inline]
    pub fn base(&self) -> &Type {
        let mut ty = self;
        loop {
            match ty {
                Type::Brand(b) => ty = &b.base,
                Type::Pattern(p) => ty = &p.base,
                _ => return ty,
            }
        }
    }

    #[inline(always)]
    pub fn is_branded(&self) -> bool {
        matches!(self, Type::Brand(_))
    }
    #[inline]
    pub fn brand(&self) -> Option<&Brand> {
        match self {
            Type::Brand(b) => Some(b),
            _ => None,
        }
    }
    #[inline]
    pub fn pattern(&self) -> Option<&Pattern> {
        match self {
            Type::Pattern(p) => Some(p),
            _ => None,
        }
    }

    /// The type of a named field, if this is a record that has one.
    ///
    /// Read here rather than off the rendered text: a record renders one field
    /// per line, and taking that back apart is how a member lookup ends up wrong
    /// about a field whose type happens to contain a brace.
    pub fn field(&self, name: &str) -> Option<Type> {
        match self.base() {
            Type::TypeTable(fields, _) => fields.iter().find_map(|(key, value)| match key {
                ConstValue::String(s) if s.as_ref() == name.as_bytes() => Some((**value).clone()),
                _ => None,
            }),
            _ => None,
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Type> {
        Some(match keyword {
            ctype::INT | "integer" => Type::Int,
            ctype::FLO | "num" | "number" => Type::Float,
            "str" | ctype::STR => Type::String,
            ctype::BOO | "boolean" => Type::Bool,
            ctype::TAB => Type::Table(None, None, None),
            ctype::ARR | "list" => Type::Array(None),
            "func" | ctype::FUN | "fn" => Type::Function(None),
            ctype::NIL => Type::Nil,
            ctype::ANY => Type::Any,
            ctype::UNK => Type::Unknown,
            ctype::NEV => Type::Never,
            ctype::COR => Type::Coroutine,
            // a userdata of any kind: the name of one is declared by the
            // runtime, and a program that does not know it says "userdata"
            "userdata" => Type::UserData { name: "".into() },
            _ => return None,
        })
    }
    /// Optional types are a union with `nil`, `nonnilable` strips every `nil`
    /// member while `nilable` puts it back, `|` on two types builds a union
    ///
    /// ```
    /// use duka_shared::dtype::Type;
    ///
    /// let maybe = Type::Int | Type::Nil;
    /// assert_eq!(maybe.clone().nonnilable(), Type::Int);
    /// assert_eq!(Type::Int.nilable(), maybe);
    /// assert_eq!(maybe.clone().nilable(), maybe); // already optional
    ///
    /// // nil was all there was, so the type collapses to `Any`
    /// assert_eq!(Type::Nil.nonnilable(), Type::Any);
    /// ```
    pub fn nonnilable(self) -> Self {
        self.into_vec_non_nil()
            .into_iter()
            .reduce(Type::bitor)
            .unwrap_or(Type::Any)
    }
    /// 展开为成员列表并去掉所有 nil(递归把嵌套 union 展平)
    fn into_vec_non_nil(self) -> Vec<Type> {
        match self {
            Type::Nil => vec![],
            Type::Union(ts) => ts
                .into_vec()
                .into_iter()
                .flat_map(Type::into_vec_non_nil)
                .collect(),
            other => vec![other],
        }
    }
    pub fn nilable(self) -> Self {
        self | Type::Nil
    }
    pub fn is_var_arg(&self) -> bool {
        match self {
            Type::Function(Some(ft)) => ft.return_var_arg.is_some(),
            _ => false,
        }
    }
    /// Assignability, `self` is the declared type and `actual` what goes into it
    ///
    /// - `Int` takes an int literal, a string literal is rejected
    /// - `Float` widens from `Int`, so an int literal passes there too
    /// - `Any` takes everything, `Never` takes nothing
    ///
    /// ```
    /// use duka_shared::dtype::Type;
    /// use duka_shared::value::ConstValue;
    ///
    /// let int_lit = Type::Literal(ConstValue::Int(1));
    /// let str_lit = Type::Literal(ConstValue::String(b"s".to_vec().into()));
    ///
    /// assert!(Type::Int.accepts(&int_lit));
    /// assert!(!Type::Int.accepts(&str_lit));
    /// assert!(Type::Float.accepts(&int_lit));
    /// assert!(!Type::Int.accepts(&Type::Float));
    /// assert!(Type::Any.accepts(&str_lit));
    /// assert!(!Type::Never.accepts(&int_lit));
    /// ```
    pub fn accepts(&self, actual: &Type) -> bool {
        match self {
            Type::Brand(b) => {
                return match actual {
                    Type::Any => true,
                    Type::Never => false,
                    Type::Brand(a) => b.label == a.label && b.base.accepts(&a.base),
                    _ => false,
                };
            }
            Type::Pattern(p) => return p.base.accepts(actual.base()),
            _ => {}
        }
        if matches!(actual, Type::Brand(_) | Type::Pattern(_)) {
            return self.accepts(actual.base());
        }
        // `unknown` is the top: it takes every value and gives nothing back
        if matches!(self, Type::Unknown) {
            return true;
        }
        if matches!(actual, Type::Unknown) {
            return matches!(self, Type::Any);
        }
        // `never` is the bottom: it takes nothing, whatever the value is
        if matches!(self, Type::Never) {
            return false;
        }
        // `any` is the escape hatch, and it runs both ways: a value whose type
        // was written `any` goes where anything is wanted. `unknown` above is
        // the one that does not -- that is the difference between them.
        if matches!(actual, Type::Any) {
            return true;
        }
        match self {
            Type::Never => false,
            Type::Any => true,
            Type::Function(None) => matches!(actual, Type::Function(..) | Type::Any),
            Type::Function(Some(ft)) => match actual {
                Type::Function(None) | Type::Any => true,
                Type::Function(Some(ft2)) => ft.accepts(ft2),
                _ => false,
            },
            Type::Int if matches!(actual, Type::Literal(ConstValue::Int(..))) => true,
            Type::String if matches!(actual, Type::Literal(ConstValue::String(..))) => true,
            Type::Bool if matches!(actual, Type::Literal(ConstValue::Bool(..))) => true,
            Type::Nil if matches!(actual, Type::Literal(ConstValue::Nil)) => true,
            Type::Float => matches!(
                actual,
                Type::Int
                    | Type::Float
                    | Type::Any
                    | Type::Literal(ConstValue::Float(..))
                    | Type::Literal(ConstValue::Int(..))
            ),
            Type::Array(inner) => match actual {
                Type::Array(a) => match (inner, a) {
                    (None, _) => true,
                    (Some(ai), None) => ai.accepts(&Type::Any),
                    (Some(ai), Some(aa)) => ai.accepts(aa),
                },
                Type::TypeTuple(tys, rest) => {
                    let fits = |t: &Type| inner.as_ref().is_none_or(|ty| ty.accepts(t));
                    tys.iter().all(fits) && rest.as_ref().is_none_or(|t| fits(t))
                }
                _ => *actual == Type::Any,
            },
            Type::Table(k, v, mt) => match actual {
                Type::Table(ak, av, amt) => {
                    let k_ok = match (k, ak) {
                        (None, _) => true,
                        (Some(ki), None) => ki.accepts(&Type::Any),
                        (Some(ki), Some(ak)) => ki.accepts(ak),
                    };
                    let v_ok = match (v, av) {
                        (None, _) => true,
                        (Some(vi), None) => vi.accepts(&Type::Any),
                        (Some(vi), Some(av)) => vi.accepts(av),
                    };
                    let mt_ok = match (mt.as_deref(), amt.as_deref()) {
                        (None, _) => true,
                        (Some(expected), Some(actual)) => expected == actual,
                        (Some(_), None) => false,
                    };
                    k_ok && v_ok && mt_ok
                }
                Type::Object { .. } => {
                    k.as_deref().is_none_or(|v| matches!(v, Type::Any))
                        && v.as_deref().is_none_or(|v| matches!(v, Type::Any))
                }
                Type::TypeTable(..) => {
                    k.as_deref().is_none_or(|k| k.accepts(&Type::String))
                        && v.as_deref().is_none_or(|v| matches!(v, Type::Any))
                }
                _ => *actual == Type::Any,
            },
            Type::Param(name) => {
                matches!(actual, Type::Param(n) if n == name) || *actual == Type::Any
            }
            Type::Var(index) => {
                matches!(actual, Type::Var(i) if i == index) || *actual == Type::Any
            }
            Type::Literal(lv) => {
                matches!(actual, Type::Literal(av) if lv == av) || *actual == Type::Any
            }
            Type::Object {
                id,
                base,
                args,
                module,
                ..
            } => match actual {
                Type::Any => true,
                Type::Object {
                    id: aid,
                    base: abase,
                    args: aargs,
                    module: amodule,
                    ..
                } => {
                    if module == amodule && id == aid {
                        args.iter().zip(aargs.iter()).all(|(a, b)| a.accepts(b))
                    } else if module == amodule {
                        // A base is acceptable where a derived class is wanted,
                        // but only within one table: the chain is a walk through
                        // that table's own `base` links and there is no table here
                        // to walk, so a class declared in another module is only
                        // ever the same class as itself.
                        matches!(abase, Some(b) if b == id) || matches!(base, Some(b) if b == aid)
                    } else {
                        false
                    }
                }
                _ => false,
            },
            // Two type functions are the same type function when they are the
            // same interned body, and there is no subtyping between them: one
            // does not stand in for another, and neither stands in for a
            // function type, because applying one is not calling the other.
            Type::TypeFn { id, .. } => {
                matches!(actual, Type::TypeFn { id: aid, .. } if aid == id)
            }
            Type::TypeTable(fields, _) => match actual {
                Type::TypeTable(af, _) => fields.iter().all(|(k, dv)| {
                    af.iter()
                        .find(|(ak, _)| ak == k)
                        .is_some_and(|(_, av)| dv.accepts(av))
                        || dv.accepts(&Type::Nil)
                }),
                // `{}` is a record with nothing to check: every table is one
                Type::Table(..) if fields.is_empty() => true,
                _ => *actual == Type::Any,
            },
            Type::TypeTuple(items, rest) => match actual {
                Type::Array(Some(inner)) => {
                    let members: &[Type] = match &**inner {
                        Type::Union(us) => us,
                        other => std::slice::from_ref(other),
                    };
                    members.iter().all(|m| items.iter().any(|i| i.accepts(m)))
                }
                Type::Array(None) => true,
                Type::TypeTuple(items2, rest2) => {
                    // The expected prefix has to be there: a tail means the
                    // sequence may stop early, so it cannot fill a fixed slot.
                    let prefix = items.len() <= items2.len()
                        && items.iter().zip(items2).all(|(a, b)| a.accepts(b));
                    let tail = match rest {
                        // A closed tuple is exactly as long as it says.
                        None => items2.len() == items.len() && rest2.is_none(),
                        // Every slot past the prefix and the actual tail, if
                        // any, are elements of the expected tail.
                        Some(e) => {
                            items2.iter().skip(items.len()).all(|t| e.accepts(t))
                                && rest2.as_deref().is_none_or(|t| e.accepts(t))
                        }
                    };
                    prefix && tail
                }
                _ => *actual == Type::Any,
            },
            // A coroutine is a coroutine: nothing else stands in for one, and
            // one stands in for nothing else.
            Type::Coroutine => matches!(actual, Type::Coroutine | Type::Any),
            // A userdata is the userdata it names. An unnamed one is a userdata
            // of any kind, and takes any of them.
            Type::UserData { name } => match actual {
                Type::Any => true,
                Type::UserData { name: other } => name.is_empty() || *name == *other,
                _ => false,
            },
            Type::Union(u) => match actual {
                Type::Any => true,
                Type::Union(u2) => u2
                    .iter()
                    .all(|i| u.contains(i) || u.iter().any(|v| v.accepts(i))),
                c => u.contains(c) || u.iter().any(|v| v.accepts(c)),
            },
            Type::Rec(inner) => rec_accepts(inner, actual),
            _ => actual == self || matches!(actual, Type::Any | Type::Never),
        }
    }

    /// Like `accepts`, but Literal members additionally compare against a
    /// compile-time constant (so `local f: Flag = "read"` works while a
    /// runtime string value is rejected).
    pub fn accepts_value(&self, actual: &Type, cv: Option<&ConstValue>) -> bool {
        match self {
            Type::Pattern(p) => {
                return match cv {
                    Some(value) => p.check(value),
                    None => p.base.accepts_value(actual.base(), None),
                };
            }
            Type::Brand(b) => {
                return match actual {
                    Type::Brand(a) => b.label == a.label && b.base.accepts_value(&a.base, cv),
                    Type::Any => true,
                    // a literal is dimensionless, so it is any unit already:
                    // `1.0` in a place that wants `m` is `1.0 m`. A nominal
                    // label has no identity element, so a constant cannot
                    // invent one -- that is what `as` is for.
                    _ => cv.is_some_and(|value| match b.label {
                        Label::Unit(_) => b.base.accepts(&Type::Literal(value.clone())),
                        Label::Nominal(_) => false,
                    }),
                };
            }
            _ => {}
        }
        if matches!(self, Type::Unknown) {
            return true;
        }
        if matches!(actual, Type::Unknown) {
            return matches!(self, Type::Any);
        }
        match self {
            Type::Literal(lv) => match cv {
                Some(cv) => lv == cv,
                // A value whose type is this literal says as much as the value
                // itself would, and a value can carry that type without being
                // written as a literal: `as_const(1)` is a call, not a literal,
                // and it is still `1`.
                None => self.accepts(actual),
            },
            Type::Union(u) => match actual {
                Type::Any => true,
                Type::Union(u2) => u2
                    .iter()
                    .all(|i| u.contains(i) || u.iter().any(|m| m.accepts_value(i, cv))),
                c => u.contains(c) || u.iter().any(|m| m.accepts_value(c, cv)),
            },
            Type::Rec(inner) => {
                let mut visited = std::collections::HashSet::new();
                rec_bisim(inner, actual, &mut visited)
            }
            Type::Any => true,
            Type::Never => false,
            Type::Nil => matches!(actual, Type::Nil | Type::Any),
            other => other.accepts(actual),
        }
    }
}

// 交集
impl BitAnd for Type {
    type Output = Type;
    fn bitand(self, rhs: Self) -> Self::Output {
        if self == rhs {
            return self;
        }
        match (self, rhs) {
            (Type::Literal(ConstValue::Bool(b)), Type::Literal(ConstValue::Bool(b2)))
                if b != b2 =>
            {
                Type::Bool
            }
            (Type::Never, _) | (_, Type::Never) => Type::Never,
            (Type::Any, _) | (_, Type::Any) => Type::Any,
            (Type::TypeTable(a, _), Type::TypeTable(b, _)) => {
                let mut items = HashMap::new();
                for (k, v) in a.into_iter().chain(b) {
                    if items.insert(k, v).is_some() {
                        return Type::Never;
                    }
                }
                Type::TypeTable(items.into_iter().collect(), None)
            }
            (Type::Float, Type::Int) | (Type::Int, Type::Float) => Type::Int,
            (Type::Array(i1), Type::Array(i2)) => match (i1, i2) {
                (None, _) | (_, None) => Type::Array(None),
                (Some(i1), Some(i2)) => Type::Array(Some(Box::new(*i1 | *i2))),
            },
            (Type::Table(i1, j1, m1), Type::Table(i2, j2, m2)) => Type::Table(
                match (i1, i2) {
                    (None, _) | (_, None) => None,
                    (Some(i1), Some(i2)) => Some(Box::new(*i1 | *i2)),
                },
                match (j1, j2) {
                    (None, _) | (_, None) => None,
                    (Some(j1), Some(j2)) => Some(Box::new(*j1 | *j2)),
                },
                if m1 == m2 { m1 } else { None },
            ),
            _ => Type::Never,
        }
    }
}
impl Type {
    /// A union of the members that survived removing one.
    ///
    /// Removing the last member leaves nothing inhabited behind, which is what
    /// `Never` already means everywhere else in `accepts`: `Never => false`.
    /// Building `Type::Union([])` instead would be a union with no way to
    /// satisfy it that is not spelled `Never`.
    pub fn from_union_kept(kept: Vec<Type>) -> Type {
        // the members were picked one at a time and a pick may itself be a union:
        // a `T | nil` that solved to `X | nil` has to read as one nil, not two
        let mut flat: Vec<Type> = vec![];
        for ty in kept {
            match ty {
                Type::Union(us) => {
                    for member in us {
                        if !flat.contains(&member) {
                            flat.push(member);
                        }
                    }
                }
                other => {
                    if !flat.contains(&other) {
                        flat.push(other);
                    }
                }
            }
        }
        let mut kept = flat.into_iter();
        match (kept.next(), kept.next()) {
            (None, _) => Type::Never,
            // one member left is not a union of one, it is that member
            (Some(only), None) => only,
            (Some(first), Some(second)) => {
                Type::Union([first, second].into_iter().chain(kept).collect())
            }
        }
    }

    /// A tuple of the members that survived removing one.
    ///
    /// Unlike a union, an empty tuple is still inhabited -- it is the type of
    /// the empty array -- so this does not collapse to `Never`.
    pub fn from_tuple_kept(kept: Vec<Type>) -> Type {
        Type::TypeTuple(kept, None)
    }
}

//并集
impl BitOr for Type {
    type Output = Type;
    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Type::Never, a) | (a, Type::Never) => a,
            (Type::TypeTable(..), Type::Table(None, None, m))
            | (Type::Table(None, None, m), Type::TypeTable(..)) => Type::Table(None, None, m),
            (Type::TypeTable(..), Type::Table(k, None, m))
            | (Type::Table(k, None, m), Type::TypeTable(..))
                if matches!(k.as_deref(), Some(Type::String)) =>
            {
                Type::Table(Some(Box::new(Type::String)), None, m)
            }
            (Type::Any, _) | (_, Type::Any) => Type::Any,
            (Type::Union(a), Type::Union(b)) => {
                let mut vec = a.into_vec();
                for bi in b {
                    if !vec.contains(&bi) {
                        vec.push(bi);
                    }
                }
                Type::Union(vec.into_boxed_slice())
            }
            (Type::Union(a), b) => {
                let mut vec = a.into_vec();
                if !vec.contains(&b) {
                    vec.push(b);
                }
                Type::Union(vec.into_boxed_slice())
            }
            (Type::Float, Type::Int) | (Type::Int, Type::Float) => Type::Float,
            (a, b) if a == b => a,
            (Type::Literal(ConstValue::Bool(a)), Type::Literal(ConstValue::Bool(b))) if a != b => {
                Type::Bool
            }
            (a, b) if a.accepts(&b) && folds_into(&a, &b) => a,
            (a, b) if b.accepts(&a) && folds_into(&b, &a) => b,
            (a, b) => Type::Union([a, b].into()),
        }
    }
}

fn rec_accepts(expected: &Type, actual: &Type) -> bool {
    let mut visited = std::collections::HashSet::new();
    rec_bisim_ctx(expected, actual, &mut visited, &mut vec![])
}

pub(crate) fn rec_bisim(
    expected: &Type,
    actual: &Type,
    visited: &mut std::collections::HashSet<(usize, usize)>,
) -> bool {
    rec_bisim_ctx(expected, actual, visited, &mut vec![])
}

type Binders<'a> = Vec<(Box<str>, &'a Type)>;

fn rec_bisim_ctx<'a>(
    expected: &'a Type,
    actual: &Type,
    visited: &mut std::collections::HashSet<(usize, usize)>,
    binders: &mut Binders<'a>,
) -> bool {
    let ek = expected as *const Type as usize;
    let ak = actual as *const Type as usize;
    if !visited.insert((ek, ak)) {
        return true;
    }

    match (expected, actual) {
        (Type::Rec(inner), _) => {
            binders.push((String::new().into_boxed_str(), inner));
            let r = rec_bisim_ctx(inner, actual, visited, binders);
            binders.pop();
            r
        }
        (_, Type::Rec(ainner)) => rec_bisim_ctx(expected, ainner, visited, binders),
        (Type::Param(_), _) => match binders.last() {
            Some((_, bound)) => {
                let b = *bound;
                rec_bisim_ctx(b, actual, visited, binders)
            }
            None => true,
        },
        (_, Type::Param(_)) => true,
        (Type::Union(us), _) => {
            us.iter()
                .any(|u| rec_bisim_ctx(u, actual, visited, binders))
                || *actual == Type::Any
        }
        (_, Type::Union(aus)) => {
            aus.iter()
                .all(|au| rec_bisim_ctx(expected, au, visited, binders))
                || *expected == Type::Any
        }
        (Type::Array(Some(inner)), Type::Array(a)) => a
            .as_deref()
            .is_none_or(|aa| rec_bisim_ctx(inner, aa, visited, binders)),
        (Type::Array(None), Type::Array(_)) => true,
        (Type::TypeTuple(items, rest), Type::Array(Some(inner))) => {
            let members: Vec<&Type> = match &**inner {
                Type::Union(us) => us.iter().collect(),
                other => vec![other],
            };
            let slots: Vec<&Type> = items.iter().chain(rest.as_deref()).collect();
            members
                .iter()
                .all(|m| slots.iter().any(|i| rec_bisim_ctx(i, m, visited, binders)))
        }
        (Type::TypeTuple(_, _), Type::Array(None)) => true,
        (Type::TypeTable(fields, _), Type::TypeTable(af, _)) => fields.iter().all(|(k, dv)| {
            af.iter()
                .find(|(ak, _)| ak == k)
                .is_some_and(|(_, av)| dv.accepts(av))
                || dv.accepts(&Type::Nil)
        }),
        _ => expected.accepts(actual),
    }
}

/// What a constraint is about.
///
/// Positional on purpose: the caller knows which argument it passed and which
/// name that argument was bound to, so nothing has to be recovered by reading
/// the expression back.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Subject {
    Arg(usize),
    TypeParam(Box<str>),
    /// A variable in the flow the guarantee is about, by the name it is written.
    Local(Box<str>),
}

/// The atomic constraints a type function contributes.
///
/// Atomic because the flow applies them one at a time -- a fact is one of these
/// about one subject -- and composing is what `And`, `Or` and `Not` are for. A
/// type function that only returned a type would leave the flow to work out what
/// it had been told, and that part fails quietly.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Constraint {
    Is(Subject, Type),
    IsNot(Subject, Type),
    Sub(Subject, Type),
    /// has field in
    Has(Subject, Box<str>),
    Instance(Subject, Type),
    And(Vec<Constraint>),
    Or(Vec<Constraint>),
    Not(Box<Constraint>),
    Never(Subject),
}

impl Constraint {
    /// The argument position this is about, when it is written as one.
    pub fn subject_index(&self) -> Option<usize> {
        match self.subject()? {
            Subject::Arg(i) => Some(*i),
            _ => None,
        }
    }

    pub fn subject(&self) -> Option<&Subject> {
        match self {
            Constraint::Is(s, _)
            | Constraint::IsNot(s, _)
            | Constraint::Sub(s, _)
            | Constraint::Has(s, _)
            | Constraint::Instance(s, _)
            | Constraint::Never(s) => Some(s),
            Constraint::And(_) | Constraint::Or(_) | Constraint::Not(_) => None,
        }
    }

    /// Every atomic constraint behind this one, in order.
    pub fn flatten(&self) -> Vec<&Constraint> {
        match self {
            Constraint::And(cs) => cs.iter().flat_map(Constraint::flatten).collect(),
            _ => vec![self],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Brand, CmpOp, FunctionType, Label, NumPred, Pattern, Pred, TextPred, Type, Units};
    use crate::value::ConstValue;

    #[test]
    fn accepts_subtyping() {
        assert!(Type::Float.accepts(&Type::Int));
        assert!(Type::Float.accepts(&Type::Float));
        assert!(Type::Int.accepts(&Type::Int));
        assert!(!Type::Int.accepts(&Type::Float));
        assert!(!Type::Float.accepts(&Type::Bool));
        assert!(Type::Any.accepts(&Type::Int));
        assert!(Type::Int.accepts(&Type::Any));
        assert!(Type::String.accepts(&Type::String));
        assert!(!Type::String.accepts(&Type::Table(None, None, None)));
        assert!((Type::Bool | Type::String).accepts(&Type::String));
    }

    #[test]
    fn keyword_parsing() {
        assert_eq!(Type::from_keyword("int"), Some(Type::Int));
        assert_eq!(Type::from_keyword("number"), Some(Type::Float));
        assert_eq!(Type::from_keyword("fn"), Some(Type::Function(None)));
        assert_eq!(Type::from_keyword("frog"), None);
    }

    #[test]
    fn union_accepts_member() {
        let union = Type::Union([Type::Int, Type::Nil].into());
        assert!(union.accepts(&Type::Int));
        assert!(union.accepts(&Type::Nil));
        assert!(!union.accepts(&Type::String));
    }

    #[test]
    fn union_accepts_subtype_member() {
        let union = Type::Float | Type::Nil;
        assert!(union.accepts(&Type::Int));
    }

    #[test]
    fn union_accepts_union() {
        let lhs = Type::Int | Type::Nil;
        let rhs = Type::Float | Type::Nil;
        assert!(rhs.accepts(&lhs));
        let wider = Type::Union([Type::Int, Type::Nil, Type::String].into());
        assert!(!lhs.accepts(&wider));
    }

    #[test]
    fn table_accepts_object() {
        let table = Type::Table(None, None, None);
        let obj = Type::Object {
            id: 0,
            name: "A".into(),
            base: None,
            args: [].into(),
            module: None,
            mt: None,
        };
        assert!(table.accepts(&obj));
        assert!(!obj.accepts(&table));
        assert!(obj.accepts(&obj.clone()));
    }

    #[test]
    fn a_class_in_another_module_is_only_itself() {
        let here = Type::Object {
            id: 0,
            name: "Point".into(),
            base: None,
            args: [].into(),
            module: None,
            mt: None,
        };
        // the same id and the same name, declared elsewhere: two analyses number
        // their classes from zero, so this is a different class
        let there = Type::Object {
            id: 0,
            name: "Point".into(),
            base: None,
            args: [].into(),
            module: Some("m.duka".into()),
            mt: None,
        };
        assert!(!here.accepts(&there), "a name is not a class");
        assert!(!there.accepts(&here));
        assert!(there.accepts(&Type::Any), "and `any` still stands in");
        assert!(there.accepts(&there.clone()));
    }

    #[test]
    fn map_type_rewrites_leaves_inside_every_container() {
        let table = Type::TypeTable(
            vec![(
                ConstValue::String(b"a".to_vec().into_boxed_slice()),
                Box::new(Type::Param("T".into())),
            )],
            Some(Box::new(Type::Array(Some(Box::new(Type::Param(
                "U".into(),
            )))))),
        );
        let t = Type::TypeTuple(
            vec![
                table,
                Type::Function(Some(ft(&[Type::Param("T".into())], false, &[], false))),
            ],
            Some(Box::new(Type::Param("V".into()))),
        );
        let mapped = super::map_type(&t, &mut |leaf| match leaf {
            Type::Param(_) => Some(Type::Int),
            _ => None,
        });
        let Type::TypeTuple(items, rest) = &mapped else {
            panic!("the shape is kept");
        };
        assert!(matches!(rest.as_deref(), Some(Type::Int)));
        let Type::TypeTable(fields, mt) = &items[0] else {
            panic!("a record stays a record");
        };
        assert!(matches!(*fields[0].1, Type::Int));
        let Some(Type::Array(Some(inner))) = mt.as_deref() else {
            panic!("the metatable is walked too");
        };
        assert!(matches!(**inner, Type::Int));
        let Type::Function(Some(sig)) = &items[1] else {
            panic!("a function stays a function");
        };
        assert!(matches!(sig.params[0], Type::Int));
    }

    fn ft(params: &[Type], var_arg: bool, returns: &[Type], return_var_arg: bool) -> FunctionType {
        FunctionType {
            params: params.into(),
            var_arg,
            returns: returns.into(),
            return_var_arg: if return_var_arg {
                Some(Box::new(Type::Any))
            } else {
                None
            },
            ..Default::default()
        }
    }

    #[test]
    fn returns_match_same() {
        let decl = ft(&[], false, &[Type::Int], false);
        let actual = ft(&[], false, &[Type::Int], false);
        assert!(decl.accepts(&actual));
    }

    #[test]
    fn returns_match_mismatch() {
        let decl = ft(&[], false, &[Type::Int], false);
        let actual = ft(&[], false, &[Type::String], false);
        assert!(!decl.accepts(&actual));
    }

    #[test]
    fn returns_match_vararg_decl() {
        let decl = ft(&[], false, &[], true);
        let actual = ft(&[], false, &[Type::String], false);
        assert!(decl.accepts(&actual));
    }

    #[test]
    fn returns_match_unknown_actual() {
        let decl = ft(&[], false, &[Type::Int], false);
        let actual = ft(&[], false, &[], false);
        assert!(decl.accepts(&actual));
    }

    #[test]
    fn returns_match_vararg_actual_covers_rest() {
        let decl = ft(&[], false, &[Type::Int], false);
        let actual = ft(&[], false, &[Type::Int, Type::String], true);
        assert!(decl.accepts(&actual));
    }

    #[test]
    fn returns_match_missing_filled_nil() {
        let decl = ft(&[], false, &[Type::Int, Type::String], false);
        let actual = ft(&[], false, &[Type::Int], false);
        assert!(!decl.accepts(&actual));
    }

    fn meta() -> Type {
        Type::TypeTable(
            vec![(
                ConstValue::String(b"__index".to_vec().into()),
                Box::new(Type::Int),
            )],
            None,
        )
    }

    #[test]
    fn metatable_is_part_of_identity() {
        let plain = Type::Table(None, None, None);
        let with = Type::Table(None, None, Some(Box::new(meta())));
        assert_eq!(with, Type::Table(None, None, Some(Box::new(meta()))));
        assert_ne!(plain, with);
        assert_ne!(
            Type::Object {
                id: 0,
                name: "A".into(),
                base: None,
                args: [].into(),
                module: None,
                mt: None,
            },
            Type::Object {
                id: 0,
                name: "A".into(),
                base: None,
                args: [].into(),
                module: None,
                mt: Some(Box::new(meta())),
            }
        );
    }

    #[test]
    fn a_union_keeps_tables_that_differ_only_by_metatable() {
        let union = Type::Table(None, None, None) | Type::Table(None, None, Some(Box::new(meta())));
        assert!(
            matches!(&union, Type::Union(ms) if ms.len() == 2),
            "{union:?}"
        );
        assert_eq!(
            Type::Table(None, None, Some(Box::new(meta())))
                | Type::Table(None, None, Some(Box::new(meta()))),
            Type::Table(None, None, Some(Box::new(meta())))
        );
    }

    #[test]
    fn metatable_takes_no_part_in_matching() {
        let declared = Type::Table(
            Some(Box::new(Type::String)),
            Some(Box::new(Type::Int)),
            None,
        );
        let actual = Type::Table(
            Some(Box::new(Type::String)),
            Some(Box::new(Type::Int)),
            Some(Box::new(meta())),
        );
        assert!(declared.accepts(&actual));
        assert!(Type::Table(None, None, None).accepts(&actual));
        assert!(Type::Any.accepts(&actual));
    }

    #[test]
    fn display_only_changes_when_a_metatable_is_present() {
        assert_eq!(Type::Table(None, None, None).to_string(), "table");
        assert_eq!(
            Type::Table(
                Some(Box::new(Type::String)),
                Some(Box::new(Type::Int)),
                None
            )
            .to_string(),
            "table<string, int>"
        );
        assert_eq!(
            Type::Table(None, None, Some(Box::new(Type::Int))).to_string(),
            "table + metatable int"
        );
        assert_eq!(
            Type::Object {
                id: 0,
                name: "A".into(),
                base: None,
                args: [].into(),
                module: None,
                mt: Some(Box::new(Type::Int)),
            }
            .to_string(),
            "A + metatable int"
        );
    }

    fn brand(base: Type, label: &str) -> Type {
        Type::Brand(Box::new(Brand {
            base,
            label: Label::Nominal(label.into()),
        }))
    }

    fn unit(base: Type, units: &str) -> Type {
        Type::Brand(Box::new(Brand {
            base,
            label: Label::Unit(Units::parse(units).unwrap()),
        }))
    }

    fn pat(base: Type, pred: Pred) -> Type {
        Type::Pattern(Box::new(Pattern { base, pred }))
    }

    #[test]
    fn a_label_is_a_subtype_of_what_it_labels() {
        assert!(Type::Int.accepts(&brand(Type::Int, "len")));
        assert!(!brand(Type::Int, "len").accepts(&Type::Int));
    }

    #[test]
    fn two_labels_do_not_stand_in_for_each_other() {
        assert!(!brand(Type::Int, "len").accepts(&brand(Type::Int, "usd")));
        assert!(brand(Type::Int, "len").accepts(&brand(Type::Int, "len")));
    }

    #[test]
    fn a_regex_predicate_reads_the_value_whole() {
        let pred = TextPred::Regex("[a-z]+".into());
        assert!(pred.check(b"abc"));
        assert!(!pred.check(b"ab1"));
        assert!(!pred.check(b" ABC"));
    }

    #[test]
    fn units_are_a_group_so_order_does_not_matter() {
        assert_eq!(Units::parse("m*s").unwrap(), Units::parse("s*m").unwrap());
        assert_eq!(Units::parse("m/m").unwrap(), Units::parse("").unwrap());
        assert_eq!(Units::parse("m/1").unwrap(), Units::parse("m").unwrap());
        assert_eq!(
            Units::parse("m")
                .unwrap()
                .div(&Units::parse("s^2").unwrap()),
            Units::parse("m/s^2").unwrap()
        );
    }

    #[test]
    fn a_unit_survives_being_widened_but_a_brand_does_not_vanish() {
        assert!(unit(Type::Float, "m").accepts(&unit(Type::Int, "m")));
        assert!(!unit(Type::Float, "m").accepts(&unit(Type::Float, "s")));
    }

    #[test]
    fn a_pattern_checks_the_value_when_it_is_known() {
        let port = pat(
            Type::Int,
            Pred::Num(NumPred::And(
                vec![
                    NumPred::Cmp(CmpOp::Gt, ConstValue::Int(0)),
                    NumPred::Cmp(CmpOp::Le, ConstValue::Int(65535)),
                ]
                .into(),
            )),
        );
        assert!(port.accepts_value(&Type::Int, Some(&ConstValue::Int(80))));
        assert!(!port.accepts_value(&Type::Int, Some(&ConstValue::Int(70000))));
    }

    #[test]
    fn a_pattern_lets_an_unknown_value_through() {
        let port = pat(
            Type::Int,
            Pred::Num(NumPred::Cmp(CmpOp::Gt, ConstValue::Int(0))),
        );
        assert!(port.accepts_value(&Type::Int, None));
        assert!(Type::Int.accepts(&port));
    }

    #[test]
    fn a_text_pattern_reads_the_literal() {
        let mail = pat(Type::String, Pred::Text(TextPred::Has("@".into())));
        assert!(mail.accepts_value(
            &Type::String,
            Some(&ConstValue::String(b"a@b".to_vec().into()))
        ));
        assert!(!mail.accepts_value(
            &Type::String,
            Some(&ConstValue::String(b"nope".to_vec().into()))
        ));
    }

    #[test]
    fn a_label_and_a_pattern_render_the_way_the_editor_shows_them() {
        assert_eq!(brand(Type::Int, "len").to_string(), "int ::len::");
        assert_eq!(unit(Type::Float, "m/s^2").to_string(), "float <m/s^2>");
        assert_eq!(
            pat(
                Type::Int,
                Pred::Num(NumPred::Cmp(CmpOp::Gt, ConstValue::Int(0)))
            )
            .to_string(),
            "int*"
        );
        assert_eq!(
            brand(
                pat(
                    Type::Int,
                    Pred::Num(NumPred::Cmp(CmpOp::Gt, ConstValue::Int(0)))
                ),
                "len"
            )
            .to_string(),
            "int* ::len::"
        );
        assert_eq!(
            (brand(Type::Int, "len") | Type::Nil).to_string(),
            "(int ::len::)?"
        );
    }
}
