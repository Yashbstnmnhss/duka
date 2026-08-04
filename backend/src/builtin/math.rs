use std::f64::consts::{E, PI};

use duka_gc::Heap;
use duka_shared::{
    builtin::Builtins,
    constants::ctype,
    types::ValueCount,
    value::{DukaFloat, DukaInt},
};

use crate::{
    builtin::{BuiltinFn, ensure_type, optional, require, required},
    errors::DukaRuntimeError,
    value::RuntimeValue,
    vm::coroutine::CoState,
};

pub fn registry() -> Builtins<BuiltinFn> {
    Builtins::new()
        .register("max", impl_max as BuiltinFn)
        .register("min", impl_min as BuiltinFn)
        .register("abs", impl_abs as BuiltinFn)
}
pub fn consts_registry() -> Builtins<RuntimeValue> {
    Builtins::new()
        .register("PI", RuntimeValue::Float(PI))
        .register("E", RuntimeValue::Float(E))
}

fn impl_max(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    Ok(ValueCount::Exact(1))
}

fn impl_min(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    Ok(ValueCount::Exact(1))
}

fn impl_abs(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "abs", "value")?.clone();
    if let RuntimeValue::Int(i) = val {
        sv.set_stack(0, RuntimeValue::Int(i.abs()))?;
    } else if let RuntimeValue::Float(f) = val {
        sv.set_stack(0, RuntimeValue::Float(f.abs()))?;
    } else {
        sv.set_stack(0, val)?;
    }
    Ok(ValueCount::Exact(1))
}
fn impl_sin(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "sin", "value")?.clone();
    sv.set_stack(
        0,
        val.eval_to_float()
            .ok_or_else(|| {
                DukaRuntimeError::ArgumentInvalidType(
                    0,
                    "sin".to_string(),
                    ctype::NUM,
                    val.type_of(),
                )
            })
            .map(|v| RuntimeValue::Float(v.sin()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
