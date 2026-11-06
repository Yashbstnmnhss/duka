use std::{collections::HashMap, fmt::Display};

use duka_shared::dtype::{Constraint, Ty, Type};

use crate::{
    analyzer::builtin::TypeFunctionBuiltin,
    parser::ast::{FuncBody, Param, ReturnAnnotation, TypeDesc},
};

/// What the type context adds past the value context: the evaluator's
/// intermediates. A container in `TypeContext` may hold any of these in any
/// slot; the value context cannot hold one at all.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeLeaf {
    Final(Type),
    Closure(Box<TypeClosure>),
    Tagged { ty: Box<TypeContext>, id: usize },
    Constraints(Box<[Constraint]>),
}

/// The context the evaluator works in: every container slot is a
/// `TypeContext`, so a closure survives being put into a record or a tuple.
pub type TypeContext = Ty<TypeLeaf>;

impl Display for TypeLeaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeLeaf::Final(ty) => write!(f, "{ty}"),
            TypeLeaf::Tagged { ty, .. } => write!(f, "{ty}"),
            TypeLeaf::Closure(tc) => write!(f, "Closure({})", tc.name()),
            TypeLeaf::Constraints(cs) => write!(f, "constraints[{}]", cs.len()),
        }
    }
}

impl Default for TypeLeaf {
    fn default() -> Self {
        TypeLeaf::Final(Type::Any)
    }
}

/// The type a value denotes when only comparison or printing matters: a
/// closure is `any` because there is no table to intern it into, and the
/// containers are rebuilt around that.
pub fn to_type_lossy(value: &TypeContext) -> Type {
    match value {
        Ty::Ext(TypeLeaf::Final(ty)) => ty.clone(),
        Ty::Ext(TypeLeaf::Tagged { ty, .. }) => to_type_lossy(ty),
        Ty::Ext(TypeLeaf::Closure(_)) | Ty::Ext(TypeLeaf::Constraints(_)) => Ty::Any,
        Ty::Union(ts) => Ty::Union(ts.iter().map(to_type_lossy).collect()),
        Ty::TypeTuple(items, rest) => Ty::TypeTuple(
            items.iter().map(to_type_lossy).collect(),
            rest.as_deref().map(|t| Box::new(to_type_lossy(t))),
        ),
        Ty::Array(inner) => Ty::Array(inner.as_deref().map(|t| Box::new(to_type_lossy(t)))),
        Ty::Table(k, v, mt) => Ty::Table(
            k.as_deref().map(|t| Box::new(to_type_lossy(t))),
            v.as_deref().map(|t| Box::new(to_type_lossy(t))),
            mt.as_deref().map(|t| Box::new(to_type_lossy(t))),
        ),
        Ty::TypeTable(items, mt) => Ty::TypeTable(
            items
                .iter()
                .map(|(k, v)| (k.clone(), Box::new(to_type_lossy(v))))
                .collect(),
            mt.as_deref().map(|t| Box::new(to_type_lossy(t))),
        ),
        Ty::Rec(inner) => Ty::Rec(Box::new(to_type_lossy(inner))),
        Ty::Object {
            id,
            name,
            base,
            args,
            module,
            mt,
        } => Ty::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args.iter().map(to_type_lossy).collect(),
            module: module.clone(),
            mt: mt.as_deref().map(|t| Box::new(to_type_lossy(t))),
        },
        Ty::Any => Ty::Any,
        Ty::Unknown => Ty::Unknown,
        Ty::Never => Ty::Never,
        Ty::Nil => Ty::Nil,
        Ty::Bool => Ty::Bool,
        Ty::Int => Ty::Int,
        Ty::Float => Ty::Float,
        Ty::String => Ty::String,
        Ty::Coroutine => Ty::Coroutine,
        Ty::UserData { name } => Ty::UserData { name: name.clone() },
        Ty::Param(name) => Ty::Param(name.clone()),
        Ty::Var(index) => Ty::Var(*index),
        Ty::Literal(value) => Ty::Literal(value.clone()),
        Ty::TypeFn { id, name } => Ty::TypeFn {
            id: *id,
            name: name.clone(),
        },
        Ty::Function(ft) => Ty::Function(ft.clone()),
        Ty::Brand(brand) => Ty::Brand(brand.clone()),
        Ty::Pattern(pattern) => Ty::Pattern(pattern.clone()),
    }
}

