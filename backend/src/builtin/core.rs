use crate::builtin::arg::err;
use crate::builtin::{format_arg, get_string};
use duka_gc::Heap;
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::constants::{MetaMethod, ctype};
use duka_shared::types::ValueCount;
use duka_shared::value::{DukaFloat, DukaInt};

use crate::builtin::ensure_type;
#[cfg(feature = "docs")]
use crate::builtin::require::__DUKA_IMPL_REQUIRE_META;
use crate::builtin::require::{__DUKA_IMPL_REQUIRE_NAME, impl_require};
use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure, make_pairs_iterator};
use crate::vm::coroutine::{CoState, NativeApi};
use duka_gc::GcCell;

duka_builtin_def! {
    mod core
    fn {
        meta:
            impl_require co,
            impl_print co,
            impl_typeof,
            impl_to_string co,
            impl_to_number,
            impl_assert,
            impl_error,
            impl_is_error,
            impl_unwrap,
            impl_expect,
            impl_get_metatable,
            impl_set_metatable,
            impl_instanceof,
            impl_pairs,
            impl_ipairs,
            impl_costatus co,
            impl_try co,
            impl_clone
    }
    const {}
}

#[duka_builtin(name = "clone", doc = "Clone a value", params(val: any))]
fn impl_clone(h: &mut Heap, val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(match val {
        RuntimeValue::Array(a) => RuntimeValue::from_vec(h, a.borrow().items.clone()),
        RuntimeValue::Table(t) => {
            let t = t.borrow();
            RuntimeValue::Table(h.alloc(GcCell::new(RuntimeDukaTable {
                inner: t.inner.clone(),
                metatable: t.metatable.clone(),
            })))
        }
        _ => val,
    })
}
#[duka_builtin(name = "collect_garbage", doc = "Try to collect garbages", flags(@feature(gc)))]
fn impl_collect_garbage(api: &mut NativeApi) -> Result<(), DukaRuntimeError> {
    api.request_gc();
    Ok(())
}
#[duka_builtin(name = "curry", doc = "Bind arguments to a function partially", params(f: fn, args: vararg))]
fn impl_curry(
    h: &mut Heap,
    sv: &mut CoState,
    api: &mut NativeApi,
    f: RuntimeValue,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let _ = (sv, api);
    if !f.is_function() {
        return Ok(f);
    }
    let bound = std::rc::Rc::new(args);
    let func = f.clone();
    Ok(RuntimeValue::from_rust_closure(
        h,
        RustClosure::returns(
            move |c: &mut CoState, h: &mut Heap, api: &mut NativeApi| {
                let rest = c.take_stack_many(1, ValueCount::VarArg)?;
                let mut all: Vec<RuntimeValue> = bound.as_ref().clone();
                all.extend(rest);
                let results = c.normal_call(h, api, func.clone(), &all)?;
                for v in results {
                    c.append_stack(v)?;
                }
                Ok(ValueCount::VarArg)
            },
            Some("curried".into()),
        ),
    ))
}

#[duka_builtin(
    name = "try",
    doc = "Run a function in protected mode, results follow Result Protocol",
    params(func: fn | table, params: vararg),
)]
fn impl_try(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    func: RuntimeValue,
    params: Vec<RuntimeValue>,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    match sv.protected_call(h, api, func, &params)? {
        Ok(values) => {
            let mut out = Vec::with_capacity(values.len() + 1);
            out.push(RuntimeValue::Bool(true));
            out.extend(values);
            Ok(out)
        }
        Err(kind) => Ok(err(h, kind)),
    }
}

#[duka_builtin(name = "expect", doc = "Expect a non-nil value", params(val: any, msg: string = "Got nil value".to_owned()), returns(any))]
fn impl_expect(val: RuntimeValue, msg: String) -> Result<RuntimeValue, DukaRuntimeError> {
    if val.is_nil() {
        Err(DukaRuntimeError::Custom(msg))
    } else {
        Ok(val)
    }
}
#[duka_builtin(name = "unwrap", doc = "Unwrap a result", params(val: vararg), returns(vararg))]
fn impl_unwrap(mut val: Vec<RuntimeValue>) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    match val.as_slice() {
        [RuntimeValue::Bool(false), t] if t.is_string() => {
            Err(DukaRuntimeError::Custom(t.to_string()))
        }
        [RuntimeValue::Bool(true), ..] => {
            val.remove(0);
            Ok(val)
        }
        _ => Ok(val),
    }
}

#[duka_builtin(name = "is_error", doc = "Check if it is an error", params(val: vararg))]
fn impl_is_error(val: Vec<RuntimeValue>) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Bool(match val.as_slice() {
        [RuntimeValue::Bool(false), t] if t.is_string() => true,
        _ => false,
    }))
}

#[duka_builtin(name = "costatus", doc = "Get coroutine's status", params(coroutine: any))]
fn impl_costatus(
    api: &mut NativeApi,
    coroutine: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    ensure_type(&coroutine, ctype::COR, "costatus", 1)?;
    let RuntimeValue::Coroutine(id) = coroutine else {
        unreachable!()
    };
    Ok(RuntimeValue::from_short_str_unsafe(
        api.co_status(id).name(),
    ))
}

#[duka_builtin(name = "print", doc = "Prints to standard output", params(args: vararg))]
fn impl_print(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    args: Vec<RuntimeValue>,
) -> Result<(), DukaRuntimeError> {
    for i in 0..args.len() {
        let f = format_arg(sv, h, api, &args[i])?;
        api.write(&f)?;
        if i != args.len() - 1 {
            api.write(" ")?;
        }
    }
    api.write("\n")?;
    Ok(())
}

