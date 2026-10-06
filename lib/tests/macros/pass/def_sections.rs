use duka_lib::duka_gc::Heap;
use duka_lib::errors::DukaRuntimeError;
use duka_macros::{duka_builtin, duka_builtin_def};

#[duka_builtin(name = "f", params(x: int), returns(int))]
fn impl_f(_h: &mut Heap, x: i64) -> Result<(), DukaRuntimeError> {
    let _ = x;
    Ok(())
}

duka_builtin_def! {
    mod t
    doc "reordered sections"
    const {}
    fn {
        plain: impl_f
    }
}

fn main() {}