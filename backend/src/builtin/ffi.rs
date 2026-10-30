use std::ffi::CString;

use crate::{
    builtin::arg::{err, ok, oks},
    errors::DukaRuntimeError,
    value::RuntimeValue,
    vm::coroutine::NativeApi,
};
use duka_ffi::{FFI, bridge::Value};
use duka_gc::Heap;
use duka_shared::value::DukaInt;
use duka_macros::{duka_builtin, duka_builtin_def};

duka_builtin_def! {
    mod ffi
    doc "FFI (Foreign Function Interface)"
    example r#"ffi.cdef("int abs(int n);")
ffi.load("libc.so.6")
local ok, n = ffi.call("abs", -1)"#
    flags(@feature(ffi))
    fn {
        meta: impl_cdef co,
              impl_refresh co,
              impl_load co,
              impl_close co,
              impl_call co,
              impl_get co,
              impl_set co
    }
}

#[duka_builtin(
    params(code: string),
    flags(@returns(result)),
    doc = "Declares the C types and functions a call reads, written the way a header writes them"
)]
pub fn impl_cdef(
    heap: &mut Heap,
    api: &mut NativeApi,
    code: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut_or_put(FFI::new()) else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    match ffi.define(&code) {
        Ok(()) => Ok(oks([])),
        Err(error) => Ok(err(heap, error)),
    }
}

#[duka_builtin(
    flags(@returns(result)),
    doc = "Looks the declared names up again, after a library was opened"
)]
pub fn impl_refresh(
    api: &mut NativeApi,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    if let Some(ffi) = b.get_mut::<FFI>() {
        ffi.rebind();
    }
    Ok(oks([]))
}

#[duka_builtin(
    params(path: string),
    flags(@returns(result)),
    doc = "Opens a shared library and returns the id a call can name it by"
)]
pub fn impl_load(
    heap: &mut Heap,
    api: &mut NativeApi,
    path: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut_or_put(FFI::new()) else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    match ffi.load(&path) {
        Ok(id) => Ok(ok(RuntimeValue::Int(id as DukaInt))),
        Err(error) => Ok(err(heap, error)),
    }
}

#[duka_builtin(
    params(id: int),
    doc = "Closes the library an id names, answering whether it was open"
)]
pub fn impl_close(api: &mut NativeApi, id: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut::<FFI>() else {
        return Ok(RuntimeValue::Bool(false));
    };
    Ok(RuntimeValue::Bool(ffi.close(id as usize)))
}

#[duka_builtin(
    params(name: string, args: vararg),
    flags(@returns(result)),
    doc = "Calls a declared C function with the given arguments"
)]
pub fn impl_call(
    heap: &mut Heap,
    api: &mut NativeApi,
    name: String,
    args: Vec<RuntimeValue>,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    // a C string has to outlive the call, so what is built here is kept until
    // the answer is read
    let mut keep: Vec<CString> = Vec::with_capacity(args.len());
    let mut values: Vec<Value> = Vec::with_capacity(args.len());
    for arg in &args {
        match to_value(arg, &mut keep) {
            Ok(value) => values.push(value),
            Err(message) => {
                return Ok(err(heap, DukaRuntimeError::Custom(message)));
            }
        }
    }

    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut_or_put(FFI::new()) else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    match ffi.call(None, &name, &values) {
        Ok(value) => Ok(match from_value(value) {
            Some(value) => ok(value),
            None => oks([]),
        }),
        Err(error) => Ok(err(heap, error)),
    }
}

#[duka_builtin(
    params(name: string),
    flags(@returns(result)),
    doc = "Reads a declared C variable"
)]
pub fn impl_get(
    heap: &mut Heap,
    api: &mut NativeApi,
    name: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut_or_put(FFI::new()) else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    match ffi.get_var(None, &name) {
        Ok(value) => Ok(match from_value(value) {
            Some(value) => ok(value),
            None => oks([]),
        }),
        Err(error) => Ok(err(heap, error)),
    }
}

#[duka_builtin(
    params(name: string, value: any),
    flags(@returns(result)),
    doc = "Writes a declared C variable"
)]
pub fn impl_set(
    heap: &mut Heap,
    api: &mut NativeApi,
    name: String,
    value: RuntimeValue,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let mut keep: Vec<CString> = Vec::with_capacity(1);
    let value = match to_value(&value, &mut keep) {
        Ok(value) => value,
        Err(message) => return Ok(err(heap, DukaRuntimeError::Custom(message))),
    };
    let Some(globals) = api.vm_globals.clone() else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    let mut b = globals.borrow_mut();
    let Some(ffi) = b.get_mut_or_put(FFI::new()) else {
        return Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()));
    };
    match ffi.set_var(None, &name, value) {
        Ok(()) => Ok(oks([])),
        Err(error) => Ok(err(heap, error)),
    }
}

/// A Duka value as the C value a call passes.
///
/// A number keeps its full width, because what a parameter wants is read off
/// the declaration and narrowing it here would lose what the declaration could
/// have used. A string is copied and kept alive for the length of the call; a
/// value C has no shape for is refused rather than cast.
fn to_value(value: &RuntimeValue, keep: &mut Vec<CString>) -> Result<Value, String> {
    match value {
        RuntimeValue::Nil => Ok(Value::Ptr(std::ptr::null_mut())),
        RuntimeValue::Int(i) => Ok(Value::LongLong(*i)),
        RuntimeValue::Float(f) => Ok(Value::Double(*f)),
        RuntimeValue::Bool(b) => Ok(Value::LongLong(*b as i64)),
        value if value.is_string() => {
            let text = value.eval_to_string();
            let c = CString::new(text.as_bytes())
                .map_err(|_| "a C string cannot hold a NUL byte".to_owned())?;
            let ptr = c.as_ptr() as *mut std::ffi::c_void;
            keep.push(c);
            Ok(Value::Ptr(ptr))
        }
        other => Err(format!(
            "a value of type \"{}\" has no C form",
            other.type_name_of()
        )),
    }
}

/// A C value as the Duka value a call hands back.
///
/// `void` and a null pointer are `nil`: there is no value, and a null pointer
/// is the C spelling of not having one.
fn from_value(value: Value) -> Option<RuntimeValue> {
    Some(match value {
        Value::Void => return None,
        Value::Char(v) => RuntimeValue::Int(v as DukaInt),
        Value::UChar(v) => RuntimeValue::Int(v as DukaInt),
        Value::Short(v) => RuntimeValue::Int(v as DukaInt),
        Value::UShort(v) => RuntimeValue::Int(v as DukaInt),
        Value::Int(v) => RuntimeValue::Int(v as DukaInt),
        Value::UInt(v) => RuntimeValue::Int(v as DukaInt),
        Value::Long(v) | Value::LongLong(v) => RuntimeValue::Int(v),
        Value::ULong(v) => RuntimeValue::Int(v as DukaInt),
        Value::ULongLong(v) => RuntimeValue::Int(v as DukaInt),
        Value::Float(v) => RuntimeValue::Float(v as f64),
        Value::Double(v) => RuntimeValue::Float(v),
        Value::Ptr(ptr) if ptr.is_null() => RuntimeValue::Nil,
        Value::Ptr(ptr) => RuntimeValue::Int(ptr as usize as DukaInt),
    })
}
