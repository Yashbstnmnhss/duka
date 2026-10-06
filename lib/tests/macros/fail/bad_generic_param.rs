use duka_lib::duka_gc::Heap;
use duka_lib::errors::DukaRuntimeError;
use duka_lib::value::RuntimeValue;
use duka_macros::duka_builtin;

#[duka_builtin(name = "f", params(x: table<int>), returns(int))]
fn impl_f(_h: &mut Heap, x: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(x)
}

fn main() {}