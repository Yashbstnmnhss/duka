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
            impl_map co,
            impl_filter co,
            impl_take co,
            impl_to_array co
    }
    const {}
}

// 迭代器库中的返回大多是闭包, 闭包必须声明captures,才能让GC捕获

enum Source {
    Array(RuntimeValue, usize),
    Func(RuntimeValue),
}

// 尝试消耗source
fn source_pull(
    c: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    src: &mut Source,
) -> Result<Option<RuntimeValue>, DukaRuntimeError> {
    match src {
        Source::Array(arr, idx) => {
            let v = match arr {
                RuntimeValue::Array(a) => a.borrow().items.get(*idx).cloned(),
                _ => unreachable!(),
            };
            *idx += 1;
            Ok(v)
        }
        Source::Func(f) => {
            let values = c
                .protected_call(h, api, f.clone(), &[])?
                .map_err(|e| e)?;
            if values.first() == Some(&RuntimeValue::Bool(true)) {
                Ok(values.get(1).cloned())
            } else {
                Ok(None)
            }
        }
    }
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
#[duka_builtin(
    doc = "Map each element of an iterable through a function, lazily",
    params(coll: any, f: fn),
    returns(any)
)]
fn impl_map(
    _sv: &mut CoState,
    h: &mut Heap,
    _api: &mut NativeApi,
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
            let r = c.call_user_sync(h, api, cb.clone(), &[v])?;
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
    _sv: &mut CoState,
    h: &mut Heap,
    _api: &mut NativeApi,
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
                let keep = c.call_user_sync(h, api, cb.clone(), &[v.clone()])?;
                if keep.eval_to_bool() {
                    c.set_stack(0, RuntimeValue::Bool(true))?;
                    c.set_stack(1, v)?;
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
    name = "take",
    doc = "Take at most n elements from an iterable, lazily",
    params(coll: any, n: int),
    returns(any)
)]
fn impl_take(
    h: &mut Heap,
    _api: &mut NativeApi,
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
            c.set_stack(1, v)?;
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
    loop {
        let Some(v) = source_pull(sv, h, api, &mut src)? else {
            break;
        };
        items.push(v);
    }
    let res = RuntimeDukaArray { items };
    Ok(RuntimeValue::Array(h.alloc(GcCell::new(res))))
}

// 来源可能是array也可能是iterator function
fn source_of(coll: &RuntimeValue) -> Result<Source, DukaRuntimeError> {
    match coll {
        RuntimeValue::Array(_) => Ok(Source::Array(coll.clone(), 0)),
        RuntimeValue::NativeFunc(_) | RuntimeValue::UserFunc(_) => Ok(Source::Func(coll.clone())),
        _ => Err(DukaRuntimeError::InvalidValueType(ctype::ARR)),
    }
}
