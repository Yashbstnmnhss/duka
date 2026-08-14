use crate::builtin::BuiltinFn;
use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};

use crate::{
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue},
};

duka_builtin_def! {
    mod table
    fn {
        meta:
            impl_raw_get,
            impl_raw_set,
            impl_keys,
            impl_values,
            impl_has
    }
    const {

    }
}

#[duka_builtin(
    
    doc = "Get property in table by given key without calling metamethod",
    params(tab: table, key: any),
    returns(any)
)]
fn impl_raw_get(tab: RuntimeValue, key: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let r = match tab {
        RuntimeValue::Table(t) => t.borrow().get(&key).cloned().unwrap_or(RuntimeValue::Nil),
        _ => RuntimeValue::Nil,
    };
    Ok(r)
}

#[duka_builtin(
    
    doc = "Set property in table by given key and value without calling metamethod",
    params(tab: table, key: any, val: any)
)]
fn impl_raw_set(
    tab: RuntimeValue,
    key: RuntimeValue,
    val: RuntimeValue,
) -> Result<(), DukaRuntimeError> {
    if let RuntimeValue::Table(t) = tab {
        t.borrow_mut().set(key, val);
    }
    Ok(())
}
#[duka_builtin(
    
    doc = "Get an array with keys in table",
    params(tab: table),
    returns(array)
)]
fn impl_keys(h: &mut Heap, tab: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        unreachable!()
    };
    let t = t.borrow();
    let res = RuntimeDukaArray {
        items: t.inner.keys().cloned().collect(),
    };
    Ok(RuntimeValue::Array(h.alloc(GcCell::new(res))))
}

#[duka_builtin(
    
    doc = "Get an array with values in table",
    params(tab: table),
    returns(array)
)]
fn impl_values(h: &mut Heap, tab: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        unreachable!()
    };
    let t = t.borrow();
    let res = RuntimeDukaArray {
        items: t.inner.values().cloned().collect(),
    };
    Ok(RuntimeValue::Array(h.alloc(GcCell::new(res))))
}

#[duka_builtin(
    
    name = "has",
    doc = "Whether given key is in target table",
    params(tab: table, key: any),
    returns(bool)
)]
fn impl_has(tab: RuntimeValue, key: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        unreachable!()
    };
    Ok(RuntimeValue::Bool(t.borrow().inner.contains_key(&key)))
}
#[duka_builtin(
    
    doc = "Set property in table by given key and value without calling metamethod",
    params(tab: table, key: any, val: any)
)]
fn impl_insert(
    tab: RuntimeValue,
    key: RuntimeValue,
    val: RuntimeValue,
) -> Result<(), DukaRuntimeError> {
    if let RuntimeValue::Table(t) = tab {
        t.borrow_mut().set(key, val);
    }
    Ok(())
}
#[duka_builtin(
    
    doc = "Remove property in table by given key",
    params(tab: table, key: any),
    returns(any)
)]
fn impl_remove(tab: RuntimeValue, key: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    if let RuntimeValue::Table(t) = tab {
        Ok(t.borrow_mut().inner.remove(&key).unwrap_or_default())
    } else {
        unreachable!()
    }
}