#[duka_builtin(name = "typeof", doc = "Get type name of value", params(val: any))]
fn impl_typeof(val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let name = val.type_name_of();
    Ok(RuntimeValue::from_short_str_unsafe(name))
}

#[duka_builtin(name = "to_string", doc = "Convert to string", params(val: any))]
fn impl_to_string(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    val: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = get_string(sv, h, api, val)?;
    Ok(RuntimeValue::from_string(h, s))
}

#[duka_builtin(name = "to_number", doc = "Convert to number", params(val: any))]
fn impl_to_number(val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(match val {
        v if v.is_number() => v,
        RuntimeValue::ShortString(..)
        | RuntimeValue::MediumString(_)
        | RuntimeValue::LongString(_) => {
            let s = format!("{}", val);
            if let Ok(n) = s.parse::<DukaInt>() {
                RuntimeValue::Int(n)
            } else if let Ok(f) = s.parse::<DukaFloat>() {
                RuntimeValue::Float(f)
            } else {
                RuntimeValue::Nil
            }
        }
        _ => RuntimeValue::Nil,
    })
}

#[duka_builtin(name = "assert", doc = "Assertion", params(cond: any, msg: string = "assertion failed".to_owned()))]
fn impl_assert(cond: RuntimeValue, msg: String) -> Result<RuntimeValue, DukaRuntimeError> {
    if !cond.eval_to_bool() {
        return Err(DukaRuntimeError::Custom(msg));
    }
    Ok(cond)
}

#[duka_builtin(name = "error", doc = "Raise an error", params(msg: string = "error".to_owned()), flags(@returns(exit)))]
fn impl_error(msg: String) -> Result<(), DukaRuntimeError> {
    Err(DukaRuntimeError::Custom(msg))
}

#[duka_builtin(name = "get_metatable", doc = "Get metatable", params(val: table))]
fn impl_get_metatable(val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let r = match val {
        RuntimeValue::Table(t) => t
            .borrow()
            .metatable
            .map(RuntimeValue::Table)
            .unwrap_or_default(),
        _ => RuntimeValue::Nil,
    };
    Ok(r)
}

#[duka_builtin(
    name = "set_metatable", doc = "Set metatable", 
    params(val: table, metatable: table | nil),
    returns(table)
)]
fn impl_set_metatable(
    val: RuntimeValue,
    metatable: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(tab) = val else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    match metatable {
        RuntimeValue::Nil => {
            tab.borrow_mut().metatable = None;
        }
        RuntimeValue::Table(mt) => {
            tab.borrow_mut().metatable = Some(mt);
        }
        _ => return Err(DukaRuntimeError::InvalidValueType(ctype::TAB)),
    }
    Ok(RuntimeValue::Table(tab))
}

#[duka_builtin(name = "instanceof", doc = "Check if the value is an instance of target", params(value: any, target: any))]
fn impl_instanceof(
    h: &mut Heap,
    value: RuntimeValue,
    target: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(cls) = target else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    let mut result = false;
    if let RuntimeValue::Table(x) = value {
        let index_key = RuntimeValue::meta_method_key(h, &MetaMethod::Index);
        let mut cur: Option<_> = Some(x);
        let mut seen: Vec<_> = vec![];
        while let Some(t) = cur {
            if seen.contains(&t) {
                break;
            }
            seen.push(t);
            if std::ptr::eq(&*t, &*cls) {
                result = true;
                break;
            }
            let inner = t.borrow();
            cur = inner.metatable.as_ref().cloned().or_else(|| {
                inner.get(&index_key).and_then(|v| match v {
                    RuntimeValue::Table(n) => Some(*n),
                    _ => None,
                })
            });
        }
    }
    Ok(RuntimeValue::Bool(result))
}

#[duka_builtin(name = "pairs", doc = "Return key-value iterator for table", params(tab: table), flags(@returns(iterator)))]
fn impl_pairs(h: &mut Heap, tab: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    let entries: Vec<(RuntimeValue, RuntimeValue)> = t
        .borrow()
        .inner
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let func = make_pairs_iterator(h, entries);
    Ok(func)
}

#[duka_builtin(name = "ipairs", doc = "Return index-value iterator for table", params(tab: table), flags(@returns(iterator)))]
fn impl_ipairs(h: &mut Heap, tab: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    let mut items: Vec<RuntimeValue> = vec![];
    {
        let tab = t.borrow();
        let mut i: DukaInt = 0;
        while let Some(v) = tab.array_get(i as usize) {
            items.push(v.clone());
            i += 1;
        }
    }
    let captures = items.clone();
    let mut iter = items.into_iter().enumerate();
    let func = RustClosure::returns_with_captures(
        move |c, _h, _n| match iter.next() {
            Some((i, v)) => {
                c.set_stack(0, RuntimeValue::Bool(true))?;
                c.set_stack(1, RuntimeValue::Int(i as DukaInt))?;
                c.set_stack(2, v)?;
                Ok(ValueCount::Exact(3))
            }
            None => {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                Ok(ValueCount::Exact(1))
            }
        },
        captures,
        Some("__ipairs_iter".into()),
    );
    let func = h.alloc(GcCell::new(func));
    Ok(RuntimeValue::NativeFunc(func))
}
