use std::{collections::HashMap, fmt::Display};

use duka_shared::dtype::Type;
use serde::{Deserialize, Serialize};

use crate::parser::ast::{FuncBody, Param};

/// 用于type eval求值时的中间值, 由TypeDescriptor而来 最终化为Type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TypeValue {
    Type(Type),
    Tagged { ty: Type, id: usize },
    Closure(Box<TypeClosure>),
}

impl Default for TypeValue {
    fn default() -> Self {
        TypeValue::Type(Type::Any)
    }
}
impl Display for TypeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type(ty) | Self::Tagged { ty, .. } => write!(f, "{}", ty),
            Self::Closure(tc) => write!(f, "Closure({})", tc.name),
        }
    }
}

impl From<Type> for TypeValue {
    fn from(value: Type) -> Self {
        TypeValue::Type(value)
    }
}

impl TypeValue {
    pub fn accepts(&self, other: &TypeValue) -> bool {
        match (self, other) {
            (TypeValue::Type(a), TypeValue::Type(b))
            | (TypeValue::Type(a), TypeValue::Tagged { ty: b, .. })
            | (TypeValue::Tagged { ty: a, .. }, TypeValue::Type(b))
            | (TypeValue::Tagged { ty: a, .. }, TypeValue::Tagged { ty: b, .. }) => a.accepts(b),
            (TypeValue::Closure(_), _) | (_, TypeValue::Closure(_)) => true, // treated as any in type
        }
    }
    /// Used by unary & binary expression
    pub fn without_tag(self) -> Self {
        match self {
            TypeValue::Tagged { ty, .. } => TypeValue::Type(ty),
            a => a,
        }
    }
    pub fn as_type(&self) -> Option<&Type> {
        match self {
            TypeValue::Type(t) => Some(t),
            TypeValue::Tagged { ty, .. } => Some(ty),
            _ => None,
        }
    }
    pub fn to_type(&self) -> Type {
        match self {
            TypeValue::Type(t) | TypeValue::Tagged { ty: t, .. } => t.clone(),
            TypeValue::Closure(_) => Type::Any,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeClosure {
    pub name: Box<str>,
    pub params: Box<[Param]>,
    pub body: Box<FuncBody>,
    pub captured: Vec<HashMap<Box<str>, (TypeValue, bool)>>,
}
