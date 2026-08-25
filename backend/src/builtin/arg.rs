use std::error::Error;

use duka_gc::Heap;
use duka_shared::constants::ctype;
use duka_shared::types::ValueCount;
use duka_shared::value::{DukaFloat, DukaInt};

use crate::errors::DukaRuntimeError;
use crate::value::RuntimeValue;
use crate::vm::coroutine::CoState;

fn get(
    sv: &mut CoState,
    idx: usize,
    func: &str,
    want: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    sv.take_stack(idx + 1)
        .map_err(|_| DukaRuntimeError::ArgumentMissing(idx, func.into(), want.into()))
}

fn bad(idx: usize, func: &str, v: &RuntimeValue, want: &'static str) -> DukaRuntimeError {
    DukaRuntimeError::ArgumentInvalidType(idx, func.into(), want, v.type_name_of())
}

pub fn take_string(sv: &mut CoState, idx: usize, func: &str) -> Result<String, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::STR)?;
    if !v.is_string() {
        return Err(bad(idx, func, &v, ctype::STR));
    }
    Ok(v.eval_to_string().into_owned())
}

pub fn take_bytes(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::STR)?;
    if !v.is_string() {
        return Err(bad(idx, func, &v, ctype::STR));
    }
    Ok(v)
}

pub fn take_int(sv: &mut CoState, idx: usize, func: &str) -> Result<DukaInt, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::INT)?;
    if !matches!(v, RuntimeValue::Int(..)) {
        return Err(bad(idx, func, &v, ctype::INT));
    }
    v.eval_to_int()
        .ok_or_else(|| bad(idx, func, &v, ctype::INT))
}

pub fn take_num(sv: &mut CoState, idx: usize, func: &str) -> Result<DukaFloat, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::NUM)?;
    if !v.is_number() {
        return Err(bad(idx, func, &v, ctype::NUM));
    }
    v.eval_to_float()
        .ok_or_else(|| bad(idx, func, &v, ctype::NUM))
}

/// Like `take_num` but returns the original value unchanged, so the caller keeps
/// the runtime type (Int stays Int, Float stays Float).
pub fn take_number(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::NUM)?;
    if !v.is_number() {
        return Err(bad(idx, func, &v, ctype::NUM));
    }
    Ok(v)
}

pub fn take_bool(sv: &mut CoState, idx: usize, func: &str) -> Result<bool, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::BOO)?;
    if !v.is_bool() {
        return Err(bad(idx, func, &v, ctype::BOO));
    }
    Ok(v.eval_to_bool())
}

pub fn take_array(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::ARR)?;
    if !v.is_array() {
        return Err(bad(idx, func, &v, ctype::ARR));
    }
    Ok(v)
}

pub fn take_table(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, ctype::TAB)?;
    if !v.is_table() {
        return Err(bad(idx, func, &v, ctype::TAB));
    }
    Ok(v)
}

pub fn take_function(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, "function")?;
    if !v.is_function() {
        return Err(bad(idx, func, &v, ctype::FUN));
    }
    Ok(v)
}

pub fn take_any(
    sv: &mut CoState,
    idx: usize,
    func: &str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    get(sv, idx, func, "any")
}

pub fn take_many(
    sv: &mut CoState,
    idx: usize,
    _func: &str,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let vals = sv.take_stack_many(idx + 1, ValueCount::VarArg)?;
    Ok(vals.into_vec())
}

fn union_matches(v: &RuntimeValue, m: &str) -> bool {
    match m {
        ctype::NUM | ctype::FLO => v.is_number(),
        ctype::INT => matches!(v, RuntimeValue::Int(..)),
        ctype::STR => v.is_string(),
        ctype::BOO => v.is_bool(),
        ctype::TAB => v.is_table(),
        ctype::FUN => v.is_function(),
        ctype::NIL => matches!(v, RuntimeValue::Nil),
        ctype::ANY => true,
        _ => v.type_name_of() == m,
    }
}

/// Accepts any value whose type is one of the allowed members, returning it unchanged so the caller keeps the runtime type
pub fn take_union(
    sv: &mut CoState,
    idx: usize,
    func: &str,
    members: &[&'static str],
    want: &'static str,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let v = get(sv, idx, func, want)?;
    if !members.iter().any(|m| union_matches(&v, m)) {
        return Err(bad(idx, func, &v, want));
    }
    Ok(v)
}

pub type DukaResult = Vec<RuntimeValue>;
pub type DukaIterator = Vec<RuntimeValue>;

/// For result
pub fn ok(val: RuntimeValue) -> DukaResult {
    vec![RuntimeValue::Bool(true), val]
}
/// For results
pub fn oks<const N: usize>(vals: [RuntimeValue; N]) -> DukaResult {
    let mut v = vec![RuntimeValue::Bool(true)];
    v.extend(vals);
    v
}
/// For result
pub fn err<E: Error>(heap: &mut Heap, e: E) -> DukaResult {
    vec![
        RuntimeValue::Bool(false),
        RuntimeValue::from_string(heap, e.to_string()),
    ]
}

/// For iterator
pub fn item(val: RuntimeValue) -> DukaIterator {
    vec![RuntimeValue::Bool(true), val]
}
/// For iterator
pub fn items<const N: usize>(vals: [RuntimeValue; N]) -> DukaIterator {
    let mut v = vec![RuntimeValue::Bool(true)];
    v.extend(vals);
    v
}
/// For iterator
pub fn stop() -> DukaIterator {
    vec![RuntimeValue::Bool(false)]
}
