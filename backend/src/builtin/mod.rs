use duka_gc::Heap;

use crate::value::RustClosure;
use crate::vm::VMContext;

mod core;
pub mod require;

pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    let registry = core::registry().into_inner();
    for (name, func) in registry {
        ctx.register_func(heap, name, RustClosure::returns(func));
    }
}
