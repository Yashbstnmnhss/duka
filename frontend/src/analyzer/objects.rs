use duka_shared::{
    dtype::{FunctionType, ObjectId},
    errors::Span,
};

use crate::parser::ast::TypeValue;

/// object中的成员(properties)
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectMember {
    pub name: Box<str>,
    pub ty: TypeValue,
    pub span: Span,
}
/// object中的静态,实例方法
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectMethod {
    pub name: Box<str>,
    pub sig: FunctionType,
    pub span: Span,
    pub is_static: bool,
}
///object类型本身
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectType {
    pub name: Box<str>,
    pub global: bool,
    pub base: Option<ObjectId>,
    pub base_ref: Option<(Box<str>, Span)>,
    pub members: Box<[ObjectMember]>,
    pub methods: Box<[ObjectMethod]>,
    pub decl_span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodLink {
    pub call_span: Span,
    pub name_span: Span,
    pub decl_span: Span,
    pub owner: ObjectId,
}
