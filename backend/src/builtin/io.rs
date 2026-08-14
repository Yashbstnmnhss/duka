use std::fs::File;

use crate::builtin::arg::{err, ok};
use crate::value::RuntimeValue;
use crate::{builtin::BuiltinFn, errors::DukaRuntimeError};
use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};

duka_builtin_def! {
    mod io
    fn {

    }
    const {}
}

#[duka_builtin(
    doc = "",
    params(name: string),
    returns(any)
)]
fn impl_open(
    heap: &mut duka_gc::Heap,
    name: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    Ok(ok(match FileData::new(name) {
        Ok(v) => v.into_value(heap),
        Err(e) => return Ok(err(heap, e)),
    }))
}

duka_user_data! {
    struct FileData {
        inner: File
    }
    constructor fn new(path: String) -> Result<Self, DukaRuntimeError> {
        Ok(Self {
            inner: File::open(path).map_err(|e| DukaRuntimeError::IOError(e.to_string()))?
        })
    }
}
