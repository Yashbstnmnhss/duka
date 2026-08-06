use duka_macros::Info;
use serde::{Deserialize, Serialize};

/// 类型标注
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Info)]
pub enum Type {
    Nil,
    Bool,
    Int,
    Float,
    #[tag(special)]
    /// `int | float`
    Num,
    String,
    Bytes,
    Table,
    Function,
    #[tag(special)]
    Any,
}

impl Type {
    pub fn from_keyword(keyword: &str) -> Option<Type> {
        Some(match keyword {
            "int" | "integer" => Type::Int,
            "float" => Type::Float,
            "num" | "number" => Type::Num,
            "str" | "string" => Type::String,
            "bool" | "boolean" => Type::Bool,
            "bytes" => Type::Bytes,
            "table" => Type::Table,
            "func" | "function" | "fn" => Type::Function,
            "nil" => Type::Nil,
            "any" => Type::Any,
            _ => return None,
        })
    }

    pub fn accepts(&self, actual: &Type) -> bool {
        match self {
            Type::Any => true,
            Type::Num => matches!(actual, Type::Num | Type::Int | Type::Float | Type::Any),
            _ => actual == self || *actual == Type::Any,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Type;

    #[test]
    fn accepts_subtyping() {
        assert!(Type::Num.accepts(&Type::Int));
        assert!(Type::Num.accepts(&Type::Float));
        assert!(Type::Num.accepts(&Type::Num));
        assert!(Type::Int.accepts(&Type::Int));
        assert!(!Type::Int.accepts(&Type::Float));
        assert!(!Type::Float.accepts(&Type::Int));
        assert!(Type::Any.accepts(&Type::Int));
        assert!(Type::Int.accepts(&Type::Any));
        assert!(Type::String.accepts(&Type::String));
        assert!(!Type::String.accepts(&Type::Table));
    }

    #[test]
    fn keyword_parsing() {
        assert_eq!(Type::from_keyword("int"), Some(Type::Int));
        assert_eq!(Type::from_keyword("number"), Some(Type::Num));
        assert_eq!(Type::from_keyword("fn"), Some(Type::Function));
        assert_eq!(Type::from_keyword("frog"), None);
    }
}