pub fn collapse(value: &TypeContext, closures: &mut Vec<TypeClosure>) -> Type {
    let mut leaf = |value: &TypeLeaf| match value {
        TypeLeaf::Final(ty) => ty.clone(),
        TypeLeaf::Tagged { ty, .. } => collapse(ty, closures),
        TypeLeaf::Closure(c) => {
            let id = match closures.iter().position(|k| k == c.as_ref()) {
                Some(id) => id,
                None => {
                    closures.push((**c).clone());
                    closures.len() - 1
                }
            };
            Ty::TypeFn {
                id,
                name: c.name().into(),
            }
        }
        TypeLeaf::Constraints(_) => Ty::Any,
    };
    match value {
        Ty::Ext(ext) => leaf(ext),
        Ty::Union(ts) => Ty::Union(ts.iter().map(|x| collapse(x, closures)).collect()),
        Ty::TypeTuple(items, rest) => Ty::TypeTuple(
            items.iter().map(|x| collapse(x, closures)).collect(),
            rest.as_deref().map(|x| Box::new(collapse(x, closures))),
        ),
        Ty::Array(inner) => Ty::Array(inner.as_deref().map(|x| Box::new(collapse(x, closures)))),
        Ty::Table(k, v, mt) => Ty::Table(
            k.as_deref().map(|x| Box::new(collapse(x, closures))),
            v.as_deref().map(|x| Box::new(collapse(x, closures))),
            mt.as_deref().map(|x| Box::new(collapse(x, closures))),
        ),
        Ty::TypeTable(items, mt) => Ty::TypeTable(
            items
                .iter()
                .map(|(k, v)| (k.clone(), Box::new(collapse(v, closures))))
                .collect(),
            mt.as_deref().map(|x| Box::new(collapse(x, closures))),
        ),
        Ty::Rec(inner) => Ty::Rec(Box::new(collapse(inner, closures))),
        Ty::Object {
            id,
            name,
            base,
            args,
            module,
            mt,
        } => Ty::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args.iter().map(|x| collapse(x, closures)).collect(),
            module: module.clone(),
            mt: mt.as_deref().map(|x| Box::new(collapse(x, closures))),
        },
        Ty::Any => Ty::Any,
        Ty::Unknown => Ty::Unknown,
        Ty::Never => Ty::Never,
        Ty::Nil => Ty::Nil,
        Ty::Bool => Ty::Bool,
        Ty::Int => Ty::Int,
        Ty::Float => Ty::Float,
        Ty::String => Ty::String,
        Ty::Coroutine => Ty::Coroutine,
        Ty::UserData { name } => Ty::UserData { name: name.clone() },
        Ty::Param(name) => Ty::Param(name.clone()),
        Ty::Var(index) => Ty::Var(*index),
        Ty::Literal(value) => Ty::Literal(value.clone()),
        Ty::TypeFn { id, name } => Ty::TypeFn {
            id: *id,
            name: name.clone(),
        },
        Ty::Function(ft) => Ty::Function(ft.clone()),
        Ty::Brand(brand) => Ty::Brand(brand.clone()),
        Ty::Pattern(pattern) => Ty::Pattern(pattern.clone()),
    }
}

/// Embeds a final type into the type context unchanged.
pub fn lift(value: &Type) -> TypeContext {
    Ty::Ext(TypeLeaf::Final(value.clone()))
}

/// Embeds one evaluator value into the type context.
pub fn lift_value(value: &TypeValue) -> TypeContext {
    value.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_shared::value::ConstValue;

    fn unit_mul(_: Box<[TypeValue]>) -> crate::analyzer::builtin::TypeResult {
        Err("test stub".to_owned())
    }

    #[test]
    fn collapse_interns_a_closure_inside_a_record() {
        let record = TypeContext::TypeTable(
            vec![(
                ConstValue::String(b"__mul".to_vec().into_boxed_slice()),
                Box::new(Ty::Ext(TypeLeaf::Closure(Box::new(TypeClosure::rust(
                    "MulUnit", unit_mul,
                ))))),
            )],
            None,
        );
        let mut closures = vec![];
        let ty = collapse(&record, &mut closures);
        let Ty::TypeTable(fields, _) = ty else {
            panic!("the record shape is kept");
        };
        assert!(matches!(*fields[0].1, Ty::TypeFn { .. }));
        assert_eq!(closures.len(), 1, "the closure is interned once");
    }

    #[test]
    fn collapse_descends_through_a_container_tree() {
        let inner = TypeContext::TypeTable(
            vec![(
                ConstValue::String(b"a".to_vec().into_boxed_slice()),
                Box::new(Ty::Ext(TypeLeaf::Final(Ty::Int))),
            )],
            None,
        );
        let value = TypeContext::TypeTuple(vec![inner], Some(Box::new(Ty::String)));
        let ty = collapse(&value, &mut vec![]);
        let Ty::TypeTuple(items, rest) = ty else {
            panic!("the tuple shape is kept");
        };
        assert!(matches!(items[0], Ty::TypeTable(..)));
        assert!(matches!(rest.as_deref(), Some(Ty::String)));
    }

    #[test]
    fn lift_then_collapse_is_the_same_type() {
        let ty = Ty::TypeTable(
            vec![(
                ConstValue::String(b"a".to_vec().into_boxed_slice()),
                Box::new(Ty::Int),
            )],
            None,
        );
        assert_eq!(collapse(&lift(&ty), &mut vec![]), ty);
    }
}

/// The evaluator's value: the type context, with the intermediates it carries.
pub type TypeValue = TypeContext;

#[inline(always)]
pub fn tv(value: Type) -> TypeValue {
    Ty::Ext(TypeLeaf::Final(value))
}

