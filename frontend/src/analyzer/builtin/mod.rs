use duka_shared::builtin::Builtins;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::docs::{MetaInfo, MetaItemInfo};
use duka_shared::dtype::FunctionType;
use duka_shared::dtype::{Constraint, Subject, Type};
use duka_shared::regex;
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use std::sync::LazyLock;
use std::sync::RwLock;

use crate::analyzer::tyval::TypeValue;

/// Duka's truthiness rather than a comparison against `bool`.
pub fn concept_holds(t: &Type) -> bool {
    !matches!(
        t,
        Type::Literal(ConstValue::Bool(false))
            | Type::Literal(ConstValue::Nil)
            | Type::Nil
            | Type::Never
    )
}

/// A concept carrying nothing but its own answer.
fn obligation(t: &Type) -> Option<Type> {
    concept_holds(t)
        .then(|| Type::Literal(ConstValue::Bool(true)))
        .filter(|_| matches!(t, Type::Literal(ConstValue::Bool(_))))
}

/// What a type function computes, and what it promises about its arguments.
///
/// The two travel together: a function that turns `string | nil` into `string`
/// is the same function that promises the argument was not nil. The flow reads
/// the second half and never has to guess from the shape of the first.
pub struct TypeBuiltin {
    pub func: fn(Box<[TypeValue]>) -> Result<(TypeValue, Vec<Constraint>), String>,
}

#[inline(always)]
fn plain(value: TypeValue) -> Result<(TypeValue, Vec<Constraint>), String> {
    Ok((value, Vec::new()))
}

macro_rules! type_functions {
    ($(
        #[duka_builtin(doc = $doc: literal)]
        $name: literal($p: ident [$count: literal] $(, $for: literal: $bound: expr)*) $body: block
    ),*) => {
        pub static TYPE_BUILTINS: GlobalBuiltins<TypeBuiltin> = LazyLock::new(|| {
            RwLock::new(
                Builtins::<TypeBuiltin>::new()
                    $(.register(
                        $name,
                        TypeBuiltin {
                            func: |$p| {
                                ($p.len() < $count).then_error(|| concat!("Expected ", stringify!($count)," type argument").to_owned())?;
                                $(($bound.accepts(&$p[$for].to_type())).or_else_error(|| concat!("Expected ", stringify!($bound), " for the type argument at ", stringify!($for)).to_owned())?;)*
                                $body
                            },
                        }
                    ))*
            )
        });
        pub const TYPE_BUILTINS_META: MetaInfo = MetaInfo {
            name: "type-context",
            doc: "Builtins for type-context",
            example: None,
            info: MetaItemInfo::Module {
                inner: &[
                    $(MetaInfo {
                        name: $name,
                        doc: $doc,
                        example: None,
                        info: MetaItemInfo::TypeFunction {
                            param_count: $count
                        },
                        flags: &[],
                    }),*
                ]
            },
            flags: &[("feature", &["type-context"])],
        };
    };
}

#[inline]
fn get_str(bytes: &[u8]) -> Result<&str, String> {
    str::from_utf8(bytes).map_err(|e| e.to_string())
}

