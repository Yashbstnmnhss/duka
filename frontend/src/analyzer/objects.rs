use duka_shared::{
    dtype::{FunctionType, ObjectId},
    errors::Span,
};

use crate::parser::ast::{FuncBody, ParamShape, TypeDesc, TypeParam};

/// object中的成员(properties)
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectMember {
    pub name: Box<str>,
    pub ty: TypeDesc,
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
    /// The variables the class itself declares, in the order they are written.
    ///
    /// A member that mentions one of them is a function of what the class was
    /// applied to -- the first field of a `Pair<int, string>` is an `int` -- so
    /// the names are kept here rather than left to resolve as ordinary lookups,
    /// where a name nobody declared would answer `any` and every application of
    /// the class would lose its arguments.
    pub type_params: Box<[Box<str>]>,
    pub members: Box<[ObjectMember]>,
    pub methods: Box<[ObjectMethod]>,
    pub decl_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodLink {
    pub call_span: Span,
    pub name_span: Span,
    pub decl_span: Span,
    pub owner: ObjectId,
}

/// What a declaration's type parameter list said, in the order it was written.
///
/// A reader needs the list as it stands -- the bounds, the defaults, and which
/// of the parameters take the rest of the arguments -- and the symbol table
/// keeps those per name, which is the wrong shape for it: order is part of what
/// was written, and so is the `...` in front of a parameter. Keeping it here is
/// what lets the editor render a declaration without reading the text back.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredTypeParam {
    pub name: Box<str>,
    pub span: Span,
    pub bound: Option<Box<str>>,
    pub default: Option<Box<str>>,
    /// `...V` takes the rest of the type arguments as a list; `V` takes one.
    pub pack: bool,
}

/// Reads the list a signature was written with, in the order it was written.
pub fn declared_type_params(body: &FuncBody) -> Box<[DeclaredTypeParam]> {
    body.1
        .iter()
        .map(
            |TypeParam((name, span), bound, default, shape)| DeclaredTypeParam {
                name: name.clone().into_boxed_str(),
                span: *span,
                bound: bound.as_ref().map(|b| b.to_string().into_boxed_str()),
                default: default.as_ref().map(|d| d.to_string().into_boxed_str()),
                pack: matches!(shape, ParamShape::Pack),
            },
        )
        .collect()
}
