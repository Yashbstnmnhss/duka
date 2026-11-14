use duka_macros::duka_builtin_type_def;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::dtype::FunctionType;
use duka_shared::dtype::{Brand, Constraint, Label, Subject, Ty, Type, Units};
use duka_shared::effects::{EFFECT_ATOMS, Effect, EffectSet};
use duka_shared::regex;
use duka_shared::value::ConstValue;
use std::sync::{LazyLock, RwLock};

use crate::analyzer::tyval::{TypeValue, TypeValueExt};

mod patterns;

#[inline]
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

#[inline]
/// A concept carrying nothing but its own answer.
fn obligation(t: &Type) -> Option<Type> {
    concept_holds(t)
        .then_some(Type::Literal(ConstValue::Bool(true)))
        .filter(|_| matches!(t, Type::Literal(ConstValue::Bool(_))))
}

pub type TypeResult = Result<(TypeValue, Vec<Constraint>), String>;
pub type TypeFunctionBuiltin = fn(Box<[TypeValue]>) -> TypeResult;

#[inline(always)]
pub(crate) fn plain(value: TypeValue) -> TypeResult {
    Ok((value, Vec::new()))
}

#[inline]
pub fn builtin_value(name: &str) -> Option<TypeValue> {
    let bi = TYPE_BUILTINS.read().ok()?;
    bi.get(&name).cloned()
}

#[inline]
fn get_str(bytes: &[u8]) -> Result<&str, String> {
    str::from_utf8(bytes).map_err(|e| e.to_string())
}