type_functions! {
    #[duka_builtin(doc = "Throw an error with a message")]
    "Error"(v[1]) {
        Err(v[0].to_string())
    },
    #[duka_builtin(doc = "If A isn't true, throws an error with message B")]
    "Assert"(v[2]) {
        let ty = v[0].to_type();
        let value = if matches!(ty, Type::Literal(ConstValue::Bool(true))) {
            TypeValue::Type(ty)
        } else {
            return Err(v[1].to_string());
        };
        plain(value)
    },
    #[duka_builtin(doc = "Stringify a type")]
    "Stringify"(v[1]) {
        plain(TypeValue::Type(
            Type::Literal(ConstValue::String(
                v[0].to_string().into_bytes().into_boxed_slice()
            ))
        ))
    },
    #[duka_builtin(doc = "Pack types from type array into a union type")]
    "Union"(v[1]) {
        let t = v[0].to_type();
        if let Type::TypeTuple(types, _) = t {
            plain(TypeValue::Type(Type::Union(types.into_boxed_slice())))
        } else {
            plain(v.into_iter().next().expect("Checked"))
        }
    },
    #[duka_builtin(doc = "Unpack a union type")]
    "Unpack"(v[1]) {
        let t = v[0].to_type();
        if let Type::Union(types) = t {
            plain(TypeValue::Type(Type::TypeTuple(types.into_vec(), false)))
        } else {
            plain(v.into_iter().next().expect("Checked"))
        }
    },
    #[duka_builtin(doc = "Whether A is an iterable type value")]
    "IsIterable"(v[1]) {
        plain(TypeValue::Type(
            Type::Literal(ConstValue::Bool(
                matches!(v[0].as_type(), Some(
                    Type::TypeTable(..) | Type::TypeTuple(..) | Type::Object { .. }
                ))
            ))
        ))
    },
    #[duka_builtin(doc = "Whether B is a sub type of A")]
    "IsSubType"(v[2]) {
        let constraints = vec![Constraint::Sub(Subject::Arg(1), v[1].to_type())];
        Ok((
            TypeValue::Type(Type::Literal(ConstValue::Bool(v[1].accepts(&v[0])))),
            constraints,
        ))
    },
    #[duka_builtin(doc = "A type with nil taken out of it")]
    "NonNil"(v[1]) {
        let constraints = vec![Constraint::IsNot(Subject::Arg(0), Type::Nil)];
        Ok((TypeValue::Type(v[0].to_type().nonnilable()), constraints))
    },
    #[duka_builtin(doc = "Get the super type of a type value in type context")]
    "Super"(v[1]) {
        let ty = v.into_iter().next().expect("Checked");
        plain(TypeValue::Type(match ty.to_type() {
            Type::Literal(l) => l.type_of(),
            Type::TypeTable(..) => Type::Table(None, None, None),
            Type::TypeTuple(..) => Type::Array(None),
            Type::Param(..) => Type::Any,
            a => a
        }))
    },
    #[duka_builtin(doc = "Whether A is in B")]
    "In"(v[2]) {
        if let Some(other) = &v[0].as_type() {
            if let Some(Type::Union(types)) = &v[1].as_type()  {
                return plain(Type::Literal(ConstValue::Bool(types.contains(other))).into())
            }
            else if let Some(Type::TypeTuple(types, _)) = &v[1].as_type() {
                return plain(Type::Literal(ConstValue::Bool(types.contains(other))).into())
            }
            else if let Some(Type::TypeTable(types, _)) = &v[1].as_type() &&
                let Type::Literal(c) = other
            {
                return plain(Type::Literal(ConstValue::Bool(types.iter().any(|i| &i.0 == c))).into())
            }
        }
        plain(Type::Literal(ConstValue::Bool(false)).into())
    },
    #[duka_builtin(doc = "Whether the function type has var-arg parameter")]
    "HasVarArg"(v[1], 0: Type::Function(None)) {
        plain(TypeValue::Type(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.var_arg))
        } else {
            Type::Literal(ConstValue::Bool(false))
        }))
    },
    #[duka_builtin(doc = "Whether the function type has var-arg returns")]
    "HasRetVarArg"(v[1], 0: Type::Function(None)) {
        plain(TypeValue::Type(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.return_var_arg))
        } else {
            Type::Literal(ConstValue::Bool(false))
        }))
    },
    #[duka_builtin(doc = "Build a function type")]
    "BuildFunction"(v[4]) {
        let mut v = v.into_iter();
        let args = v.next().expect("Checked");
        let var_arg = v.next().expect("Checked");
        let rets = v.next().expect("Checked");
        let var_ret = v.next().expect("Checked");

        fn deal(tv: TypeValue) -> Box<[Type]> {
            match tv.without_tag() {
                TypeValue::Type(ty) => match ty {
                    Type::TypeTuple(t, _) => t.into_boxed_slice(),
                    Type::Never => [].into(),
                    other => [other].into()
                },
                _ => [].into()
            }
        }

        plain(
            TypeValue::Type(Type::Function(Some(
                FunctionType {
                    params: deal(args),
                    var_arg: matches!(var_arg.without_tag(), TypeValue::Type(Type::Literal(ConstValue::Bool(true)))),
                    returns: deal(rets),
                    return_var_arg : matches!(var_ret.without_tag(), TypeValue::Type(Type::Literal(ConstValue::Bool(true)))),
                    ..Default::default()
                }
            )))
        )
    },
    #[duka_builtin(doc = "Remove B in union or type array A")]
    "Exclude"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked");
        let second = v.next().expect("Checked");
        if let Some(t) = second.as_type() {
            match first {
                TypeValue::Type(Type::Union(types)) if types.contains(t) => {
                    let kept: Vec<Type> = types.into_iter().filter(|i| i != t).collect();
                    return plain(Type::from_union_kept(kept).into())
                },
                TypeValue::Type(Type::TypeTuple(types, _)) if types.contains(t) => {
                    let kept: Vec<Type> = types.into_iter().filter(|i| i != t).collect();
                    return plain(Type::from_tuple_kept(kept).into())
                },
                _ => ()
            }
        }
        plain(first)
    },
    #[duka_builtin(doc = "Return true if literal string type A ends with B")]
    "EndsWith"(v[2]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(end))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let end = get_str(end)?;
                Type::Literal(ConstValue::Bool(tar.ends_with(end)))
            } else {
                Type::Literal(ConstValue::Bool(false))
            },
        ))
    },
    #[duka_builtin(doc = "Return true if literal string type A starts with B")]
    "StartsWith"(v[2]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(start))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let start = get_str(start)?;
                Type::Literal(ConstValue::Bool(tar.starts_with(start)))
            } else {
                Type::Literal(ConstValue::Bool(false))
            },
        ))
    },
    #[duka_builtin(doc = "Remove an element in type tuple or type table by its index or key")]
    "Remove"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked").without_tag();
        let second = v.next().expect("Checked").without_tag();
        plain(match (first, second) {
            (TypeValue::Type(Type::TypeTuple(mut t, rest)), TypeValue::Type(Type::Literal(ConstValue::Int(idx)))) => {
                // a negative index counts back from the tail; either end that
                // leaves no element is reported rather than removed
                let at = usize::try_from(idx)
                    .ok()
                    .or_else(|| (t.len() as i64).checked_add(idx).and_then(|i| usize::try_from(i).ok()))
                    .filter(|&at| at < t.len());
                let Some(at) = at else {
                    return Err(format!(
                        "index {idx} is out of range for a tuple of length {}",
                        t.len()
                    ));
                };
                t.remove(at);
                TypeValue::Type(Type::TypeTuple(t, rest))
            },
            (TypeValue::Type(Type::TypeTable(mut t, mt)), TypeValue::Type(Type::Literal(key))) => {
                t.retain(|v| v.0 != key);
                TypeValue::Type(Type::TypeTable(t, mt))
            },
            (other, _) => other
        })
    },
    #[duka_builtin(doc = "Insert new element to the head of a type tuple or type table")]
    "AppendHead"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked").without_tag();
        let second = v.next().expect("Checked");
        plain(match first {
            TypeValue::Type(Type::TypeTuple(mut t, rest)) => {
                t.insert(0, second.to_type());
                TypeValue::Type(Type::TypeTuple(t, rest))
            },
            TypeValue::Type(Type::TypeTable(mut t, mt)) => {
                if let Some(Type::Literal(cv)) = second.as_type() {
                    let val = v.next().unwrap_or_default().to_type();
                    t.insert(0, (cv.clone(), Box::new(val)));
                }
                TypeValue::Type(Type::TypeTable(t, mt))
            },
            other => other
        })
    },
    #[duka_builtin(doc = "Append new element to type tuple or type table")]
    "Append"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked").without_tag();
        let second = v.next().expect("Checked");
        plain(match first {
            TypeValue::Type(Type::TypeTuple(mut t, rest)) => {
                t.push(second.to_type());
                TypeValue::Type(Type::TypeTuple(t, rest))
            },
            TypeValue::Type(Type::TypeTable(mut t, mt)) => {
                if let Some(Type::Literal(cv)) = second.as_type() {
                    let val = v.next().unwrap_or_default().to_type();
                    t.push((cv.clone(), Box::new(val)));
                }
                TypeValue::Type(Type::TypeTable(t, mt))
            },
            other => other
        })
    },
    #[duka_builtin(doc = "Slice string literal type A with start index B and optional end index C")]
    "Slice"(v[2]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::Int(start))) = &v[1].as_type() {
                let start = *start as usize;
                let tar = get_str(tar)?;
                Type::Literal(ConstValue::String(if let Some(TypeValue::Type(Type::Literal(ConstValue::Int(end)))) = v.get(2) {
                    let end = *end as usize;
                    &tar[start..end]
                } else {
                    &tar[start..]
                }.as_bytes().into()))
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Split string literal type A by separator string literal type B")]
    "Split"(v[2]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(sep))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let sep = get_str(sep)?;
                Type::TypeTuple(
                    tar.split(sep).map(|i| Type::Literal(ConstValue::String(i.as_bytes().into()))).collect(),
                    false,
                )
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Convert a string literal type into uppercase")]
    "Uppercase"(v[1]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().map(|b| b.to_ascii_uppercase()).collect(),
                ))
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Capitalize a string literal type")]
    "Capitalize"(v[1]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().enumerate().map(|(i, b)| if i == 0 { b.to_ascii_uppercase() } else { *b }).collect(),
                ))
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Uncapitalize a string literal type")]
    "Uncapitalize"(v[1]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().enumerate().map(|(i, b)| if i == 0 { b.to_ascii_lowercase() } else { *b }).collect(),
                ))
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Convert a string literal type into lowercase")]
    "Lowercase"(v[1]) {
        plain(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().map(|b| b.to_ascii_lowercase()).collect(),
                ))
            } else {
                return plain(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Search a RegEx pattern in a string literal type")]
    "Regex"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked");
        let second = v.next().expect("Checked");
        if let Type::Literal(ConstValue::String(pat)) = first.to_type() && let Type::Literal(ConstValue::String(who)) = second.to_type() {
            let pat = get_str(&pat)?;
            let who2 = get_str(&who)?;
            let mac = regex::Runner::new(&regex::compile(pat).map_err(|e| e.to_string())?).search(who2, 0);
            plain(TypeValue::Type(Type::TypeTuple(
                mac.map(|v| v.captures.into_iter().map(|i| Type::Literal(ConstValue::String(who[i.0..i.1].into()))).collect()).unwrap_or_default(),
                false,
            )))
        } else {
            plain(second)
        }
    },
    #[duka_builtin(doc = "Whether A is a member of union, tuple or record B")]
    "NotIn"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked").to_type();
        let second = v.next().expect("Checked").to_type();
        let Some(members) = member_types(&second) else {
            return Ok((
                TypeValue::Type(Type::Never),
                vec![Constraint::Never(Subject::Arg(0))],
            ));
        };
        let value = if members.contains(&first) {
            Type::Never
        } else {
            first
        };
        let constraints: Vec<Constraint> = members
            .into_iter()
            .map(|m| Constraint::IsNot(Subject::Arg(0), m))
            .collect();
        Ok((TypeValue::Type(value), constraints))
    },
    #[duka_builtin(doc = "Narrow A when A has a member called B")]
    "Has"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked").to_type();
        let second = v.next().expect("Checked").to_type();
        let Type::Literal(ConstValue::String(name)) = second else {
            return plain(TypeValue::Type(Type::Any));
        };
        let name = get_str(&name)?;
        let value = if first.field(name).is_some() {
            first
        } else if matches!(first, Type::Table(..) | Type::Param(_) | Type::Any) {
            // a table holds whatever was put in it, and a variable is not
            // decided -- the same reading the flow gives the constraint
            first
        } else {
            Type::Never
        };
        let constraints = vec![
            Constraint::Has(Subject::Arg(0), name.to_owned().into_boxed_str())
        ];
        Ok((TypeValue::Type(value), constraints))
    },
    #[duka_builtin(doc = "A if it answers, otherwise B -- one of the two, whichever answers")]
    "Any"(v[2]) {
        let mut v = v.into_iter();
        let a = v.next().expect("Checked").to_type();
        let b = v.next().expect("Checked").to_type();
        let narrowed = match (a.clone(), b.clone()) {
            (a, _) if !concept_holds(&a) => b,
            (_, b) if !concept_holds(&b) => a,
            (a, b) => match (obligation(&a), obligation(&b)) {
                (Some(_), None) => a,
                (None, Some(_)) => b,
                _ => a | b,
            },
        };
        plain(TypeValue::Type(if concept_holds(&narrowed) {
            narrowed
        } else {
            Type::Never
        }))
    },
    #[duka_builtin(doc = "Narrow A when both answer -- either side may be the one that narrowed")]
    "All"(v[2]) {
        let mut v = v.into_iter();
        let a = v.next().expect("Checked").to_type();
        let b = v.next().expect("Checked").to_type();
        if !concept_holds(&a) || !concept_holds(&b) {
            return plain(TypeValue::Type(Type::Never));
        }
        plain(TypeValue::Type(match (obligation(&a), obligation(&b)) {
            (Some(_), None) => b,
            (None, Some(_)) => a,
            _ => a & b,
        }))
    },
    #[duka_builtin(doc = "Hand a type back while keeping the constraints its evaluation contributed")]
    "provide"(v[1]) {
        plain(v.into_iter().next().expect("Checked"))
    },
    #[duka_builtin(doc = "Narrow A when C does not answer -- impossible once C does")]
    "Not"(v[2]) {
        let mut v = v.into_iter();
        let a = v.next().expect("Checked").to_type();
        let c = v.next().expect("Checked").to_type();
        plain(TypeValue::Type(if concept_holds(&c) {
            Type::Never
        } else {
            a
        }))
    },
    #[duka_builtin(doc = "Get metatable type")]
    "GetMetatable"(v[1]) {
        plain(TypeValue::Type(Type::Any))
    },
    #[duka_builtin(doc = "Attach metatable MT to T, or take it off when MT is nil")]
    "WithMetatable"(v[2]) {
        let mut v = v.into_iter();
        let t = v.next().expect("Checked").to_type();
        let mt = match v.next().expect("Checked").to_type() {
            Type::Nil => None,
            other => Some(Box::new(other)),
        };
        plain(TypeValue::Type(match t {
            Type::TypeTable(fields, _) => Type::TypeTable(fields, mt),
            Type::Table(k, v, _) => Type::Table(k, v, mt),
            Type::Object {
                id,
                name,
                base,
                args,
                module,
                ..
            } => Type::Object {
                id,
                name,
                base,
                args,
                module,
                mt,
            },
            other => {
                return Err(format!(
                    "WithMetatable takes a table or object type, got {}",
                    other
                ))
            }
        }))
    }
}

