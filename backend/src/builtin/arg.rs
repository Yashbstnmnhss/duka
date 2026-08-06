use duka_shared::constants::ctype;
use duka_shared::value::{DukaFloat, DukaInt};

use crate::errors::DukaRuntimeError;
use crate::value::RuntimeValue;
use crate::vm::coroutine::CoState;

fn get(sv: &CoState, idx: usize, func: &str, want: &str) -> Result<RuntimeValue, DukaRuntimeError> {
    sv.get_stack(idx + 1)
        .cloned()
        .map_err(|_| DukaRuntimeError::ArgumentMissing(idx, func.into(), want.into()))
}

fn bad(idx: usize, func: &str, v: &RuntimeValue, want: &'static str) -> DukaRuntimeError {
    DukaRuntimeError::ArgumentInvalidType(idx, func.into(), want, v.type_of())
}

pub fn take_string(sv: &CoState, idx: usize, func: &str) -> Result<String, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::STR)?;
    if !v.is_string() {
        return Err(bad(idx, func, &v, ctype::STR));
    }
    Ok(v.eval_to_string().into_owned())
}

pub fn take_bytes(sv: &CoState, idx: usize, func: &str) -> Result<Vec<u8>, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::STR)?;
    if !v.is_string() {
        return Err(bad(idx, func, &v, ctype::STR));
    }
    Ok(v.eval_to_string().as_bytes().to_vec())
}

pub fn take_int(sv: &CoState, idx: usize, func: &str) -> Result<DukaInt, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::INT)?;
    if !matches!(v, RuntimeValue::Int(..)) {
        return Err(bad(idx, func, &v, ctype::INT));
    }
    v.eval_to_int()
        .ok_or_else(|| bad(idx, func, &v, ctype::INT))
}

pub fn take_num(sv: &CoState, idx: usize, func: &str) -> Result<DukaFloat, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::NUM)?;
    if !v.is_number() {
        return Err(bad(idx, func, &v, ctype::NUM));
    }
    v.eval_to_float()
        .ok_or_else(|| bad(idx, func, &v, ctype::NUM))
}

/// Like `take_num` but returns the original value unchanged, so the caller keeps
/// the runtime type (Int stays Int, Float stays Float).
pub fn take_number(sv: &CoState, idx: usize, func: &str) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::NUM)?;
    if !v.is_number() {
        return Err(bad(idx, func, &v, ctype::NUM));
    }
    Ok(v)
}

pub fn take_bool(sv: &CoState, idx: usize, func: &str) -> Result<bool, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::BOO)?;
    if !v.is_bool() {
        return Err(bad(idx, func, &v, ctype::BOO));
    }
    Ok(v.eval_to_bool())
}

pub fn take_table(sv: &CoState, idx: usize, func: &str) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::TAB)?;
    if !v.is_table() {
        return Err(bad(idx, func, &v, ctype::TAB));
    }
    Ok(v)
}

pub fn take_function(
    sv: &CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, "function")?;
    if !v.is_function() {
        return Err(bad(idx, func, &v, ctype::FUN));
    }
    Ok(v)
}

pub fn take_any(sv: &CoState, idx: usize, func: &str) -> Result<RuntimeValue, DukaRuntimeError> {
    get(sv, idx, func, "any")
}
