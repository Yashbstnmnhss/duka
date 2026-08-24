use std::collections::HashMap;

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
        Self::Type(Type::default())
    }
}

impl TypeValue {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Closure(..) => "type function",
            Self::Type(t) => t.name(),
            Self::Tagged { ty, .. } => ty.name(),
        }
    }
    /// Used by unary & binary expression
    pub fn without_tag(self) -> Self {
        match self {
            TypeValue::Tagged { ty, .. } => TypeValue::Type(ty),
            a => a,
        }
    }
    pub fn concretize(&self) -> Type {
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
