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
    /// The type this value denotes, with a type function kept as one.
    ///
    /// This is what a type constructor has to use. `to_type` answers `any` for a
    /// closure because there was nowhere to put the body, and that is the right
    /// answer when all the caller wants is to compare or print. It is the wrong
    /// answer when the value is going *into* a type -- a record field, an array
    /// element, a type argument -- because then the type function is lost and
    /// every read of it afterwards is working with `any`.
    pub fn to_type_in(&self, closures: &mut Vec<TypeClosure>) -> Type {
        match self {
            TypeValue::Type(t) | TypeValue::Tagged { ty: t, .. } => t.clone(),
            TypeValue::Closure(c) => {
                // the same body is interned once, so that two mentions of it
                // are the same type and not two equal-looking ones
                let id = match closures.iter().position(|k| k == c.as_ref()) {
                    Some(id) => id,
                    None => {
                        closures.push((**c).clone());
                        closures.len() - 1
                    }
                };
                Type::TypeFn {
                    id,
                    name: c.name.clone(),
                }
            }
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
