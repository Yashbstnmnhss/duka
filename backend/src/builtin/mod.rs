use duka_gc::{GcCell, Heap};
use duka_shared::builtin::Builtins;
use duka_shared::types::ValueCount;

use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure};
use crate::vm::VMContext;
use crate::vm::coroutine::CoState;

mod core;
pub mod require;
mod table;

type BuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;

pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    let registry = core::registry().into_inner();
    for (name, func) in registry {
        ctx.register_func(heap, name, RustClosure::returns(func));
    }

    register_builtin_module(table::registry(), "table", heap, ctx);
}

fn register_builtin_module(
    module: Builtins<BuiltinFn>,
    name: impl Into<String>,
    heap: &mut Heap,
    ctx: &mut VMContext,
) {
    let mut table = RuntimeDukaTable::new(module.len());
    for (k, v) in module.into_inner() {
        let func = heap.alloc(GcCell::new(RustClosure::returns(v)));
        table.set_by_key(heap, k.into(), RuntimeValue::NativeFunc(func));
    }
    ctx.register_table(heap, name, table);
}

fn required(
    c: &mut CoState,
    idx: usize,
    func: impl Into<String>,
    msg: impl Into<String>,
) -> Result<&RuntimeValue, DukaRuntimeError> {
    if !c.ensure_address(idx + 1) {
        return Err(DukaRuntimeError::ArgumentMissing(
            idx,
            func.into(),
            msg.into(),
        ));
    }
    c.get_stack(idx + 1)
}