pub(crate) fn string_arg(value: &TypeValue) -> Result<String, String> {
    match value.to_type() {
        Type::Literal(ConstValue::String(bytes)) => {
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        other => Err(format!("expected a string literal, got {other}")),
    }
}

fn unit_table(units: &Units, mt: Option<Box<TypeValue>>) -> TypeValue {
    Ty::TypeTable(
        units
            .0
            .iter()
            .map(|(name, exp)| {
                (
                    ConstValue::String(name.as_bytes().to_vec().into_boxed_slice()),
                    Box::new(tv!(Type::Literal(ConstValue::Int(*exp as i64)))),
                )
            })
            .collect(),
        mt,
    )
}

fn units_of(value: &TypeValue) -> Result<Units, String> {
    let ty = value.to_type();
    let Type::TypeTable(fields, _) = &ty else {
        return Err(format!("expected a unit, got {ty}"));
    };
    let mut items = Vec::with_capacity(fields.len());
    for (key, value) in fields.iter() {
        let ConstValue::String(name) = key else {
            return Err("a unit table is keyed by symbol names".to_owned());
        };
        let Type::Literal(ConstValue::Int(exp)) = &**value else {
            return Err("a unit table holds whole exponents".to_owned());
        };
        let exp = i32::try_from(*exp).map_err(|_| "a unit exponent is out of range".to_owned())?;
        items.push((
            String::from_utf8_lossy(name).into_owned().into_boxed_str(),
            exp,
        ));
    }
    Ok(Units::normalized(items))
}

fn unit_algebra(
    left: &TypeValue,
    right: &TypeValue,
    op: fn(&Units, &Units) -> Units,
    not_closed: bool, // division is not closed
) -> TypeResult {
    if let TypeValue::Brand(b) = left
        && let Brand {
            base,
            label: Label::Unit(unit),
        } = &**b
        && let TypeValue::Brand(b2) = right
        && let Brand {
            base: base2,
            label: Label::Unit(unit2),
        } = &**b2
        && base.is_numeric()
        && base2.is_numeric()
    {
        let float = not_closed || matches!(base, Ty::Float) || matches!(base2, Ty::Float);
        let unit = op(unit, unit2);
        return plain(Ty::Brand(Box::new(Brand {
            base: if float { Ty::Float } else { Ty::Int },
            label: Label::Unit(unit),
        })));
    }

    let a = units_of(left)?;
    let b = units_of(right)?;
    let carried = match left.to_type() {
        Type::TypeTable(_, Some(mt)) => Some(Box::new(tv!(*mt))),
        _ => match right.to_type() {
            Type::TypeTable(_, Some(mt)) => Some(Box::new(tv!(*mt))),
            _ => None,
        },
    };
    plain(unit_table(&op(&a, &b), carried))
}

fn collect_label(t: &Type, found: &mut Option<Label>) -> Result<(), String> {
    match t {
        Type::TypeTuple(items, _) => {
            for item in items {
                collect_label(item, found)?;
            }
            Ok(())
        }
        Type::Brand(b) => {
            match found {
                Some(seen) if *seen != b.label => {
                    return Err(format!("`{seen}` and `{}` are different units", b.label));
                }
                _ => *found = Some(b.label.clone()),
            }
            collect_label(&b.base, found)
        }
        Type::Array(Some(inner)) => collect_label(inner, found),
        Type::Table(_, Some(value), _) => collect_label(value, found),
        _ => Ok(()),
    }
}

fn collect_numbers(t: &Type, wide: &mut Option<Type>) -> Option<()> {
    match t.base() {
        Type::TypeTuple(items, _) => {
            for item in items {
                collect_numbers(item, wide)?;
            }
            Some(())
        }
        Type::Array(Some(inner)) => collect_numbers(inner, wide),
        Type::Table(_, Some(value), _) => collect_numbers(value, wide),
        Type::Int | Type::Literal(ConstValue::Int(_)) => {
            if !matches!(wide, Some(Type::Float)) {
                *wide = Some(Type::Int);
            }
            Some(())
        }
        Type::Float | Type::Literal(ConstValue::Float(_)) => {
            *wide = Some(Type::Float);
            Some(())
        }
        _ => None,
    }
}

fn widen_of(pack: &Type) -> Option<Type> {
    let mut wide = None;
    collect_numbers(pack, &mut wide)?;
    Some(wide.unwrap_or(Type::Any))
}

/// The label a single type carries, if it carries one.
fn label_of(t: &Type) -> Option<Label> {
    match t {
        Type::Brand(b) => Some(b.label.clone()),
        _ => None,
    }
}

fn with_label(base: Type, label: Option<Label>) -> Type {
    match label {
        Some(label) => Type::Brand(Box::new(Brand { base, label })),
        None => base,
    }
}

fn union_of(pack: &Type) -> Type {
    fn walk(t: &Type, out: &mut Vec<Type>) {
        match t {
            Type::TypeTuple(items, _) => {
                for item in items {
                    walk(item, out);
                }
            }
            other => out.push(other.clone()),
        }
    }
    let mut flat = Vec::new();
    walk(pack, &mut flat);
    if flat.is_empty() {
        Type::Any
    } else {
        Type::from_union_kept(flat)
    }
}

fn same_unit(v: Box<[TypeValue]>) -> TypeResult {
    let pack = v.into_iter().next().expect("Checked").to_type();
    let mut label = None;
    collect_label(&pack, &mut label)?;
    let answer = match widen_of(&pack) {
        Some(base) => with_label(base, label),
        None => union_of(&pack),
    };
    // one collection answers `T?`: it may hold nothing to pick from
    let empty_maybe = matches!(
        &pack,
        Type::TypeTuple(items, _) if items.len() == 1
            && matches!(
                items[0].base(),
                Type::Array(_) | Type::Table(..) | Type::TypeTuple(..)
            )
    );
    plain(tv!(if empty_maybe {
        answer | Type::Nil
    } else {
        answer
    }))
}

fn unit_of(v: Box<[TypeValue]>) -> TypeResult {
    let mut v = v.into_iter();
    let source = v.next().expect("Checked").to_type();
    let base = v.next().expect("Checked").to_type();
    let mut label = None;
    collect_label(&source, &mut label)?;
    plain(tv!(with_label(base, label)))
}

fn sqrt_of(v: Box<[TypeValue]>) -> TypeResult {
    let source = v.into_iter().next().expect("Checked").to_type();
    match label_of(&source) {
        None => plain(tv!(Type::Float)),
        Some(Label::Nominal(name)) => Err(format!(
            "the square root of `{name}` has no meaning: a name is not a quantity"
        )),
        Some(Label::Unit(units)) => {
            for (name, exp) in units.0.iter() {
                if exp % 2 != 0 {
                    return Err(format!(
                        "the square root of `{units}` has no whole exponent for `{name}`"
                    ));
                }
            }
            let halved: Vec<(Box<str>, i32)> = units
                .0
                .iter()
                .map(|(name, exp)| (name.clone(), exp / 2))
                .collect();
            plain(tv!(Type::Brand(Box::new(Brand {
                base: Type::Float,
                label: Label::Unit(Units(halved.into_boxed_slice())),
            }))))
        }
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

fn effects_table(effects: Option<&EffectSet>) -> Type {
    let mut fields: Vec<(ConstValue, Box<Type>)> = vec![];
    if let Some(es) = effects {
        for atom in EFFECT_ATOMS {
            if es.yes_has(*atom) {
                fields.push((
                    ConstValue::String(atom.name().as_bytes().into()),
                    Box::new(Type::Literal(ConstValue::Bool(true))),
                ));
            } else if es.no_has(*atom) {
                fields.push((
                    ConstValue::String(atom.name().as_bytes().into()),
                    Box::new(Type::Literal(ConstValue::Bool(false))),
                ));
            }
        }
    }
    Type::TypeTable(fields, None)
}

fn effects_of(v: &TypeValue) -> Result<Option<EffectSet>, String> {
    let mut set = EffectSet::default();
    let fields = match v.to_type() {
        Type::Nil => return Ok(None),
        Type::TypeTable(fields, _) => fields,
        other => return Err(format!("expected an effects table, got {other}")),
    };
    for (key, value) in fields.iter() {
        let ConstValue::String(name) = key else {
            return Err("an effects table is keyed by effect names".to_owned());
        };
        let text = String::from_utf8_lossy(name);
        let Some(atom) = Effect::parse(&text) else {
            return Err(format!("`{text}` is not an effect name"));
        };
        let Type::Literal(ConstValue::Bool(on)) = &**value else {
            return Err(format!(
                "the effect `{text}` wants a bool, got {}",
                &**value
            ));
        };
        if *on {
            let _ = set.with(atom);
        } else {
            let _ = set.deny(atom);
        }
    }
    Ok((set != EffectSet::default()).then_some(set))
}

/// One effect by its name.
fn effect_atom(name: &str) -> Result<Effect, String> {
    Effect::parse(name).ok_or_else(|| format!("`{name}` is not an effect name"))
}

/// Whether an effect set has what a policy asks: an effect the policy turns on
/// is met when the set holds it, one it denies is met when the set denies it,
/// and one the policy leaves out is not looked at. A set that says nothing
/// about an effect does not meet it.
fn effects_meet(actual: Option<&EffectSet>, want: Option<&EffectSet>) -> bool {
    let (Some(actual), Some(want)) = (actual, want) else {
        return want.is_none();
    };
    EFFECT_ATOMS
        .iter()
        .all(|atom| match (want.yes_has(*atom), want.no_has(*atom)) {
            (true, _) => actual.yes_has(*atom),
            (_, true) => actual.no_has(*atom),
            _ => true,
        })
}

duka_builtin_type_def! {
    type mod type_context
    name "type-context"
    doc "Builtins for type-context"
    flags(@feature("type-context"))

    fn {
        #[duka_builtin(doc = "Throw an error with a message", count = 1)]
        fn Error(v: Box<[TypeValue]>) -> TypeResult {
            Err(v[0].to_string())
        },

        #[duka_builtin(doc = "If A isn't true, throws an error with message B", count = 2)]
        fn Assert(v: Box<[TypeValue]>) -> TypeResult {
            let ty = v[0].to_type();
            let value = if matches!(ty, Type::Literal(ConstValue::Bool(true))) {
                tv!(ty)
            } else {
                return Err(v[1].to_string());
            };
            plain(value)
        },

        #[duka_builtin(doc = "Stringify a type", count = 1)]
        fn Stringify(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                Type::Literal(ConstValue::String(
                    v[0].to_string().into_bytes().into_boxed_slice()
                ))
            ))
        },

        #[duka_builtin(doc = "Pack types from type array into a union type", count = 1)]
        fn Union(v: Box<[TypeValue]>) -> TypeResult {
            let t = v[0].to_type();
            if let Type::TypeTuple(types, rest) = t {
                let mut types = types;
                if let Some(tail) = rest {
                    types.push(*tail);
                }
                plain(tv!(Type::Union(types.into_boxed_slice())))
            } else {
                plain(v.into_iter().next().expect("Checked"))
            }
        },

        #[duka_builtin(doc = "Unpack a union type", count = 1)]
        fn Unpack(v: Box<[TypeValue]>) -> TypeResult {
            let t = v[0].to_type();
            if let Type::Union(types) = t {
                plain(tv!(Type::TypeTuple(types.into_vec(), None)))
            } else {
                plain(v.into_iter().next().expect("Checked"))
            }
        },

        #[duka_builtin(doc = "The element type of an array, table or tuple type", count = 1)]
        fn Elem(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(match v[0].to_type() {
                Type::Array(Some(inner)) => *inner,
                Type::Array(None) => Type::Any,
                Type::Table(_, Some(value), _) => *value,
                Type::Table(_, None, _) => Type::Any,
                Type::TypeTuple(items, rest) => items
                    .into_iter()
                    .chain(rest.map(|t| *t))
                    .reduce(|a, b| a | b)
                    .unwrap_or(Type::Any),
                // an iterator's yields are not a part of its type here
                Type::Function(_) => Type::Any,
                other => other,
            }))
        },

        #[duka_builtin(doc = "Whether A is an iterable type value", count = 1)]
        fn IsIterable(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                Type::Literal(ConstValue::Bool(
                    matches!(v[0].as_type(), Some(
                        Type::TypeTable(..) | Type::TypeTuple(..) | Type::Object { .. }
                    ))
                ))
            ))
        },

        #[duka_builtin(doc = "Whether B is a sub type of A", count = 2)]
        fn IsSubType(v: Box<[TypeValue]>) -> TypeResult {
            let constraints = vec![Constraint::Sub(Subject::Arg(1), v[1].to_type())];
            Ok((
                tv!(Type::Literal(ConstValue::Bool(v[1].accepts(&v[0])))),
                constraints,
            ))
        },

        #[duka_builtin(doc = "A type with nil taken out of it", count = 1)]
        fn NonNil(v: Box<[TypeValue]>) -> TypeResult {
            let constraints = vec![Constraint::IsNot(Subject::Arg(0), Type::Nil)];
            Ok((tv!(v[0].to_type().nonnilable()), constraints))
        },

        #[duka_builtin(doc = "Get the super type of a type value in type context", count = 1)]
        fn Super(v: Box<[TypeValue]>) -> TypeResult {
            let ty = v.into_iter().next().expect("Checked");
            plain(tv!(match ty.to_type() {
                Type::Literal(l) => l.type_of(),
                Type::TypeTable(..) => Type::Table(None, None, None),
                Type::TypeTuple(..) => Type::Array(None),
                Type::Param(..) => Type::Any,
                a => a
            }))
        },

        #[duka_builtin(doc = "Whether A is in B", count = 2)]
        fn In(v: Box<[TypeValue]>) -> TypeResult {
            if let Some(other) = &v[0].as_type() {
                if let Some(Type::Union(types)) = &v[1].as_type()  {
                    return plain(tv!(Type::Literal(ConstValue::Bool(types.contains(other)))))
                }
                else if let Some(Type::TypeTuple(types, _)) = &v[1].as_type() {
                    return plain(tv!(Type::Literal(ConstValue::Bool(types.contains(other)))))
                }
                else if let Some(Type::TypeTable(types, _)) = &v[1].as_type() &&
                    let Type::Literal(c) = other
                {
                    return plain(tv!(Type::Literal(ConstValue::Bool(types.iter().any(|i| &i.0 == c)))))
                }
            }
            plain(tv!(Type::Literal(ConstValue::Bool(false))))
        },

        #[duka_builtin(doc = "Whether the function type has var-arg parameter", count = 1, bind(0: Type::Function(None)))]
        fn HasVarArg(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
                Type::Literal(ConstValue::Bool(ft.var_arg))
            } else {
                Type::Literal(ConstValue::Bool(false))
            }))
        },

        #[duka_builtin(doc = "Whether the function type has var-arg returns", count = 1, bind(0: Type::Function(None)))]
        fn HasRetVarArg(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
                Type::Literal(ConstValue::Bool(ft.return_var_arg.is_some()))
            } else {
                Type::Literal(ConstValue::Bool(false))
            }))
        },

        #[duka_builtin(doc = "Build a function type", count = 4)]
        fn BuildFunction(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let args = v.next().expect("Checked");
            let var_arg = v.next().expect("Checked");
            let rets = v.next().expect("Checked");
            let var_ret = v.next().expect("Checked");

            fn deal(tv: TypeValue) -> Box<[Type]> {
                match tv.without_tag() {
                    tv!(ty) => match ty {
                        Type::TypeTuple(t, _) => t.into_boxed_slice(),
                        Type::Never => [].into(),
                        other => [other].into()
                    },
                    _ => [].into()
                }
            }

            plain(
                tv!(Type::Function(Some(
                    FunctionType {
                        params: deal(args),
                        var_arg: matches!(var_arg.without_tag(), tv!(Type::Literal(ConstValue::Bool(true)))),
                        returns: deal(rets),
                        return_var_arg : matches!(var_ret.without_tag(), tv!(Type::Literal(ConstValue::Bool(true)))).then(|| Box::new(Type::Any)),
                        ..Default::default()
                    }
                )))
            )
        },

        #[duka_builtin(doc = "Remove B in union or type array A", count = 2)]
        fn Exclude(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked");
            let second = v.next().expect("Checked");
            if let Some(t) = second.as_type() {
                match first {
                    tv!(Type::Union(types)) if types.contains(t) => {
                        let kept: Vec<Type> = types.into_iter().filter(|i| i != t).collect();
                        return plain(tv!(Type::from_union_kept(kept)))
                    },
                    tv!(Type::TypeTuple(types, _)) if types.contains(t) => {
                        let kept: Vec<Type> = types.into_iter().filter(|i| i != t).collect();
                        return plain(tv!(Type::from_tuple_kept(kept)))
                    },
                    _ => ()
                }
            }
            plain(first)
        },

        #[duka_builtin(doc = "Return true if literal string type A ends with B", count = 2)]
        fn EndsWith(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(end))) = &v[1].as_type() {
                    let tar = get_str(tar)?;
                    let end = get_str(end)?;
                    Type::Literal(ConstValue::Bool(tar.ends_with(end)))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                },
            ))
        },

        #[duka_builtin(doc = "Return true if literal string type A starts with B", count = 2)]
        fn StartsWith(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(start))) = &v[1].as_type() {
                    let tar = get_str(tar)?;
                    let start = get_str(start)?;
                    Type::Literal(ConstValue::Bool(tar.starts_with(start)))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                },
            ))
        },

        #[duka_builtin(doc = "Remove an element in type tuple or type table by its index or key", count = 2)]
        fn Remove(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked").without_tag();
            let second = v.next().expect("Checked").without_tag();
            plain(match (first, second) {
                (tv!(Type::TypeTuple(mut t, rest)), tv!(Type::Literal(ConstValue::Int(idx)))) => {
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
                    tv!(Type::TypeTuple(t, rest))
                },
                (tv!(Type::TypeTable(mut t, mt)), tv!(Type::Literal(key))) => {
                    t.retain(|v| v.0 != key);
                    tv!(Type::TypeTable(t, mt))
                },
                (other, _) => other
            })
        },

        #[duka_builtin(doc = "Insert new element to the head of a type tuple or type table", count = 2)]
        fn AppendHead(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked").without_tag();
            let second = v.next().expect("Checked");
            plain(match first {
                tv!(Type::TypeTuple(mut t, rest)) => {
                    t.insert(0, second.to_type());
                    tv!(Type::TypeTuple(t, rest))
                },
                tv!(Type::TypeTable(mut t, mt)) => {
                    if let Some(Type::Literal(cv)) = second.as_type() {
                        let val = v.next().unwrap_or_default().to_type();
                        t.insert(0, (cv.clone(), Box::new(val)));
                    }
                    tv!(Type::TypeTable(t, mt))
                },
                other => other
            })
        },

        #[duka_builtin(doc = "Append new element to type tuple or type table", count = 2)]
        fn Append(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked").without_tag();
            let second = v.next().expect("Checked");
            plain(match first {
                tv!(Type::TypeTuple(mut t, rest)) => {
                    t.push(second.to_type());
                    tv!(Type::TypeTuple(t, rest))
                },
                tv!(Type::TypeTable(mut t, mt)) => {
                    if let Some(Type::Literal(cv)) = second.as_type() {
                        let val = v.next().unwrap_or_default().to_type();
                        t.push((cv.clone(), Box::new(val)));
                    }
                    tv!(Type::TypeTable(t, mt))
                },
                other => other
            })
        },

        #[duka_builtin(doc = "Slice string literal type A with start index B and optional end index C", count = 2)]
        fn Slice(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::Int(start))) = &v[1].as_type() {
                    let start = *start as usize;
                    let tar = get_str(tar)?;
                    Type::Literal(ConstValue::String(if let Some(tv!(Type::Literal(ConstValue::Int(end)))) = v.get(2) {
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

        #[duka_builtin(doc = "Split string literal type A by separator string literal type B", count = 2)]
        fn Split(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(sep))) = &v[1].as_type() {
                    let tar = get_str(tar)?;
                    let sep = get_str(sep)?;
                    Type::TypeTuple(
                        tar.split(sep).map(|i| Type::Literal(ConstValue::String(i.as_bytes().into()))).collect(),
                        None,
                    )
                } else {
                    return plain(v.into_iter().next().expect("Checked"))
                },
            ))
        },

        #[duka_builtin(doc = "Convert a string literal type into uppercase", count = 1)]
        fn Uppercase(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                    Type::Literal(ConstValue::String(
                        str.iter().map(|b| b.to_ascii_uppercase()).collect(),
                    ))
                } else {
                    return plain(v.into_iter().next().expect("Checked"))
                },
            ))
        },

        #[duka_builtin(doc = "Capitalize a string literal type", count = 1)]
        fn Capitalize(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                    Type::Literal(ConstValue::String(
                        str.iter().enumerate().map(|(i, b)| if i == 0 { b.to_ascii_uppercase() } else { *b }).collect(),
                    ))
                } else {
                    return plain(v.into_iter().next().expect("Checked"))
                },
            ))
        },

        #[duka_builtin(doc = "Uncapitalize a string literal type", count = 1)]
        fn Uncapitalize(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                    Type::Literal(ConstValue::String(
                        str.iter().enumerate().map(|(i, b)| if i == 0 { b.to_ascii_lowercase() } else { *b }).collect(),
                    ))
                } else {
                    return plain(v.into_iter().next().expect("Checked"))
                },
            ))
        },

        #[duka_builtin(doc = "Convert a string literal type into lowercase", count = 1)]
        fn Lowercase(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(
                if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                    Type::Literal(ConstValue::String(
                        str.iter().map(|b| b.to_ascii_lowercase()).collect(),
                    ))
                } else {
                    return plain(v.into_iter().next().expect("Checked"))
                },
            ))
        },

        #[duka_builtin(doc = "Search a RegEx pattern in a string literal type", count = 2)]
        fn Regex(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked");
            let second = v.next().expect("Checked");
            if let Type::Literal(ConstValue::String(pat)) = first.to_type() && let Type::Literal(ConstValue::String(who)) = second.to_type() {
                let pat = get_str(&pat)?;
                let who2 = get_str(&who)?;
                let mac = regex::Runner::new(&regex::compile(pat).map_err(|e| e.to_string())?).search(who2, 0);
                plain(tv!(Type::TypeTuple(
                    mac.map(|v| v.captures.into_iter().map(|i| Type::Literal(ConstValue::String(who[i.0..i.1].into()))).collect()).unwrap_or_default(),
                    None,
                )))
            } else {
                plain(second)
            }
        },

        #[duka_builtin(doc = "Whether A is a member of union, tuple or record B", count = 2)]
        fn NotIn(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked").to_type();
            let second = v.next().expect("Checked").to_type();
            let Some(members) = member_types(&second) else {
                return Ok((
                    tv!(Type::Never),
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
            Ok((tv!(value), constraints))
        },

        #[duka_builtin(doc = "Narrow A when A has a member called B", count = 2)]
        fn Has(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let first = v.next().expect("Checked").to_type();
            let second = v.next().expect("Checked").to_type();
            let Type::Literal(ConstValue::String(name)) = second else {
                return plain(tv!(Type::Any));
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
            Ok((tv!(value), constraints))
        },

        #[duka_builtin(doc = "A if it answers, otherwise B -- one of the two, whichever answers", count = 2)]
        fn Any(v: Box<[TypeValue]>) -> TypeResult {
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
            plain(tv!(if concept_holds(&narrowed) {
                narrowed
            } else {
                Type::Never
            }))
        },

        #[duka_builtin(doc = "Narrow A when both answer -- either side may be the one that narrowed", count = 2)]
        fn All(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let a = v.next().expect("Checked").to_type();
            let b = v.next().expect("Checked").to_type();
            if !concept_holds(&a) || !concept_holds(&b) {
                return plain(tv!(Type::Never));
            }
            plain(tv!(match (obligation(&a), obligation(&b)) {
                (Some(_), None) => b,
                (None, Some(_)) => a,
                _ => a & b,
            }))
        },

        #[duka_builtin(doc = "Whether given type is a type tuple", count = 1)]
        fn IsTuple(v: Box<[TypeValue]>) -> TypeResult {
            plain(tv!(Type::Literal(ConstValue::Bool(matches!(v.into_iter().next().expect("Checked").to_type(), Type::TypeTuple(..))))))
        },

        #[duka_builtin(doc = "Hand a type back while keeping the constraints its evaluation contributed", count = 1)]
        fn provide(v: Box<[TypeValue]>) -> TypeResult {
            plain(v.into_iter().next().expect("Checked"))
        },

        #[duka_builtin(doc = "Narrow A when C does not answer -- impossible once C does", count = 2)]
        fn Not(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let a = v.next().expect("Checked").to_type();
            let c = v.next().expect("Checked").to_type();
            plain(tv!(if concept_holds(&c) {
                Type::Never
            } else {
                a
            }))
        },

        #[duka_builtin(doc = "Whether the given type tuple is checking types loosely (etc [bool, string, ...] returns true)", count = 1)]
        fn IsLoosely(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let t = v.next().expect("Checked").to_type();
            plain(tv!(
                Type::Literal(ConstValue::Bool(
                    matches!(t, Type::TypeTuple(_, Some(_)))
                ))
            ))
        },

        #[duka_builtin(doc = "Give T the nominal label NAME, `int ::len::`", count = 2)]
        fn WithBrand(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let base = v.next().expect("Checked").to_type();
            let name = string_arg(&v.next().expect("Checked"))?;
            if name.is_empty() || name.contains("::") {
                return Err("WithBrand wants a non-empty name without `::`".to_owned());
            }
            plain(tv!(Type::Brand(Box::new(Brand {
                base,
                label: Label::Nominal(name.into()),
            }))))
        },

        #[duka_builtin(doc = "Get metatable type", count = 1)]
        fn GetMetatable(v: Box<[TypeValue]>) -> TypeResult {
            if let Ty::TypeTable(_, mt) = v[0].untag() {
                return plain(mt.as_deref().cloned().unwrap_or(Ty::Nil));
            }
            let t = v[0].to_type();
            plain(tv!(match t {
                Type::TypeTable(_, mt) => mt.as_deref().cloned().unwrap_or_default(),
                Type::Table(.., mt) => mt.as_deref().cloned().unwrap_or_default(),
                Type::Object { mt, .. } => mt.as_deref().cloned().unwrap_or_default(),
                other => {
                    return Err(format!(
                        "GetMetatable takes a table or object type, got {}",
                        other
                    ))
                }
            }))
        },

        #[duka_builtin(doc = "Parse unit text into its normalized exponent table", count = 1)]
        fn ParseUnit(v: Box<[TypeValue]>) -> TypeResult {
            let text = string_arg(&v[0])?;
            let units = Units::parse(&text)?;
            plain(unit_table(&units, None))
        },

        #[duka_builtin(doc = "Give a number the unit a unit name stands for", count = 2)]
        fn WithUnit(v: Box<[TypeValue]>) -> TypeResult {
            let base = v[0].to_type();
            if !matches!(base.base(), Type::Int | Type::Float) {
                return Err(format!("WithUnit needs a number, got {base}"));
            }
            let units = units_of(&v[1])?;
            if units.is_empty() {
                return plain(tv!(base));
            }
            plain(tv!(Type::Brand(Box::new(Brand {
                base,
                label: Label::Unit(units),
            }))))
        },

        #[duka_builtin(doc = "The unit A·B", count = 2)]
        fn MulUnit(v: Box<[TypeValue]>) -> TypeResult {
            unit_algebra(&v[0], &v[1], Units::mul, false)
        },

        #[duka_builtin(doc = "The unit A/B", count = 2)]
        fn DivUnit(v: Box<[TypeValue]>) -> TypeResult {
            unit_algebra(&v[0], &v[1], Units::div, true)
        },

        #[duka_builtin(doc = "The unit A^n, n a whole constant", count = 2)]
        fn PowUnit(v: Box<[TypeValue]>) -> TypeResult {
            let units = units_of(&v[0])?;
            let n = match v[1].to_type() {
                Type::Literal(ConstValue::Int(n)) => n,
                other => return Err(format!("a unit power needs a whole constant, got {other}")),
            };
            let Ok(n) = i32::try_from(n) else {
                return Err("a unit exponent is out of range".to_owned());
            };
            let carried = match v[0].to_type() {
                Type::TypeTable(_, mt) => mt.map(|m| Box::new(tv!(*m))),
                _ => None,
            };
            plain(unit_table(&units.pow(n), carried))
        },

        #[duka_builtin(doc = "The one unit a pack of numbers shares, on their widened type", count = 1)]
        fn SameUnit(v: Box<[TypeValue]>) -> TypeResult { same_unit(v) },

        #[duka_builtin(doc = "The unit T carries, worn by B", count = 2)]
        fn UnitOf(v: Box<[TypeValue]>) -> TypeResult { unit_of(v) },

        #[duka_builtin(doc = "The square root of T, halving its exponents", count = 1)]
        fn Sqrt(v: Box<[TypeValue]>) -> TypeResult { sqrt_of(v) },

        #[duka_builtin(doc = "The effects a function type carries, as a table of `true` for held and `false` for denied", flags(@feature(effects)), count = 1, bind(0: Type::Function(None)))]
        fn GetEffects(v: Box<[TypeValue]>) -> TypeResult {
            let v = v.into_iter().next().expect("Checked");
            plain(tv!(
                match v.to_type() {
                    Type::Function(Some(f)) => effects_table(f.effects.as_ref()),
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Fold an effects table into a function type's own effects", flags(@feature(effects)), count = 2, bind(0: Type::Function(None)))]
        fn MergeEffects(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let f = v.next().expect("Checked");
            let es = effects_of(&v.next().expect("Checked"))?;
            plain(tv!(
                match f.to_type() {
                    Type::Function(Some(mut ft)) => {
                        let mut written = ft.effects.unwrap_or_default();
                        written.merge(&es.unwrap_or_default());
                        ft.effects = (written != EffectSet::default()).then_some(written);
                        Type::Function(Some(ft))
                    }
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Give a function type exactly the effects a table describes, nil or an empty table takes them off", flags(@feature(effects)), count = 2, bind(0: Type::Function(None)))]
        fn SetEffects(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let f = v.next().expect("Checked");
            let es = effects_of(&v.next().expect("Checked"))?;
            plain(tv!(
                match f.to_type() {
                    Type::Function(Some(mut ft)) => {
                        ft.effects = es;
                        Type::Function(Some(ft))
                    }
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Whether a function type has the effects a policy asks for: `true` one it must hold, `false` one it must not hold, one left out is not looked at", flags(@feature(effects)), count = 2, bind(0: Type::Function(None)))]
        fn HasEffects(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let f = v.next().expect("Checked");
            let policy = effects_of(&v.next().expect("Checked"))?;
            plain(tv!(
                match f.to_type() {
                    Type::Function(Some(f)) => Type::Literal(ConstValue::Bool(effects_meet(
                        f.effects.as_ref(),
                        policy.as_ref()
                    ))),
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Give a function type one effect, taking it off the ones it denies", flags(@feature(effects)), count = 2, bind(0: Type::Function(None)), bind(1: Type::String))]
        fn WithEffect(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let f = v.next().expect("Checked");
            let atom = effect_atom(&string_arg(&v.next().expect("Checked"))?)?;
            let bit = atom as u8;
            plain(tv!(
                match f.to_type() {
                    Type::Function(Some(mut ft)) => {
                        let mut es = ft.effects.unwrap_or_default();
                        es.yes |= bit;
                        es.no &= !bit;
                        ft.effects = Some(es);
                        Type::Function(Some(ft))
                    }
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Deny a function type one effect, taking it off the ones it holds", flags(@feature(effects)), count = 2, bind(0: Type::Function(None)), bind(1: Type::String))]
        fn WithoutEffect(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let f = v.next().expect("Checked");
            let atom = effect_atom(&string_arg(&v.next().expect("Checked"))?)?;
            let bit = atom as u8;
            plain(tv!(
                match f.to_type() {
                    Type::Function(Some(mut ft)) => {
                        let mut es = ft.effects.unwrap_or_default();
                        es.yes &= !bit;
                        es.no |= bit;
                        ft.effects = Some(es);
                        Type::Function(Some(ft))
                    }
                    _ => Type::Nil
                }
            ))
        },

        #[duka_builtin(doc = "Attach metatable MT to T, or take it off when MT is nil", count = 2)]
        fn WithMetatable(v: Box<[TypeValue]>) -> TypeResult {
            let mt = match v[1].untag() {
                Ty::Nil => None,
                Ty::Ext(crate::analyzer::tyval::TypeLeaf::Final(Type::Nil)) => None,
                other => Some(Box::new(other.clone())),
            };
            if let Ty::TypeTable(fields, _) = v[0].untag() {
                return plain(Ty::TypeTable(fields.clone(), mt));
            }
            let t = v[0].to_type();
            let mt = mt.map(|m| Box::new(m.to_type()));
            plain(tv!(match t {
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
        },
    }

    mod { patterns }
}

pub static TYPE_BUILTINS: GlobalBuiltins<TypeValue> = LazyLock::new(|| RwLock::new(registry()));

pub use self::TYPE_MODULE_META as TYPE_BUILTINS_META;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::tyval::{TypeClosure, TypeLeaf, tv};

    fn apply(name: &str, args: Box<[Type]>) -> Result<Type, String> {
        let value = builtin_value(name).ok_or_else(|| "missing".to_owned())?;
        let Ty::Ext(TypeLeaf::Closure(c)) = value else {
            return Err("not a function".to_owned());
        };
        let TypeClosure::Rust { func, .. } = *c else {
            return Err("not a function".to_owned());
        };
        let args = args
            .into_iter()
            .map(tv)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        func(args).map(|(v, _)| v.to_type())
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

    #[test]
    fn effects_read_write_and_merge_through_tables() {
        use duka_shared::effects::{Effect, EffectSet};

        fn str_key(name: &str) -> ConstValue {
            ConstValue::String(name.as_bytes().into())
        }

        let held = FunctionType {
            effects: Some(EffectSet {
                yes: Effect::Mutable as u8,
                no: Effect::Throw as u8,
            }),
            ..Default::default()
        };
        assert_eq!(
            apply("GetEffects", [Type::Function(Some(held.clone()))].into())
                .expect("effects read back"),
            Type::TypeTable(
                vec![
                    (
                        str_key("mutable"),
                        Box::new(Type::Literal(ConstValue::Bool(true)))
                    ),
                    (
                        str_key("throw"),
                        Box::new(Type::Literal(ConstValue::Bool(false)))
                    ),
                ],
                None
            )
        );

        let cleared = apply(
            "SetEffects",
            [Type::Function(Some(held.clone())), Type::Nil].into(),
        )
        .expect("nil takes the effects off");
        let Type::Function(Some(ft)) = cleared else {
            panic!("expected a function type back, got {cleared}");
        };
        assert_eq!(ft.effects, None);

        let merged = apply(
            "MergeEffects",
            [
                Type::Function(Some(held)),
                Type::TypeTable(
                    vec![(
                        str_key("throw"),
                        Box::new(Type::Literal(ConstValue::Bool(true))),
                    )],
                    None,
                ),
            ]
            .into(),
        )
        .expect("merge folds the table in");
        let Type::Function(Some(ft)) = merged else {
            panic!("expected a function type back, got {merged}");
        };
        let mut want = EffectSet {
            yes: Effect::Mutable as u8,
            no: Effect::Throw as u8,
        };
        want.merge(&EffectSet {
            yes: Effect::Throw as u8,
            no: 0,
        });
        assert_eq!(ft.effects, Some(want));

        let bad = Type::TypeTable(
            vec![(
                str_key("explode"),
                Box::new(Type::Literal(ConstValue::Bool(true))),
            )],
            None,
        );
        assert!(
            apply(
                "SetEffects",
                [Type::Function(Some(FunctionType::default())), bad].into()
            )
            .is_err(),
            "an unknown effect name is an error"
        );
    }

    #[test]
    fn effects_turn_single_atoms_and_meet_a_policy() {
        use duka_shared::effects::{Effect, EffectSet};

        fn name_arg(name: &str) -> Type {
            Type::Literal(ConstValue::String(name.as_bytes().into()))
        }

        fn field(name: &str, on: bool) -> (ConstValue, Box<Type>) {
            (
                ConstValue::String(name.as_bytes().into()),
                Box::new(Type::Literal(ConstValue::Bool(on))),
            )
        }

        let held = Type::Function(Some(FunctionType {
            effects: Some(EffectSet {
                yes: Effect::Mutable as u8,
                no: Effect::Throw as u8,
            }),
            ..Default::default()
        }));

        let turned_on = apply("WithEffect", [held.clone(), name_arg("throw")].into())
            .expect("an effect turns on");
        let Type::Function(Some(ft)) = turned_on else {
            panic!("expected a function type back, got {turned_on}");
        };
        assert_eq!(
            ft.effects,
            Some(EffectSet {
                yes: Effect::Mutable as u8 | Effect::Throw as u8,
                no: 0,
            })
        );

        let turned_off = apply("WithoutEffect", [held.clone(), name_arg("mutable")].into())
            .expect("an effect turns off");
        let Type::Function(Some(ft)) = turned_off else {
            panic!("expected a function type back, got {turned_off}");
        };
        assert_eq!(
            ft.effects,
            Some(EffectSet {
                yes: 0,
                no: Effect::Mutable as u8 | Effect::Throw as u8,
            })
        );

        let meets = |policy: Vec<(ConstValue, Box<Type>)>| {
            apply(
                "HasEffects",
                [held.clone(), Type::TypeTable(policy, None)].into(),
            )
            .expect("policy checked")
        };
        assert_eq!(
            meets(vec![field("mutable", true)]),
            Type::Literal(ConstValue::Bool(true))
        );
        assert_eq!(
            meets(vec![field("throw", false)]),
            Type::Literal(ConstValue::Bool(true))
        );
        assert_eq!(
            meets(vec![field("throw", true)]),
            Type::Literal(ConstValue::Bool(false))
        );
        assert_eq!(
            meets(vec![field("external", true)]),
            Type::Literal(ConstValue::Bool(false))
        );
        assert_eq!(meets(vec![]), Type::Literal(ConstValue::Bool(true)));

        assert_eq!(
            apply("HasEffects", [held.clone(), Type::Nil].into()).expect("no policy, no claim"),
            Type::Literal(ConstValue::Bool(true))
        );
        let unknown = Type::Function(Some(FunctionType::default()));
        assert_eq!(
            apply("HasEffects", [unknown.clone(), Type::Nil].into()).expect("empty policy met"),
            Type::Literal(ConstValue::Bool(true))
        );
        assert_eq!(
            apply(
                "HasEffects",
                [
                    unknown,
                    Type::TypeTable(vec![field("external", true)], None)
                ]
                .into()
            )
            .expect("policy checked"),
            Type::Literal(ConstValue::Bool(false))
        );

        assert!(apply("WithEffect", [held.clone(), name_arg("explode")].into()).is_err());
        assert!(apply("WithEffect", [held.clone(), Type::Int].into()).is_err());
    }

    #[test]
    fn every_meta_name_is_in_the_registry() {
        fn walk(metas: &[duka_shared::docs::MetaInfo], out: &mut Vec<String>) {
            for m in metas {
                if let duka_shared::docs::MetaItemInfo::Module { inner } = &m.info {
                    walk(inner, out);
                } else {
                    out.push(m.name.to_owned());
                }
            }
        }
        let mut names = vec![];
        if let duka_shared::docs::MetaItemInfo::Module { inner } = &TYPE_BUILTINS_META.info {
            walk(inner, &mut names);
        }
        assert!(!names.is_empty());
        for name in names {
            assert!(
                builtin_value(&name).is_some(),
                "`{name}` is in the meta but not in the registry"
            );
        }
    }
}
