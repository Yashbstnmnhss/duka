//! See docs/stdlib.md #Iterator Protocol

use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::{constants::ctype, types::ValueCount, value::DukaInt};

use crate::{
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue, RustClosure},
    vm::coroutine::{CoState, NativeApi},
};

duka_builtin_def! {
    mod iter
    flags(@returns(iterator))
    fn {
        meta:
            impl_range,
            impl_repeat,
            impl_map,
            impl_filter,
            impl_take,
            impl_skip,
            impl_to_array co,
            impl_all co,
            impl_any co,
            impl_chain,
            impl_count co,
            impl_enumerate,
            impl_for_each co,
            impl_partition co
    }
    const {}
}

// 迭代器库中的返回大多是闭包, 闭包必须声明captures,才能让GC捕获

enum Source {
    Array(RuntimeValue, usize),
    String(RuntimeValue, usize),
    Func(RuntimeValue),
}

// 尝试消耗source
fn source_pull(
    c: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    src: &mut Source,
) -> Result<Option<Vec<RuntimeValue>>, DukaRuntimeError> {
    match src {
        Source::String(str, idx) => {
            let str = str.eval_to_string();
            if let Some(ch) = str[*idx..].chars().next() {
                let end = *idx + ch.len_utf8();
                let slice = &str[*idx..end];
                *idx = end;
                return Ok(Some(vec![RuntimeValue::from_str(h, slice)]));
            }
            Ok(None)
        }
        Source::Array(arr, idx) => {
            let v = match arr {
                RuntimeValue::Array(a) => a.borrow().items.get(*idx).cloned(),
                _ => unreachable!(),
            };
            *idx += 1;
            Ok(v.map(|i| vec![i]))
        }
        Source::Func(f) => {
            let mut values = c.protected_call(h, api, f.clone(), &[])??;
            if values.first() == Some(&RuntimeValue::Bool(true)) {
                values.remove(0);
                Ok(Some(values))
            } else {
                Ok(None)
            }
        }
    }
}