/// The members a union, tuple or record holds, or `None` for anything else.
fn member_types(t: &Type) -> Option<Vec<Type>> {
    match t {
        Type::Union(us) => Some(us.to_vec()),
        Type::TypeTuple(items, _) => Some(items.clone()),
        Type::TypeTable(fields, _) => Some(fields.iter().map(|(_, v)| (**v).clone()).collect()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(name: &str, args: Box<[Type]>) -> Result<Type, String> {
        let bi = TYPE_BUILTINS.read().map_err(|e| e.to_string())?;
        let f = bi.get(&name).ok_or_else(|| "missing".to_owned())?.func;
        let args = args
            .into_iter()
            .map(TypeValue::Type)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        f(args).map(|(v, _)| v.to_type())
    }

    fn obj() -> Type {
        Type::Object {
            id: 0,
            name: "A".into(),
            base: None,
            args: [].into(),
            module: None,
            mt: None,
        }
    }

    #[test]
    fn with_metatable_attaches_to_table_and_object() {
        let mt = Type::TypeTable(vec![], None);
        let on_table = apply(
            "WithMetatable",
            [Type::Table(None, None, None), mt.clone()].into(),
        )
        .expect("table accepts a metatable");
        assert_eq!(
            on_table,
            Type::Table(None, None, Some(Box::new(mt.clone())))
        );
        let on_obj =
            apply("WithMetatable", [obj(), mt.clone()].into()).expect("object accepts a metatable");
        let mut want = obj();
        if let Type::Object { mt: slot, .. } = &mut want {
            *slot = Some(Box::new(mt));
        }
        assert_eq!(on_obj, want);
    }

    #[test]
    fn with_metatable_nil_takes_it_off() {
        let had = Type::Table(None, None, Some(Box::new(Type::Int)));
        assert_eq!(
            apply("WithMetatable", [had, Type::Nil].into()).expect("nil detaches"),
            Type::Table(None, None, None)
        );
    }

    #[test]
    fn with_metatable_rejects_other_types() {
        assert!(apply("WithMetatable", [Type::Int, Type::Nil].into()).is_err());
    }
}
