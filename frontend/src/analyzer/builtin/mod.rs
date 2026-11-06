use duka_shared::builtin::Builtins;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::docs::{MetaInfo, MetaItemInfo};
use duka_shared::dtype::FunctionType;
use duka_shared::dtype::{
    Brand, CmpOp, Constraint, Label, NumPred, Pattern, Pred, Subject, TextPred, Ty, Type, Units,
};
use duka_shared::regex;
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use std::sync::LazyLock;
use std::sync::RwLock;

use crate::analyzer::tyval::{TypeClosure, TypeValue, TypeValueExt};

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
fn plain(value: TypeValue) -> TypeResult {
    Ok((value, Vec::new()))
}

macro_rules! type_functions {
    ($(
        #[duka_builtin(doc = $doc: literal)]
        $name: literal($p: ident [$count: literal] $(, $for: literal: $bound: expr)*) $body: block
    ),* $(,)?
    $(patterns {
        $(
            #[duka_builtin(doc = $pdoc: literal)]
            $pname: literal($pp: ident [$pcount: literal] $(, $pfor: literal: $pbound: expr)*) $pbody: block
        ),* $(,)?
    })?
    $(;
        $(
            #[duka_builtin(doc = $vdoc: literal)]
            $vname: literal = $vval: expr
        ),* $(,)?
    )?) => {
        pub static TYPE_BUILTINS: GlobalBuiltins<TypeValue> = LazyLock::new(|| {
            RwLock::new(
                Builtins::<TypeValue>::new()
                    $(.register(
                        $name,
                        closure!(Box::new(TypeClosure::rust($name, |$p| {
                            ($p.len() < $count).then_error(|| concat!("Expected ", stringify!($count)," type argument").to_owned())?;
                            $(($bound.accepts(&$p[$for].to_type())).or_else_error(|| concat!("Expected ", stringify!($bound), " for the type argument at ", stringify!($for)).to_owned())?;)*
                            $body
                        })))
                    ))*
                    $(
                        $(.register(
                            $pname,
                            closure!(Box::new(TypeClosure::rust($pname, |$pp| {
                                ($pp.len() < $pcount).then_error(|| concat!("Expected ", stringify!($pcount)," type argument").to_owned())?;
                                $(($pbound.accepts(&$pp[$pfor].to_type())).or_else_error(|| concat!("Expected ", stringify!($pbound), " for the type argument at ", stringify!($pfor)).to_owned())?;)*
                                $pbody
                            })))
                        ))*
                    )?
                    $(.register($vname, $vval))*
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
                    },)*
                    $(
                        MetaInfo {
                            name: "Patterns",
                            doc: "Pattern constructors",
                            example: None,
                            info: MetaItemInfo::Module {
                                inner: &[
                                    $(MetaInfo {
                                        name: $pname,
                                        doc: $pdoc,
                                        example: None,
                                        info: MetaItemInfo::TypeFunction {
                                            param_count: $pcount
                                        },
                                        flags: &[],
                                    },)*
                                ],
                            },
                            flags: &[],
                        },
                    )?
                    $(MetaInfo {
                        name: $vname,
                        doc: $vdoc,
                        example: None,
                        info: MetaItemInfo::Constant {
                            ty: duka_shared::docs::DocType::Base(Type::Any),
                            val: "",
                        },
                        flags: &[],
                    },)*
                ]
            },
            flags: &[("feature", &["type-context"])],
        };
    };
}

pub fn builtin_value(name: &str) -> Option<TypeValue> {
    let bi = TYPE_BUILTINS.read().ok()?;
    bi.get(&name).cloned()
}

#[inline]
fn get_str(bytes: &[u8]) -> Result<&str, String> {
    str::from_utf8(bytes).map_err(|e| e.to_string())
}

fn string_arg(value: &TypeValue) -> Result<String, String> {
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
            base: float.then_some(Ty::Float).unwrap_or(Ty::Int),
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

#[inline(always)]
fn pat_of(base: Type, pred: Pred) -> TypeResult {
    plain(tv!(Type::Pattern(Box::new(Pattern { base, pred }))))
}

fn num_pat(v: Box<[TypeValue]>, op: CmpOp) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if !base.base().is_numeric() {
        return Err(format!("this pattern needs a numeric type, got {base}"));
    }
    let value = match v.next().expect("Checked").to_type() {
        Type::Literal(cv) => cv,
        other => return Err(format!("expected a number, got {other}")),
    };
    pat_of(base, Pred::Num(NumPred::Cmp(op, value)))
}

