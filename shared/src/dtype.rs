use std::{
    collections::HashMap,
    fmt::Display,
    ops::{BitAnd, BitOr},
};

use duka_macros::Info;
use serde::{Deserialize, Serialize};

use crate::{constants::ctype, value::ConstValue};

/// 类型标注 (最终结果)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default, Info)]
#[shy]
pub enum Type {
    /* Real Type */
    Nil,
    Bool,
    Int,
    Float,
    String,
    Array(Option<Box<Type>>),
    /// key/value types, plus the metatable type `set_metatable` attached
    Table(Option<Box<Type>>, Option<Box<Type>>, Option<Box<Type>>),
    Object {
        id: ObjectId,
        name: Box<str>,
        base: Option<ObjectId>,
        args: Box<[Type]>,
        module: Option<Box<str>>,
        /// metatable attached to the class table itself, not to instances
        mt: Option<Box<Type>>,
    },
    Function(Option<FunctionType>),
    /// A coroutine handle.
    ///
    /// `spawn` hands one back and `go` takes one, so it is a thing of its own
    /// rather than any of the things it carries.
    Coroutine,
    /// A userdata value: a payload the runtime owns.
    ///
    /// The name is what tells one userdata from another -- a file is not a
    /// regular expression -- and an empty name is a userdata of any kind.
    UserData {
        name: Box<str>,
    },
    TypeFn {
        id: usize,
        name: Box<str>,
    },
    /* Epyt Laer */

    /* Type Mode */
    /// 值层泛型占位(泛型函数/对象体内的类型参数)
    Param(Box<str>),
    Literal(ConstValue),
    /// `[type, type, type]`, the flag marks a written trailing `...`
    TypeTuple(Vec<Type>, bool),
    /// `{ literal: type }` key can be any ConstValue
    TypeTable(
        Vec<(ConstValue, Box<Type>)>,
        Option<Box<Type>>, /* metatable */
    ),
    Rec(Box<Type>),
    /* Edom Epyt */

    /* Special */
    #[default]
    Any,
    Never,
    Union(Box<[Type]>),
    /* Laiceps */
}
pub type ObjectId = usize;

/// Prefix of the placeholder a recursive type function leaves behind while its
/// own result is still being computed. It cannot collide with a type parameter,
/// whose name comes from the source, so the two can share `Type::Param`.
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct FunctionType {
    pub params: Box<[Type]>,
    pub var_arg: bool,
    /// The name a rest parameter collects its types into, `...: Ts`
    pub var_arg_bind: Option<Box<str>>,
    pub returns: Box<[Type]>,
    pub return_var_arg: bool,
    /// The name the open tail collects into, `fn() -> ...Rs`; the tail is what
    /// sits past the declared slots, so `-> (bool, ...Rs)` names after `bool`
    pub return_var_arg_bind: Option<Box<str>>,
    /// The type parameters this signature declares, empty for a plain one
    pub type_vars: Box<[TypeVar]>,
}
impl FunctionType {
    pub fn accepts(&self, other: &FunctionType) -> bool {
        self.params
            .iter()
            .zip(other.params.iter())
            .all(|(a, b)| a.accepts(b))
            && self.returns_match(other)
    }

