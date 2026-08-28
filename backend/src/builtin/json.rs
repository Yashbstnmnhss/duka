use duka_gc::Heap;
use duka_macros::{duka_builtin, duka_builtin_def};

use crate::{
    builtin::{
        arg::{err, ok},
        require::{json_to_runtime, runtime_to_json},
    },
    errors::DukaRuntimeError,
    value::RuntimeValue,
};

duka_builtin_def! {
    mod json
    doc "Read & write JSON data format"
    flags(@feature(json))
    fn {
        meta:
            impl_parse,
            impl_stringify
    }
    const {}
}

#[duka_builtin(doc = "Parse a JSON string", params(json: string), returns(vararg), flags(@returns(result)))]
fn impl_parse(heap: &mut Heap, json: String) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(v) => Ok(ok(json_to_runtime(heap, &v)?)),
        Err(e) => Ok(err(heap, e)),
    }
}

#[duka_builtin(doc = "Convert a value into JSON string", params(val: any), returns(vararg), flags(@returns(result)))]
fn impl_stringify(
    heap: &mut Heap,
    val: RuntimeValue,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let v = match runtime_to_json(&val) {
        Ok(v) => v,
        Err(e) => return Ok(err(heap, e)),
    };
    match serde_json::to_string(&v) {
        Ok(v) => Ok(ok(RuntimeValue::from_string(heap, v))),
        Err(e) => return Ok(err(heap, e)),
    }
}