fn text_pat(v: Box<[TypeValue]>, f: fn(Box<str>) -> TextPred) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if !matches!(base.base(), Type::String) {
        return Err(format!("this pattern needs a string type, got {base}"));
    }
    let part = string_arg(&v.next().expect("Checked"))?;
    pat_of(base, Pred::Text(f(part.into())))
}

fn len_pat(v: Box<[TypeValue]>, op: CmpOp) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if !matches!(base.base(), Type::String) {
        return Err(format!("this pattern needs a string type, got {base}"));
    }
    let n = match v.next().expect("Checked").to_type() {
        Type::Literal(ConstValue::Int(n)) => n,
        other => return Err(format!("expected a length, got {other}")),
    };
    pat_of(base, Pred::Text(TextPred::Len(op, n)))
}

fn combine_pat(v: Box<[TypeValue]>, and: bool) -> TypeResult {
    let mut v = v.into_iter();
    let left = v.next().expect("Checked").to_type();
    let right = v.next().expect("Checked").to_type();
    let (Type::Pattern(a), Type::Pattern(b)) = (&left, &right) else {
        return Err(format!("expected two patterns, got {left} and {right}"));
    };
    let pred = match (&a.pred, &b.pred) {
        (Pred::Num(x), Pred::Num(y)) => Pred::Num(if and {
            NumPred::And(vec![x.clone(), y.clone()].into())
        } else {
            NumPred::Or(vec![x.clone(), y.clone()].into())
        }),
        (Pred::Text(x), Pred::Text(y)) => Pred::Text(if and {
            TextPred::And(vec![x.clone(), y.clone()].into())
        } else {
            TextPred::Or(vec![x.clone(), y.clone()].into())
        }),
        _ => return Err("a number pattern and a text pattern cannot combine".to_owned()),
    };
    pat_of(a.base.clone(), pred)
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

fn not_pat(v: Box<[TypeValue]>) -> TypeResult {
    let inner = v.into_iter().next().expect("Checked").to_type();
    let Type::Pattern(p) = &inner else {
        return Err(format!("expected a pattern, got {inner}"));
    };
    let pred = match &p.pred {
        Pred::Num(x) => Pred::Num(NumPred::Not(Box::new(x.clone()))),
        Pred::Text(x) => Pred::Text(TextPred::Not(Box::new(x.clone()))),
    };
    pat_of(p.base.clone(), pred)
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
            tv!(ty)
        } else {
            return Err(v[1].to_string());
        };
        plain(value)
    },
    #[duka_builtin(doc = "Stringify a type")]
    "Stringify"(v[1]) {
        plain(tv!(
            Type::Literal(ConstValue::String(
                v[0].to_string().into_bytes().into_boxed_slice()
            ))
        ))
    },
    #[duka_builtin(doc = "Pack types from type array into a union type")]
    "Union"(v[1]) {
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
    #[duka_builtin(doc = "Unpack a union type")]
    "Unpack"(v[1]) {
        let t = v[0].to_type();
        if let Type::Union(types) = t {
            plain(tv!(Type::TypeTuple(types.into_vec(), None)))
        } else {
            plain(v.into_iter().next().expect("Checked"))
        }
    },
    #[duka_builtin(doc = "The element type of an array, table or tuple type")]
    "Elem"(v[1]) {
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
    #[duka_builtin(doc = "Whether A is an iterable type value")]
    "IsIterable"(v[1]) {
        plain(tv!(
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
            tv!(Type::Literal(ConstValue::Bool(v[1].accepts(&v[0])))),
            constraints,
        ))
    },
    #[duka_builtin(doc = "A type with nil taken out of it")]
    "NonNil"(v[1]) {
        let constraints = vec![Constraint::IsNot(Subject::Arg(0), Type::Nil)];
        Ok((tv!(v[0].to_type().nonnilable()), constraints))
    },
    #[duka_builtin(doc = "Get the super type of a type value in type context")]
    "Super"(v[1]) {
        let ty = v.into_iter().next().expect("Checked");
        plain(tv!(match ty.to_type() {
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
    #[duka_builtin(doc = "Whether the function type has var-arg parameter")]
    "HasVarArg"(v[1], 0: Type::Function(None)) {
        plain(tv!(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.var_arg))
        } else {
            Type::Literal(ConstValue::Bool(false))
        }))
    },
    #[duka_builtin(doc = "Whether the function type has var-arg returns")]
    "HasRetVarArg"(v[1], 0: Type::Function(None)) {
        plain(tv!(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.return_var_arg.is_some()))
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
    #[duka_builtin(doc = "Remove B in union or type array A")]
    "Exclude"(v[2]) {
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
    #[duka_builtin(doc = "Return true if literal string type A ends with B")]
    "EndsWith"(v[2]) {
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
    #[duka_builtin(doc = "Return true if literal string type A starts with B")]
    "StartsWith"(v[2]) {
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
    #[duka_builtin(doc = "Remove an element in type tuple or type table by its index or key")]
    "Remove"(v[2]) {
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
    #[duka_builtin(doc = "Insert new element to the head of a type tuple or type table")]
    "AppendHead"(v[2]) {
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
    #[duka_builtin(doc = "Append new element to type tuple or type table")]
    "Append"(v[2]) {
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
    #[duka_builtin(doc = "Slice string literal type A with start index B and optional end index C")]
    "Slice"(v[2]) {
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
    #[duka_builtin(doc = "Split string literal type A by separator string literal type B")]
    "Split"(v[2]) {
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
    #[duka_builtin(doc = "Convert a string literal type into uppercase")]
    "Uppercase"(v[1]) {
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
    #[duka_builtin(doc = "Capitalize a string literal type")]
    "Capitalize"(v[1]) {
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
    #[duka_builtin(doc = "Uncapitalize a string literal type")]
    "Uncapitalize"(v[1]) {
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
    #[duka_builtin(doc = "Convert a string literal type into lowercase")]
    "Lowercase"(v[1]) {
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
    #[duka_builtin(doc = "Search a RegEx pattern in a string literal type")]
    "Regex"(v[2]) {
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
    #[duka_builtin(doc = "Whether A is a member of union, tuple or record B")]
    "NotIn"(v[2]) {
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
    #[duka_builtin(doc = "Narrow A when A has a member called B")]
    "Has"(v[2]) {
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
        plain(tv!(if concept_holds(&narrowed) {
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
            return plain(tv!(Type::Never));
        }
        plain(tv!(match (obligation(&a), obligation(&b)) {
            (Some(_), None) => b,
            (None, Some(_)) => a,
            _ => a & b,
        }))
    },
    #[duka_builtin(doc = "Whether given type is a type tuple")]
    "IsTuple"(v[1]) {
        plain(tv!(Type::Literal(ConstValue::Bool(matches!(v.into_iter().next().expect("Checked").to_type(), Type::TypeTuple(..))))))
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
        plain(tv!(if concept_holds(&c) {
            Type::Never
        } else {
            a
        }))
    },
    #[duka_builtin(doc = "Whether the given type tuple is checking types loosely (etc [bool, string, ...] returns true)")]
    "IsLoosely"(v[1]) {
        let mut v = v.into_iter();
        let t = v.next().expect("Checked").to_type();
        plain(tv!(
            Type::Literal(ConstValue::Bool(
                matches!(t, Type::TypeTuple(_, Some(_)))
            ))
        ))
    },
    #[duka_builtin(doc = "Give T the nominal label NAME, `int ::len::`")]
    "WithBrand"(v[2]) {
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
    #[duka_builtin(doc = "Get metatable type")]
    "GetMetatable"(v[1]) {
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
    #[duka_builtin(doc = "Parse unit text into its normalized exponent table")]
    "ParseUnit"(v[1]) {
        let text = string_arg(&v[0])?;
        let units = Units::parse(&text)?;
        plain(unit_table(&units, None))
    },

    #[duka_builtin(doc = "Give a number the unit a unit name stands for")]
    "WithUnit"(v[2]) {
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

    #[duka_builtin(doc = "The unit A·B")]
    "MulUnit"(v[2]) {
        unit_algebra(&v[0], &v[1], Units::mul, false)
    },

    #[duka_builtin(doc = "The unit A/B")]
    "DivUnit"(v[2]) {
        unit_algebra(&v[0], &v[1], Units::div, true)
    },

    #[duka_builtin(doc = "The unit A^n, n a whole constant")]
    "PowUnit"(v[2]) {
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

    #[duka_builtin(doc = "The one unit a pack of numbers shares, on their widened type")]
    "SameUnit"(v[1]) { same_unit(v) },

    #[duka_builtin(doc = "The unit T carries, worn by B")]
    "UnitOf"(v[2]) { unit_of(v) },

    #[duka_builtin(doc = "The square root of T, halving its exponents")]
    "Sqrt"(v[1]) { sqrt_of(v) },

    #[duka_builtin(doc = "Attach metatable MT to T, or take it off when MT is nil")]
    "WithMetatable"(v[2]) {
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

    patterns {
        #[duka_builtin(doc = "Numbers in range [a, b)")]
        "RangePat"(v[2]) {
            let mut v = v.into_iter();
            let base = v.next().expect("Checked").to_type();
            if !base.base().is_numeric() {
                return Err(format!("this pattern needs a numeric type, got {base}"));
            }
            let a = match v.next().expect("Checked").to_type() {
                Type::Literal(cv) => cv,
                other => return Err(format!("expected a number, got {other}")),
            };
            let b = match v.next().expect("Checked").to_type() {
                Type::Literal(cv) => cv,
                other => return Err(format!("expected a number, got {other}")),
            };
            pat_of(base, Pred::Num(NumPred::And([
                NumPred::Cmp(CmpOp::Le, a),
                NumPred::Cmp(CmpOp::Gt, b)
            ].into())))
        },

        #[duka_builtin(doc = "Numbers greater than N")]
        "GtPat"(v[2]) { num_pat(v, CmpOp::Gt) },

        #[duka_builtin(doc = "Numbers greater than or equal to N")]
        "GePat"(v[2]) { num_pat(v, CmpOp::Ge) },

        #[duka_builtin(doc = "Numbers less than N")]
        "LtPat"(v[2]) { num_pat(v, CmpOp::Lt) },

        #[duka_builtin(doc = "Numbers less than or equal to N")]
        "LePat"(v[2]) { num_pat(v, CmpOp::Le) },

        #[duka_builtin(doc = "Numbers equal to N")]
        "EqPat"(v[2]) { num_pat(v, CmpOp::Eq) },

        #[duka_builtin(doc = "Numbers other than N")]
        "NePat"(v[2]) { num_pat(v, CmpOp::Ne) },

        #[duka_builtin(doc = "Strings containing PART")]
        "HasPat"(v[2]) { text_pat(v, TextPred::Has) },

        #[duka_builtin(doc = "Strings starting with PART")]
        "StartsPat"(v[2]) { text_pat(v, TextPred::Starts) },

        #[duka_builtin(doc = "Strings ending with PART")]
        "EndsPat"(v[2]) { text_pat(v, TextPred::Ends) },

        #[duka_builtin(doc = "Strings matching REGEX in full")]
        "RegexPat"(v[2]) { text_pat(v, TextPred::Regex) },

        #[duka_builtin(doc = "Strings at least N characters long")]
        "LenGePat"(v[2]) { len_pat(v, CmpOp::Ge) },

        #[duka_builtin(doc = "Strings at most N characters long")]
        "LenLePat"(v[2]) { len_pat(v, CmpOp::Le) },

        #[duka_builtin(doc = "Strings longer than N characters")]
        "LenGtPat"(v[2]) { len_pat(v, CmpOp::Gt) },

        #[duka_builtin(doc = "Strings shorter than N characters")]
        "LenLtPat"(v[2]) { len_pat(v, CmpOp::Lt) },

        #[duka_builtin(doc = "Strings exactly N characters long")]
        "LenEqPat"(v[2]) { len_pat(v, CmpOp::Eq) },

        #[duka_builtin(doc = "Both patterns hold")]
        "AndPat"(v[2]) { combine_pat(v, true) },

        #[duka_builtin(doc = "Either pattern holds")]
        "OrPat"(v[2]) { combine_pat(v, false) },

        #[duka_builtin(doc = "The pattern does not hold")]
        "NotPat"(v[1]) { not_pat(v) },
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
    use crate::analyzer::tyval::{TypeLeaf, tv};

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
}
