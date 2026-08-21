use std::{
    fmt::Display,
    ops::{BitAnd, BitOr},
};

use serde::{Deserialize, Serialize};

use crate::{constants::ctype, value::ConstValue};

/// 类型标注
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Type {
    /* Real Type */
    Nil,
    Bool,
    Int,
    Float,
    String,
    Array(Option<Box<Type>>),
    Table(Option<Box<Type>>, Option<Box<Type>>),
    Object {
        id: ObjectId,
        name: Box<str>,
        base: Option<ObjectId>,
        args: Box<[Type]>,
    },
    Function(Option<FunctionType>),
    /* Epyt Laer */

    /* Type Mode */
    /// 值层泛型占位(泛型函数/对象体内的类型参数)
    Param(Box<str>),
    Literal(ConstValue),
    /// `[type, type, type]`
    TypeTuple(Box<[Type]>),
    /// `{ literal_string: type }`
    TypeTable(Box<[(Box<str>, Box<Type>)]>),
    /* Edom Epyt */

    /* Special */
    #[default]
    Any,
    Never,
    Union(Box<[Type]>),
    /* Laiceps */
}
pub type ObjectId = usize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionType {
    pub params: Box<[Type]>,
    pub var_arg: bool,
    pub returns: Box<[Type]>,
    pub return_var_arg: bool,
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
impl Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Type::TypeTable(t) => {
                    format!(
                        "{{\n\t{}\n}}",
                        t.iter()
                            .map(|o| format!("{}: {}", o.0, o.1))
                            .collect::<Vec<_>>()
                            .join(",\n\t")
                    )
                }
                Type::TypeTuple(t) => {
                    format!(
                        "[{}]",
                        t.iter()
                            .map(|o| o.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
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
                Type::Table(k, v) =>
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
                Type::Object { name, args, .. } =>
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
                Type::Param(name) => name.to_string(),
                Type::Literal(v) => match v {
                    ConstValue::String(s) => {
                        let c = std::str::from_utf8(s).unwrap_or("?");
                        format!("\"{c}\"")
                    }
                    other => other.to_string(),
                },
                Type::Function(ft) =>
                    if let Some(ft) = ft {
                        let mut params: Vec<String> =
                            ft.params.iter().map(|i| i.to_string()).collect();
                        if ft.var_arg {
                            params.push("...".to_owned());
                        }
                        let mut returns: Vec<String> =
                            ft.returns.iter().map(|i| i.to_string()).collect();
                        if ft.return_var_arg {
                            returns.push("...".to_owned());
                        }
                        format!(
                            "{}({}){}{}",
                            ctype::FUN,
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
                Type::Union(items) => items
                    .iter()
                    .map(|i| if matches!(i, Type::Function(..) | Type::Union(..)) {
                        format!("({})", i)
                    } else {
                        i.to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" | "),
                Type::Never => ctype::NEV.to_owned(),
            }
        )
    }
}
impl Type {
    pub fn from_keyword(keyword: &str) -> Option<Type> {
        Some(match keyword {
            "int" | "integer" => Type::Int,
            "float" | "num" | "number" => Type::Float,
            "str" | "string" => Type::String,
            "bool" | "boolean" => Type::Bool,
            "table" => Type::Table(None, None),
            "array" | "list" => Type::Array(None),
            "func" | "function" | "fn" => Type::Function(None),
            "nil" => Type::Nil,
            "any" => Type::Any,
            "never" => Type::Never,
            _ => return None,
        })
    }
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
                _ => *actual == Type::Any,
            },
            Type::Table(k, v) => match actual {
                Type::Table(ak, av) => {
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
            Type::Object { .. } => {
                matches!(actual, Type::Object { .. })
                    || *actual == Type::Any
            }
            Type::Union(u) => match actual {
                Type::Any => true,
                Type::Union(u2) => u2
                    .iter()
                    .all(|i| u.contains(i) || u.iter().any(|v| v.accepts(i))),
                c => u.contains(c) || u.iter().any(|v| v.accepts(c)),
            },
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
                None => false,
            },
            Type::Union(u) => match actual {
                Type::Any => true,
                Type::Union(u2) => u2
                    .iter()
                    .all(|i| u.contains(i) || u.iter().any(|m| m.accepts_value(i, cv))),
                c => u.contains(c) || u.iter().any(|m| m.accepts_value(c, cv)),
            },
            Type::Any => true,
            Type::Never => false,
            Type::Nil => matches!(actual, Type::Nil | Type::Any),
            other => other.accepts(actual),
        }
    }
}

impl BitAnd for Type {
    type Output = Type;
    fn bitand(self, rhs: Self) -> Self::Output {
        if self == rhs {
            return self;
        }
        match (self, rhs) {
            (Type::Never, _) | (_, Type::Never) => Type::Never,
            (Type::Any, _) | (_, Type::Any) => Type::Any,
            (Type::Literal(ConstValue::Bool(a)), Type::Literal(ConstValue::Bool(b))) => {
                Type::Literal(ConstValue::Bool(a && b))
            }
            (Type::TypeTable(_), Type::TypeTable(_)) => {
                todo!()
            }
            (Type::Array(i1), Type::Array(i2)) => match (i1, i2) {
                (None, _) | (_, None) => Type::Array(None),
                (Some(i1), Some(i2)) => Type::Array(Some(Box::new(*i1 | *i2))),
            },
            (Type::Table(i1, j1), Type::Table(i2, j2)) => Type::Table(
                match (i1, i2) {
                    (None, _) | (_, None) => None,
                    (Some(i1), Some(i2)) => Some(Box::new(*i1 | *i2)),
                },
                match (j1, j2) {
                    (None, _) | (_, None) => None,
                    (Some(j1), Some(j2)) => Some(Box::new(*j1 | *j2)),
                },
            ),
            _ => Type::Never,
        }
    }
}
impl BitOr for Type {
    type Output = Type;
    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Type::TypeTable(..), Type::Table(None, None))
            | (Type::Table(None, None), Type::TypeTable(..)) => Type::Table(None, None),
            (Type::TypeTable(..), Type::Table(k, None))
            | (Type::Table(k, None), Type::TypeTable(..))
                if matches!(k.as_deref(), Some(Type::String)) =>
            {
                Type::Table(Some(Box::new(Type::String)), None)
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
            (Type::Literal(ConstValue::Bool(a)), Type::Literal(ConstValue::Bool(b))) => {
                Type::Literal(ConstValue::Bool(a || b))
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

#[cfg(test)]
mod tests {
    use super::{FunctionType, Type};

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
        assert!(!Type::String.accepts(&Type::Table(None, None)));
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
        let table = Type::Table(None, None);
        let obj = Type::Object {
            id: 0,
            name: "A".into(),
            base: None,
            args: [].into(),
        };
        assert!(table.accepts(&obj));
        assert!(!obj.accepts(&table));
        assert!(obj.accepts(&obj.clone()));
    }

    fn ft(params: &[Type], var_arg: bool, returns: &[Type], return_var_arg: bool) -> FunctionType {
        FunctionType {
            params: params.into(),
            var_arg,
            returns: returns.into(),
            return_var_arg,
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
        // 实际函数无返回注解(未知)→ 接受
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
}
