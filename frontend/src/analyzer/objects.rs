use duka_shared::{
    dtype::{FunctionType, ObjectId, Type},
    errors::Span,
    value::ConstValue,
};

use crate::parser::ast::{
    FuncBody, ObjectDef, ObjectProperty, ParamMode, TypeDesc, TypeParam,
};

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
    /// Where the body is, so a checker walking into it can tell which class the
    /// method belongs to and what `self` is.
    pub body_span: Span,
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

/// Whether `target` is anywhere above `from` in the class table.
///
/// `Type::Object.base` is an `ObjectId` -- an index into one analysis's own
/// table -- so `Type::accepts` can compare the link but cannot follow it, and
/// says so in its own comment. Following it is what turns `C extends B extends A`
/// from two steps into one.
fn is_transitively(target: ObjectId, from: Option<ObjectId>, objects: &[ObjectType]) -> bool {
    let mut cur = from;
    while let Some(id) = cur {
        if id == target {
            return true;
        }
        cur = objects.get(id).and_then(|o| o.base);
    }
    false
}

/// `Type::accepts`, with the class table in hand for the one case it cannot
/// settle on its own: two classes standing in for each other.
///
/// A union is taken apart here rather than left to `Type::accepts`, because its
/// members are checked by a recursive `accepts` call that has no table to walk:
/// an annotation on a parameter is nilable by default, so `x: A` reaches this
/// as `A | nil` and the class is one member of it.
pub fn accepts(expected: &Type, actual: &Type, objects: &[ObjectType]) -> bool {
    match (expected, actual) {
        // two unions are compared member to member: distributing only the
        // expected side asks whether one member accepts the whole other union,
        // which is how `int?` came out not accepting `int?`
        (Type::Union(us), Type::Union(ws)) => {
            ws.iter().all(|w| us.iter().any(|u| accepts(u, w, objects)))
        }
        (Type::Union(us), _) => us.iter().any(|u| accepts(u, actual, objects)),
        _ => object_of(expected)
            .and_then(|e| object_of(actual).map(|a| (e, a)))
            .map_or_else(
                || expected.accepts(actual),
                |((id, base, args, amodule), (aid, abase, aargs, module))| {
                    if module != amodule {
                        return false;
                    }
                    if id == aid {
                        return args.iter().zip(aargs.iter()).all(|(a, b)| a.accepts(b));
                    }
                    is_transitively(id, abase, objects) || is_transitively(aid, base, objects)
                },
            ),
    }
}

/// The parts of a class a comparison needs, for the two types that carry them.
fn object_of(t: &Type) -> Option<(ObjectId, Option<ObjectId>, &[Type], &Option<Box<str>>)> {
    let Type::Object {
        id,
        base,
        args,
        module,
        ..
    } = t
    else {
        return None;
    };
    Some((*id, *base, args, module))
}

/// `Type::accepts_value` with the same table, for the call site that has a
/// literal in hand and so compares against the constant rather than its type.
pub fn accepts_value(
    expected: &Type,
    actual: &Type,
    cv: Option<&ConstValue>,
    objects: &[ObjectType],
) -> bool {
    match (expected, actual) {
        (Type::Union(us), Type::Union(ws)) => ws
            .iter()
            .all(|w| us.iter().any(|u| accepts_value(u, w, cv, objects))),
        (Type::Union(us), _) => us.iter().any(|u| accepts_value(u, actual, cv, objects)),
        _ if object_of(expected).is_some() && object_of(actual).is_some() => {
            accepts(expected, actual, objects)
        }
        _ => expected.accepts_value(actual, cv),
    }
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
    pub literal: bool,
}

/// The span an object covers: its name and everything written inside it.
///
/// A reader hovering a member of the class stands inside that span, so this is
/// what tells which class the member belongs to.
pub fn object_span(object: &ObjectDef) -> Span {
    let mut span = object.name.1;
    if let Some((_, base)) = &object.base {
        span = span + *base;
    }
    for TypeParam((_, at), ..) in object.type_params.iter() {
        span = span + *at;
    }
    for property in object.properties.iter() {
        match property {
            ObjectProperty::NameValue((_, at), value, ty) => {
                span = span + *at;
                if let Some(value) = value {
                    span = span + value.1;
                }
                if let Some(ty) = ty.as_ref().and_then(|t| t.span()) {
                    span = span + ty;
                }
            }
            ObjectProperty::KeyValue(key, value, ty) => {
                span = span + key.1;
                if let Some(value) = value {
                    span = span + value.1;
                }
                if let Some(ty) = ty.as_ref().and_then(|t| t.span()) {
                    span = span + ty;
                }
            }
        }
    }
    for (name, _, body) in object.static_methods.iter().chain(object.methods.iter()) {
        span = span + name.1 + func_body_span(body);
    }
    span
}

/// Whether a function declared in an object is a metamethod.
///
/// `__`-named ones are, and they are read as instance methods: the operation's
/// receiver is `self`, and the parameters after it are the other operands.
pub fn is_metamethod_name(name: &str) -> bool {
    name.starts_with("__")
}

/// Where a body is, which is what a checker walking into it has to match on.
pub fn func_body_span(body: &FuncBody) -> Span {
    body.4.0.last().map_or(Span::EMPTY, |stmt| stmt.1)
}

/// Reads the list a signature was written with, in the order it was written.
pub fn declared_type_params(body: &FuncBody) -> Box<[DeclaredTypeParam]> {    body.1
        .iter()
        .map(
            |TypeParam((name, span), bound, default, shape, literal)| DeclaredTypeParam {
                name: name.clone().into_boxed_str(),
                span: *span,
                bound: bound.as_ref().map(|b| b.to_string().into_boxed_str()),
                default: default.as_ref().map(|d| d.to_string().into_boxed_str()),
                pack: matches!(shape, ParamMode::Pack),
                literal: *literal,
            },
        )
        .collect()
}