    /// Parameters use a loose arity (zip); returns are validated item-by-item.
    /// - declaring fn is var-arg returns => accepts
    /// - actual fn has no return annotation => accepts (unknown / no void)
    /// - missing actual returns are filled with `nil`
    /// - an actual var-arg return covers the remainder with `any`
    fn returns_match(&self, other: &FunctionType) -> bool {
        if self.return_var_arg {
            return true;
        }
        if other.returns.is_empty() && !other.return_var_arg {
            return true;
        }
        for (idx, declared) in self.returns.iter().enumerate() {
            let actual = match other.returns.get(idx) {
                Some(t) => t,
                None if other.return_var_arg => &Type::Any,
                None => &Type::Nil,
            };
            if !declared.accepts(actual) {
                return false;
            }
        }
        true
    }
}
/// A record is a table with literal keys, and it nests. The flat one-tab-per
/// field rendering turned `{ a: { b: int } }` into a staircase, so the indent
/// follows the depth.
fn render_table(items: &[(ConstValue, Box<Type>)], depth: usize) -> String {
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

fn render_at(ty: &Type, depth: usize) -> String {
    match ty {
        Type::TypeTable(items, _) => render_table(items, depth),
        other => other.to_string(),
    }
}

/// The rendered type, plus its metatable when it carries one. A type with no
/// metatable renders exactly as it did before the slot existed.
fn with_mt(rendered: String, mt: &Option<Box<Type>>) -> String {
    match mt {
        Some(m) => format!("{rendered} + metatable {m}"),
        None => rendered,
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Type::TypeTable(t, mt) => with_mt(render_table(t, 0), mt),
                Type::Rec(inner) => format!("(recursive) {inner}"), // FIXME
                Type::TypeTuple(t, rest) => {
                    let head = t
                        .iter()
                        .map(|o| o.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let suffix = if *rest && !t.is_empty() { ", " } else { "" };
                    let tail = if *rest { "..." } else { "" };
                    format!("[{head}{suffix}{tail}]")
                }
                Type::Array(inner) =>
                    if let Some(inner) = inner {
                        format!("{}<{}>", ctype::ARR, inner)
                    } else {
                        ctype::ARR.to_owned()
                    },
                Type::Nil => ctype::NIL.to_owned(),
                Type::Bool => ctype::BOO.to_owned(),
                Type::Int => ctype::INT.to_owned(),
                Type::Float => ctype::FLO.to_owned(),
                Type::String => ctype::STR.to_owned(),
                Type::Table(k, v, mt) => with_mt(
                    if k.is_some() || v.is_some() {
                        format!(
                            "{}<{}, {}>",
                            ctype::TAB,
                            k.as_deref().unwrap_or(&Type::Any),
                            v.as_deref().unwrap_or(&Type::Any)
                        )
                    } else {
                        ctype::TAB.to_owned()
                    },
                    mt,
                ),
                // the name is what a reader can be shown; the id is the
                // implementation's handle on the body and means nothing written
                Type::TypeFn { name, .. } => format!("type function {name}"),
                Type::Object { name, args, mt, .. } => with_mt(
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
                Type::Param(name) => rec_name(name).to_string(),
                Type::Literal(v) => match v {
                    ConstValue::String(s) => {
                        let c = str::from_utf8(s).unwrap_or("?");
                        format!("\"{c}\"")
                    }
                    other => other.to_string(),
                },
                Type::Function(ft) =>
                    if let Some(ft) = ft {
                        let generics = ft
                            .type_vars
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>();
                        let mut params: Vec<String> =
                            ft.params.iter().map(|i| i.to_string()).collect();
                        if ft.var_arg {
                            params.push(match &ft.var_arg_bind {
                                Some(name) => format!("...: {name}"),
                                None => "...".to_owned(),
                            });
                        }
                        let mut returns: Vec<String> =
                            ft.returns.iter().map(|i| i.to_string()).collect();
                        if ft.return_var_arg {
                            returns.push(match &ft.return_var_arg_bind {
                                Some(name) => format!("...{name}"),
                                None => "...".to_owned(),
                            });
                        }
                        format!(
                            "{}{}({}){}{}",
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
                            }
                        )
                    } else {
                        ctype::FUN.to_owned()
                    },
                Type::Any => ctype::ANY.to_owned(),
                Type::Coroutine => ctype::COR.to_owned(),
                Type::UserData { name } => {
                    if name.is_empty() {
                        "userdata".to_owned()
                    } else {
                        name.to_string()
                    }
                }
                Type::Union(items) => {
                    let render = |i: &Type| {
                        if matches!(i, Type::Function(..) | Type::Union(..)) {
                            format!("({i})")
                        } else {
                            i.to_string()
                        }
                    };
                    // `T | nil` is what a default-nilable annotation means, and
                    // the language already spells that `T?`, so show that
                    let rest: Vec<&Type> =
                        items.iter().filter(|i| !matches!(i, Type::Nil)).collect();
                    if rest.len() == 1 && rest.len() < items.len() {
                        format!("{}?", render(rest[0]))
                    } else {
                        items.iter().map(render).collect::<Vec<_>>().join(" | ")
                    }
                }
                Type::Never => ctype::NEV.to_owned(),
            }
        )
    }
}
impl Type {
    /// The type of a named field, if this is a record that has one.
    ///
    /// Read here rather than off the rendered text: a record renders one field
    /// per line, and taking that back apart is how a member lookup ends up wrong
    /// about a field whose type happens to contain a brace.
    pub fn field(&self, name: &str) -> Option<Type> {
        match self {
            Type::TypeTable(fields, _) => fields.iter().find_map(|(key, value)| match key {
                ConstValue::String(s) if s.as_ref() == name.as_bytes() => Some((**value).clone()),
                _ => None,
            }),
            _ => None,
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Type> {
        Some(match keyword {
            "int" | "integer" => Type::Int,
            "float" | "num" | "number" => Type::Float,
            "str" | "string" => Type::String,
            "bool" | "boolean" => Type::Bool,
            "table" => Type::Table(None, None, None),
            "array" | "list" => Type::Array(None),
            "func" | "function" | "fn" => Type::Function(None),
            "nil" => Type::Nil,
            "any" => Type::Any,
            "never" => Type::Never,
            "coroutine" => Type::Coroutine,
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
            Type::Function(Some(ft)) => ft.return_var_arg,
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
                Type::TypeTuple(tys, _) => match inner {
                    Some(ty) => tys.iter().all(|t| ty.accepts(t)),
                    None => true,
                },
                _ => *actual == Type::Any,
            },
            Type::Table(k, v, _) => match actual {
                Type::Table(ak, av, _) => {
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
                    k_ok && v_ok
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
                    let fits = if *rest {
                        items.len() <= items2.len()
                    } else {
                        items.len() == items2.len() && !*rest2
                    };
                    fits && items.iter().zip(items2).all(|(a, b)| a.accepts(b))
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
        Type::TypeTuple(kept, false)
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
        (Type::TypeTuple(items, _), Type::Array(Some(inner))) => {
            let members: Vec<&Type> = match &**inner {
                Type::Union(us) => us.iter().collect(),
                other => vec![other],
            };
            members
                .iter()
                .all(|m| items.iter().any(|i| rec_bisim_ctx(i, m, visited, binders)))
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
    use super::{FunctionType, Type};
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

    fn ft(params: &[Type], var_arg: bool, returns: &[Type], return_var_arg: bool) -> FunctionType {
        FunctionType {
            params: params.into(),
            var_arg,
            returns: returns.into(),
            return_var_arg,
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
}
