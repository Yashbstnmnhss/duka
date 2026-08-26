use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::value::DukaInt;

use crate::{
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeDukaTable, RuntimeValue},
};

duka_builtin_def! {
    mod table
    fn {
        meta:
            impl_raw_get,
            impl_raw_set,
            impl_keys,
            impl_values,
            impl_has,
            impl_has_value,
            impl_raw_get_set,
            impl_merge,
            impl_remove,
            impl_clear,
            impl_capacity
    }
    const {

    }
}

#[duka_builtin(
    doc = "Create new table with given capacity",
    params(cap: int),
    returns(table)
)]
fn impl_capacity(h: &mut Heap, cap: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Table(
        h.alloc(GcCell::new(RuntimeDukaTable::new(cap as usize))),
    ))
}

#[duka_builtin(
    doc = "Clear table",
    params(tab: table),
    returns(table)
)]
fn impl_clear(tab: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        unreachable!()
    };
    t.borrow_mut().inner.clear();
    Ok(RuntimeValue::Table(t))
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
    name = "has_value",
    doc = "Whether given value is in target table",
    params(tab: table, val: any),
    returns(bool)
)]
fn impl_has_value(tab: RuntimeValue, val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::Table(t) = tab else {
        unreachable!()
    };
    Ok(RuntimeValue::Bool(
        t.borrow().inner.iter().any(|v| v.1 == &val),
    ))
}

#[duka_builtin(
    doc = "Merge another table to this table",
    params(tab: table, other: table, keep: bool = false)
)]
fn impl_merge(tab: RuntimeValue, other: RuntimeValue, keep: bool) -> Result<(), DukaRuntimeError> {
    if let RuntimeValue::Table(t) = tab
        && let RuntimeValue::Table(t2) = other
    {
        let mut t = t.borrow_mut();
        for (k, v) in &t2.borrow().inner {
            if t.get(k).is_some() && keep {
                continue;
            }
            t.set(k.clone(), v.clone());
        }
    }
    Ok(())
}

#[duka_builtin(
    doc = "Get property in tab by given key without calling metamethod. If not exist, insert with val and return it",
    params(tab: table, key: any, val: any = RuntimeValue::Nil, @default = "nil"),
    returns(any)
)]
fn impl_raw_get_set(
    tab: RuntimeValue,
    key: RuntimeValue,
    val: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    if let RuntimeValue::Table(t) = tab {
        if let Some(v) = t.borrow().get(&key).cloned() {
            return Ok(v);
        }
        t.borrow_mut().set(key, val.clone());
        Ok(val)
    } else {
        unreachable!()
    }
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