#[duka_builtin(
    doc = "Create an iterator repeats who for times (or infinity)",
    params(who: any, times: int = -1),
    returns(any)
)]
fn impl_repeat(
    h: &mut Heap,
    who: RuntimeValue,
    times: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut cur = 0i64;
    let func = RustClosure::returns_with_captures(
        move |c, _h, _n| {
            if cur >= times {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                return Ok(ValueCount::Exact(1));
            }
            cur += 1;
            c.set_stack(0, RuntimeValue::Bool(true))?;
            c.set_stack(1, who.clone())?;
            Ok(ValueCount::Exact(2))
        },
        vec![],
        Some("__iter.repeat".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

// 生成器函数
#[duka_builtin(
    doc = "Create an iterator over a range [from, to)",
    params(from: int, to: int, step: int = 1),
    returns(any)
)]
fn impl_range(
    h: &mut Heap,
    from: DukaInt,
    to: DukaInt,
    step: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut cur = from;
    let func = RustClosure::returns_with_captures(
        move |c, _h, _n| {
            if (step > 0 && cur >= to) || (step < 0 && cur <= to) {
                c.set_stack(0, RuntimeValue::Bool(false))?; // 生成range左闭右开
                return Ok(ValueCount::Exact(1));
            }
            let v = RuntimeValue::Int(cur);
            cur += step;
            c.set_stack(0, RuntimeValue::Bool(true))?;
            c.set_stack(1, v)?;
            Ok(ValueCount::Exact(2))
        },
        vec![],
        Some("__iter.range".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

// 惰性的组合子函数

// TODO: zip enumerate chain unzip

#[duka_builtin(
    doc = "",
    params(coll: any, other: any),
    returns(any)
)]
fn impl_chain(_coll: RuntimeValue, _other: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    todo!()
}

#[duka_builtin(
    doc = "Creates an iterator which gives the current iteration count as well as the next value",
    params(coll: any),
    returns(any)
)]
fn impl_enumerate(h: &mut Heap, coll: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let captures = vec![coll];
    let mut idx = 0usize;
    let func = RustClosure::returns_with_captures(
        move |c, h, api| {
            let Some(v) = source_pull(c, h, api, &mut src)? else {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                return Ok(ValueCount::Exact(1));
            };
            c.set_stack(0, RuntimeValue::Bool(true))?;
            c.set_stack(1, RuntimeValue::Int(idx as DukaInt))?;
            for val in v {
                c.append_stack(val)?;
            }
            idx += 1;
            Ok(ValueCount::Exact(2))
        },
        captures,
        Some("__iter.enumerate".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

#[duka_builtin(
    doc = "Map each element of an iterable through a function, lazily",
    params(coll: any, f: fn),
    returns(any)
)]
fn impl_map(
    h: &mut Heap,
    coll: RuntimeValue,
    f: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let cb = f.clone();
    let captures = vec![coll, f];
    let func = RustClosure::returns_with_captures(
        move |c, h, api| {
            let Some(v) = source_pull(c, h, api, &mut src)? else {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                return Ok(ValueCount::Exact(1));
            };
            let r = c
                .call_user_protected(h, api, cb.clone(), &v)?
                .into_iter()
                .next()
                .unwrap_or_default();
            c.set_stack(0, RuntimeValue::Bool(true))?;
            c.set_stack(1, r)?;
            Ok(ValueCount::Exact(2))
        },
        captures,
        Some("__iter.map".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

#[duka_builtin(
    name = "filter",
    doc = "Keep elements for which pred returns truthy, lazily",
    params(coll: any, pred: fn),
    returns(any)
)]
fn impl_filter(
    h: &mut Heap,
    coll: RuntimeValue,
    pred: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let cb = pred.clone();
    let captures = vec![coll, pred];
    let func = RustClosure::returns_with_captures(
        move |c, h, api| {
            loop {
                let Some(v) = source_pull(c, h, api, &mut src)? else {
                    c.set_stack(0, RuntimeValue::Bool(false))?;
                    return Ok(ValueCount::Exact(1));
                };
                let keep = c
                    .call_user_protected(h, api, cb.clone(), &v)?
                    .into_iter()
                    .next()
                    .map(|v| v.eval_to_bool())
                    .unwrap_or_default();
                if keep {
                    c.set_stack(0, RuntimeValue::Bool(true))?;
                    for val in v {
                        c.append_stack(val)?;
                    }
                    return Ok(ValueCount::Exact(2));
                }
            }
        },
        captures,
        Some("__iter.filter".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

#[duka_builtin(
    name = "skip",
    doc = "Skip at most n elements from an iterable, lazily",
    params(coll: any, n: int),
    returns(any)
)]
fn impl_skip(
    h: &mut Heap,
    coll: RuntimeValue,
    n: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let captures = vec![coll];
    let mut cur = 0i64;
    let func = RustClosure::returns_with_captures(
        move |c, h, api| {
            loop {
                if cur < n {
                    cur += 1;
                    continue;
                }
                let Some(v) = source_pull(c, h, api, &mut src)? else {
                    c.set_stack(0, RuntimeValue::Bool(false))?;
                    return Ok(ValueCount::Exact(1));
                };
                c.set_stack(0, RuntimeValue::Bool(true))?;
                for val in v {
                    c.append_stack(val)?;
                }
                return Ok(ValueCount::Exact(2));
            }
        },
        captures,
        Some("__iter.skip".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

#[duka_builtin(
    name = "take",
    doc = "Take at most n elements from an iterable, lazily",
    params(coll: any, n: int),
    returns(any)
)]
fn impl_take(
    h: &mut Heap,
    coll: RuntimeValue,
    n: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let mut left = n;
    let captures = vec![coll];
    let func = RustClosure::returns_with_captures(
        move |c, h, api| {
            if left <= 0 {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                return Ok(ValueCount::Exact(1));
            }
            let Some(v) = source_pull(c, h, api, &mut src)? else {
                c.set_stack(0, RuntimeValue::Bool(false))?;
                return Ok(ValueCount::Exact(1));
            };
            left -= 1;
            c.set_stack(0, RuntimeValue::Bool(true))?;
            for val in v {
                c.append_stack(val)?;
            }
            Ok(ValueCount::Exact(2))
        },
        captures,
        Some("__iter.take".into()),
    );
    Ok(RuntimeValue::NativeFunc(h.alloc(GcCell::new(func))))
}

// 收集器
#[duka_builtin(
    name = "to_array",
    doc = "Collect all elements of an iterable into an array",
    params(coll: any),
    returns(array)
)]
fn impl_to_array(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let mut items: Vec<RuntimeValue> = vec![];
    while let Some(v) = source_pull(sv, h, api, &mut src)? {
        if v.len() == 1 {
            items.push(v[0]);
        } else {
            items.push(RuntimeValue::from_vec(h, v));
        }
    }
    let res = RuntimeDukaArray { items };
    Ok(RuntimeValue::Array(h.alloc(GcCell::new(res))))
}
#[duka_builtin(
    name = "for_each",
    doc = "`foreach item in ...`",
    params(coll: any, f: fn)
)]
fn impl_for_each(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
    f: RuntimeValue,
) -> Result<(), DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    while let Some(v) = source_pull(sv, h, api, &mut src)? {
        sv.call_user_protected(h, api, f.clone(), &v)?;
    }
    Ok(())
}
#[duka_builtin(
    name = "count",
    doc = "Return the count of all elements of an iterable into an array",
    params(coll: any),
    returns(int)
)]
fn impl_count(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let mut num = 0;
    while let Some(_) = source_pull(sv, h, api, &mut src)? {
        num += 1;
    }
    Ok(RuntimeValue::Int(num))
}
#[duka_builtin(
    name = "any",
    doc = "Collect all elements of an iterable, check whether any of them fits predication",
    params(coll: any, pred: fn),
    returns(bool)
)]
fn impl_any(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
    pred: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    while let Some(v) = source_pull(sv, h, api, &mut src)? {
        if sv
            .call_user_protected(h, api, pred.clone(), &v)?
            .into_iter()
            .next()
            .map(|v| v.eval_to_bool())
            .unwrap_or_default()
        {
            return Ok(RuntimeValue::Bool(true));
        }
    }
    Ok(RuntimeValue::Bool(false))
}
#[duka_builtin(
    name = "all",
    doc = "Collect all elements of an iterable, check whether all of them fit predication",
    params(coll: any, pred: fn),
    returns(bool)
)]
fn impl_all(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
    pred: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    while let Some(v) = source_pull(sv, h, api, &mut src)? {
        if !sv
            .call_user_protected(h, api, pred.clone(), &v)?
            .into_iter()
            .next()
            .map(|v| v.eval_to_bool())
            .unwrap_or_default()
        {
            return Ok(RuntimeValue::Bool(false));
        }
    }
    Ok(RuntimeValue::Bool(true))
}
#[duka_builtin(
    name = "all",
    doc = "Make partition of source by predication",
    params(coll: any, pred: fn),
    returns(array, array),
    return_doc "First is `true`, second is `false`, both array"
)]
fn impl_partition(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    coll: RuntimeValue,
    pred: RuntimeValue,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let mut src = source_of(&coll)?;
    let mut trues: Vec<RuntimeValue> = vec![];
    let mut falses: Vec<RuntimeValue> = vec![];
    while let Some(v) = source_pull(sv, h, api, &mut src)? {
        let vs = if sv
            .call_user_protected(h, api, pred.clone(), &v)?
            .into_iter()
            .next()
            .map(|v| v.eval_to_bool())
            .unwrap_or_default()
        {
            &mut trues
        } else {
            &mut falses
        };

        if v.len() == 1 {
            vs.push(v[0]);
        } else {
            vs.push(RuntimeValue::from_vec(h, v));
        }
    }
    Ok(vec![
        RuntimeValue::from_vec(h, trues),
        RuntimeValue::from_vec(h, falses),
    ])
}

// 来源可能是array也可能是iterator function
fn source_of(coll: &RuntimeValue) -> Result<Source, DukaRuntimeError> {
    match coll {
        RuntimeValue::Array(_) => Ok(Source::Array(coll.clone(), 0)),
        rv if rv.is_string() => Ok(Source::String(coll.clone(), 0)),
        RuntimeValue::NativeFunc(_) | RuntimeValue::UserFunc(_) => Ok(Source::Func(coll.clone())),
        _ => Err(DukaRuntimeError::InvalidValueType(ctype::ARR)),
    }
}
