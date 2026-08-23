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

impl TypeValue {
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