#[inline(always)]
pub fn closure(value: Box<TypeClosure>) -> TypeValue {
    Ty::Ext(TypeLeaf::Closure(value))
}

#[inline(always)]
pub fn tagged(ty: TypeContext, id: usize) -> TypeValue {
    Ty::Ext(TypeLeaf::Tagged {
        ty: Box::new(ty),
        id,
    })
}

/// The value under any number of tags: a tag records which call produced a
/// value, not what the value is, so reading one never depends on it.
pub fn untag(value: &TypeValue) -> &TypeValue {
    match value {
        Ty::Ext(TypeLeaf::Tagged { ty, .. }) => untag(ty),
        other => other,
    }
}

#[inline(always)]
pub fn constraints(value: Box<[Constraint]>) -> TypeValue {
    Ty::Ext(TypeLeaf::Constraints(value))
}

pub trait TypeValueExt {
    fn accepts(&self, other: &Self) -> bool;
    fn without_tag(self) -> Self;
    fn untag(&self) -> &Self;
    fn as_type(&self) -> Option<&Type>;
    fn to_type(&self) -> Type;
    fn to_type_in(&self, closures: &mut Vec<TypeClosure>) -> Type;
}

impl TypeValueExt for TypeValue {
    #[inline]
    fn accepts(&self, other: &TypeValue) -> bool {
        match (self.as_type(), other.as_type()) {
            (Some(a), Some(b)) => a.accepts(b),
            _ => true,
        }
    }
    #[inline]
    fn without_tag(self) -> Self {
        match self {
            Ty::Ext(TypeLeaf::Tagged { ty, .. }) => *ty,
            other => other,
        }
    }
    #[inline]
    fn untag(&self) -> &Self {
        untag(self)
    }
    #[inline]
    fn as_type(&self) -> Option<&Type> {
        match self {
            Ty::Ext(TypeLeaf::Final(ty)) => Some(ty),
            Ty::Ext(TypeLeaf::Tagged { ty, .. }) => ty.as_type(),
            _ => None,
        }
    }
    #[inline]
    fn to_type(&self) -> Type {
        to_type_lossy(self)
    }
    #[inline]
    fn to_type_in(&self, closures: &mut Vec<TypeClosure>) -> Type {
        collapse(self, closures)
    }
}

/// What a `TypeValue::Closure` holds: the three ways a type function can be
/// defined. The body of a DUKA one lives here; a Rust one is carried as its
/// pointer, so calling it needs no registry.
#[derive(Debug, Clone)]
pub enum TypeClosure {
    Duka {
        name: Box<str>,
        params: Box<[Param]>,
        body: Box<FuncBody>,
        captured: Vec<HashMap<Box<str>, (TypeValue, bool)>>,
    },
    Inline {
        name: Box<str>,
        params: Box<[Param]>,
        ret: Box<TypeDesc>,
    },
    Rust {
        name: Box<str>,
        func: TypeFunctionBuiltin,
    },
}
impl PartialEq for TypeClosure {
    fn eq(&self, other: &Self) -> bool {
        use TypeClosure::*;
        match (self, other) {
            (
                Duka {
                    name,
                    params,
                    body,
                    captured,
                },
                Duka {
                    name: name2,
                    params: params2,
                    body: body2,
                    captured: captured2,
                },
            ) => name == name2 && params == params2 && body == body2 && captured == captured2,
            (
                Inline { name, params, ret },
                Inline {
                    name: name2,
                    params: params2,
                    ret: ret2,
                },
            ) => name == name2 && params == params2 && ret == ret2,
            (Rust { name, .. }, Rust { name: name2, .. }) => {
                name == name2 // Only check name, leave alone function pointer
            }
            _ => false,
        }
    }
}

impl TypeClosure {
    pub fn duka(
        name: &str,
        params: Box<[Param]>,
        body: Box<FuncBody>,
        captured: Vec<HashMap<Box<str>, (TypeValue, bool)>>,
    ) -> Self {
        TypeClosure::Duka {
            name: name.into(),
            params,
            body,
            captured,
        }
    }

    pub fn inline(name: &str, params: Box<[Param]>, ret: &TypeDesc) -> Self {
        TypeClosure::Inline {
            name: name.into(),
            params,
            ret: Box::new(ret.clone()),
        }
    }

    pub fn rust(name: &str, func: crate::analyzer::builtin::TypeFunctionBuiltin) -> Self {
        TypeClosure::Rust {
            name: name.into(),
            func,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            TypeClosure::Duka { name, .. }
            | TypeClosure::Inline { name, .. }
            | TypeClosure::Rust { name, .. } => name,
        }
    }

    pub fn params(&self) -> &[Param] {
        match self {
            TypeClosure::Duka { params, .. } | TypeClosure::Inline { params, .. } => params,
            TypeClosure::Rust { .. } => &[],
        }
    }

    pub fn return_annotation(&self) -> Option<&ReturnAnnotation> {
        match self {
            TypeClosure::Duka { body, .. } => body.2.as_ref(),
            _ => None,
        }
    }
}
