use std::{fmt::Display, ops::BitOr};

use serde::{Deserialize, Serialize};

use crate::constants::ctype;

/// 类型标注
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    Nil,
    Bool,
    Int,
    Float,
    String,
    Table,
    Function(Option<FunctionType>),
    Any,
    Union(Box<[Type]>),
}
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
                Type::Nil => ctype::NIL.to_owned(),
                Type::Bool => ctype::BOO.to_owned(),
                Type::Int => ctype::INT.to_owned(),
                Type::Float => ctype::FLO.to_owned(),
                Type::String => ctype::STR.to_owned(),
                Type::Table => ctype::TAB.to_owned(),
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
            "table" => Type::Table,
            "func" | "function" | "fn" => Type::Function(None),
            "nil" => Type::Nil,
            "any" => Type::Any,
            _ => return None,
        })
    }
    pub fn nonnilable(self) -> Self {
        self.into_vec_non_nil()
            .into_iter()
            .reduce(Type::bitor)
            .unwrap_or(Type::Any)
    }
    /// 展开为成员列表并去掉所有 nil(递归把嵌套 union 展平)。
    fn into_vec_non_nil(self) -> Vec<Type> {
        match self {
            Type::Nil => Vec::new(),
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
            Type::Any => true,
            Type::Function(None) => matches!(actual, Type::Function(..) | Type::Any),
            Type::Function(Some(ft)) => match actual {
                Type::Function(None) | Type::Any => true,
                Type::Function(Some(ft2)) => ft.accepts(ft2),
                _ => false,
            },
            Type::Float => matches!(actual, Type::Int | Type::Float | Type::Any),
            Type::Union(u) => match actual {
                Type::Any => true,
                Type::Union(u2) => u2
                    .iter()
                    .all(|i| u.contains(i) || u.iter().any(|v| v.accepts(i))),
                c => u.contains(c) || u.iter().any(|v| v.accepts(c)),
            },
            _ => actual == self || *actual == Type::Any,
        }
    }
}

impl BitOr for Type {
    type Output = Type;
    fn bitor(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
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
        assert!(!Type::String.accepts(&Type::Table));
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

    fn ft(
        params: &[Type],
        var_arg: bool,
        returns: &[Type],
        return_var_arg: bool,
    ) -> FunctionType {
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
