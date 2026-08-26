use std::cmp::Ordering;

use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::{
    constants::{MetaMethod, ctype},
    value::DukaInt,
};

use crate::{
    builtin::{call_meta_method, normalize},
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue, try_cmp_values},
    vm::coroutine::{CoState, NativeApi},
};

duka_builtin_def! {
    mod array
    fn {
        meta:
            impl_pack,
            impl_unpack,
            impl_has,
            impl_push,
            impl_pop,
            impl_insert,
            impl_remove,
            impl_len,
            impl_concat,
            impl_sort co,
            impl_index_of co,
    }
    const {

    }
}

#[duka_builtin(
    name = "pack",
    doc = "Pack all arguments into an array",
    params(vals: vararg),
    returns(array)
)]
fn impl_pack(h: &mut Heap, vals: Vec<RuntimeValue>) -> Result<RuntimeValue, DukaRuntimeError> {
    let res = RuntimeDukaArray { items: vals };
    Ok(RuntimeValue::Array(h.alloc(GcCell::new(res))))
}

#[duka_builtin(
    name = "unpack",
    doc = "Unpack array into a tuple(as results)",
    params(arr: array),
    returns(vararg)
)]
fn impl_unpack(arr: RuntimeValue) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    Ok(a.borrow().items.clone())
}

#[duka_builtin(
    name = "push",
    doc = "Push a value into array",
    params(arr: array, val: any)
)]
fn impl_push(arr: RuntimeValue, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    a.borrow_mut().items.push(val);
    Ok(())
}

#[duka_builtin(
    name = "pop",
    doc = "Pop a value from array",
    params(arr: array),
    returns(any)
)]
fn impl_pop(arr: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    Ok(a.borrow_mut().items.pop().unwrap_or_default())
}

#[duka_builtin(
    name = "has",
    doc = "Whether given value is in target array",
    params(arr: array, who: any),
    returns(bool)
)]
fn impl_has(arr: RuntimeValue, who: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    Ok(RuntimeValue::Bool(a.borrow().items.contains(&who)))
}

#[duka_builtin(
    name = "len",
    doc = "Get length of array",
    params(arr: array),
    returns(int)
)]
fn impl_len(arr: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    Ok(RuntimeValue::Int(a.borrow().len() as DukaInt))
}

#[duka_builtin(
    name = "insert",
    doc = "Insert value at given index in array",
    params(arr: array, index: int, value: any),
    returns(array)
)]
fn impl_insert(
    arr: RuntimeValue,
    index: DukaInt,
    value: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    let mut a = a.borrow_mut();
    let index = normalize(index, a.len());
    if a.len() == index {
        a.push(value)
    } else if a.len() > index {
        a.set(index as usize, value);
    } else {
        return Err(DukaRuntimeError::OutOfRange(ctype::ARR));
    }
    Ok(arr)
}

#[duka_builtin(
    name = "remove",
    doc = "Remove target value at given index in array",
    params(arr: array, index: int),
    returns(array)
)]
fn impl_remove(arr: RuntimeValue, index: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    let mut a = a.borrow_mut();
    let index = normalize(index, a.len());
    if a.len() > index {
        a.items.remove(index as usize);
    } else {
        return Err(DukaRuntimeError::OutOfRange(ctype::ARR));
    }
    Ok(arr)
}

#[duka_builtin(
    name = "sort",
    doc = "Sort array",
    params(
        arr: array,
        cmp: fn | nil = RuntimeValue::Nil,
        @default = "nil",
        @doc = "This function should return an integer, for `< 0` represents less, `> 0` represents greater and `= 0` represents equal (and other situations)"
    ),
    returns(array)
)]
fn impl_sort(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    arr: RuntimeValue,
    cmp: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    let mut a = a.borrow_mut();
    if cmp.is_function() {
        a.items.sort_by(|a, b| {
            let result = sv
                .call_user_protected(h, api, cmp.clone(), &[a.clone(), b.clone()])
                .map(|v| v.into_iter().next().unwrap_or_default().eval_to_int())
                .unwrap_or(None);
            match result {
                None => Ordering::Equal,
                Some(0) => Ordering::Equal,
                Some(..0) => Ordering::Less,
                Some(1..) => Ordering::Greater,
            }
        });
    } else {
        a.items.sort_by(|a, b| match (a, b) {
            (x, y) if x.is_metamethod() => {
                if call_meta_method(sv, h, api, x, MetaMethod::LT, std::slice::from_ref(y), true)
                    .ok()
                    .flatten()
                    .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
                    .flatten()
                    .unwrap_or_default()
                {
                    Ordering::Less
                } else if call_meta_method(
                    sv,
                    h,
                    api,
                    x,
                    MetaMethod::Eq,
                    std::slice::from_ref(y),
                    true,
                )
                .ok()
                .flatten()
                .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
                .flatten()
                .unwrap_or_default()
                {
                    Ordering::Equal
                } else {
                    Ordering::Greater
                }
            }
            (x, y) => try_cmp_values(x, y).unwrap_or(Ordering::Greater),
        });
    }
    Ok(arr)
}

#[duka_builtin(
    name = "concat",
    doc = "Concat two arrays",
    params(arr: array, other: array),
    returns(array)
)]
fn impl_concat(
    h: &mut Heap,
    arr: RuntimeValue,
    other: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    let RuntimeValue::Array(b) = other else {
        unreachable!()
    };
    let items = [a.borrow().items.clone(), b.borrow().items.clone()].concat();
    Ok(RuntimeValue::Array(
        h.alloc(GcCell::new(RuntimeDukaArray { items })),
    ))
}

#[duka_builtin(
    name = "index_of",
    doc = "Find an item in array and return its index",
    params(arr: array, who: any | fn),
    returns(int)
)]
fn impl_index_of(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    arr: RuntimeValue,
    who: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Array(a) = arr else {
        unreachable!()
    };
    let array = a.borrow();
    if who.is_function() {
        Ok(array
            .items
            .iter()
            .position(|i| {
                sv.call_user_protected(h, api, who.clone(), std::slice::from_ref(i))
                    .ok()
                    .map(|v| v.into_iter().next())
                    .flatten()
                    .map(|i| i.eval_to_bool())
                    .unwrap_or_default()
            })
            .map(|v| v as DukaInt)
            .unwrap_or(-1)
            .into())
    } else {
        Ok(array
            .items
            .iter()
            .position(|i| i == &who)
            .map(|v| v as DukaInt)
            .unwrap_or(-1)
            .into())
    }
}
