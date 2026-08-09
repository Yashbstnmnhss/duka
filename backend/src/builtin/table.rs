use duka_gc::Heap;
use duka_shared::{builtin::Builtins, types::ValueCount};

use crate::{
    builtin::{BuiltinFn, required},
    errors::DukaRuntimeError,
    value::RuntimeValue,
    vm::coroutine::CoState,
};

pub fn registry() -> Builtins<BuiltinFn> {
    Builtins::new()
        .register("raw_get", BuiltinFn::Plain(impl_raw_get))
        .register("raw_set", BuiltinFn::Plain(impl_raw_set))
}

pub fn builtin_metas() -> Vec<duka_shared::builtin_meta::MetaInfo> {
    registry().into_metas()
}

fn impl_raw_get(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let tab = required(sv, 0, "raw_get", "table")?.clone();
    let key = required(sv, 1, "raw_get", "key")?;
    let r = match tab {
        RuntimeValue::Table(t) => t.borrow().get(key).cloned().unwrap_or(RuntimeValue::Nil),
        _ => RuntimeValue::Nil,
    };
    sv.set_stack(0, r)?;
    Ok(ValueCount::Exact(1))
}

fn impl_raw_set(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let tab = required(sv, 0, "raw_set", "table")?.clone();
    let key = required(sv, 1, "raw_set", "key")?.clone();
    let val = required(sv, 2, "raw_set", "value")?.clone();
    if let RuntimeValue::Table(t) = tab {
        t.borrow_mut().set(key, val);
    }
    Ok(ValueCount::Exact(0))
}
