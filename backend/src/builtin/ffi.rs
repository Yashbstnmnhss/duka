use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};

use crate::errors::DukaRuntimeError;

duka_builtin_def! {
    mod ffi
    doc "FFI (Foreign Function Interface)"
    flags(@feature(ffi))
    fn {
        meta: impl_cdef
    }
}

#[duka_builtin]
pub fn impl_cdef() -> Result<(), DukaRuntimeError> {
    Ok(())
}
