use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::{constants::ctype, value::DukaInt};

use crate::{
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue},
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
            impl_concat
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
    if a.len() as DukaInt == index {
        a.push(value)
    } else if a.len() as DukaInt > index {
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
    if a.len() as DukaInt > index {
        a.items.remove(index as usize);
    } else {
        return Err(DukaRuntimeError::OutOfRange(ctype::ARR));
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
