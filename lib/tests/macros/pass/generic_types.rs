use duka_lib::duka_gc::Heap;
use duka_lib::errors::DukaRuntimeError;
use duka_lib::value::RuntimeValue;
use duka_macros::duka_builtin;

#[duka_builtin(
    name = "gather",
    doc = "generic types",
    params(
        items: array<int>,
        index: table<string, int>,
        cb: fn(int)->bool,
        opt: array<string> | nil = RuntimeValue::Nil
    ),
    returns(array<int>)
)]
fn impl_gather(
    _h: &mut Heap,
    items: RuntimeValue,
    index: RuntimeValue,
    cb: RuntimeValue,
    opt: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let _ = (index, cb, opt);
    Ok(items)
}

#[duka_builtin(
    type = "array<int>",
    name = "COPIES",
    doc = "how many copies",
    value = "how many"
)]
const DUKA_COPIES: RuntimeValue = RuntimeValue::Nil;

fn main() {}