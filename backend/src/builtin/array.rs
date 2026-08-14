use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};

use crate::{
    builtin::BuiltinFn,
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue},
};

duka_builtin_def! {
    mod array
    fn {
        meta: impl_pack, impl_unpack, impl_has, impl_push, impl_pop
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
