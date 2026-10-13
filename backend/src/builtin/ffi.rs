use crate::{
    builtin::arg::err, errors::DukaRuntimeError, value::RuntimeValue, vm::coroutine::NativeApi,
};
use duka_ffi::FFI;
use duka_gc::Heap;
use duka_macros::{duka_builtin, duka_builtin_def};

duka_builtin_def! {
    mod ffi
    doc "FFI (Foreign Function Interface)"
    flags(@feature(ffi))
    fn {
        meta: impl_cdef co,
              impl_refresh co
    }
}

#[duka_builtin(params(code: string), flags(@returns(result)))]
pub fn impl_cdef(
    heap: &mut Heap,
    api: &mut NativeApi,
    code: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    if let Some(globals) = api.vm_globals.clone() {
        let mut b = globals.borrow_mut();
        if let Some(e) = b.get_mut::<FFI>() {
            if let Err(error) = e.define(&code) {
                return Ok(err(heap, error));
            }
        } else {
            let mut ffi = FFI::new();
            if let Err(error) = ffi.define(&code) {
                return Ok(err(heap, error));
            }
            b.put(ffi);
        }
        Ok(vec![RuntimeValue::Bool(true)])
    } else {
        Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()))
    }
}

#[duka_builtin]
pub fn impl_refresh(api: &mut NativeApi) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    if let Some(globals) = api.vm_globals.clone() {
        let mut b = globals.borrow_mut();
        if let Some(e) = b.get_mut::<FFI>() {
            e.rebind();
        }
        Ok(vec![RuntimeValue::Bool(true)])
    } else {
        Err(DukaRuntimeError::Custom("FFI isn't supported".to_owned()))
    }
}
