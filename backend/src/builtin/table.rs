use duka_macros::duka_builtin;

use crate::{errors::DukaRuntimeError, value::RuntimeValue};

define_builtins! {
    fn:
        meta:
            "raw_get" => impl_raw_get, __DUKA_IMPL_RAW_GET_META,
            "raw_set" => impl_raw_set, __DUKA_IMPL_RAW_SET_META;
    const:
}

#[duka_builtin(
    module = "table",
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
    module = "table",
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
