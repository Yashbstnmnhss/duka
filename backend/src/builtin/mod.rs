use duka_gc::{GcCell, Heap};
use duka_shared::builtin::Builtins;
use duka_shared::types::ValueCount;

use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure};
use crate::vm::VMContext;
use crate::vm::coroutine::CoState;

mod core;
mod math;
pub mod require;
mod string;
mod table;

type BuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;

pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    let registry = core::registry().into_inner();
    for (name, func) in registry {
        ctx.register_func(heap, name, RustClosure::returns(func));
    }

    register_builtin_module(table::registry(), None, "table", heap, ctx);
    register_builtin_module(string::registry(), None, "string", heap, ctx);
    register_builtin_module(
        math::registry(),
        Some(math::consts_registry()),
        "math",
        heap,
        ctx,
    );
}

fn register_builtin_module(
    module_funcs: Builtins<BuiltinFn>,
    module_consts: Option<Builtins<RuntimeValue>>,
    name: impl Into<String>,
    heap: &mut Heap,
    ctx: &mut VMContext,
) {
    let mut table = RuntimeDukaTable::new(module_funcs.len());
    for (k, v) in module_funcs.into_inner() {
        let func = heap.alloc(GcCell::new(RustClosure::returns(v)));
        table.set_by_key(heap, k.into(), RuntimeValue::NativeFunc(func));
    }
    if let Some(module_consts) = module_consts {
        for (k, v) in module_consts.into_inner() {
            table.set_by_key(heap, k.into(), v);
        }
    }
    ctx.register_table(heap, name, table);
}

fn ensure_type(
    v: &RuntimeValue,
    t: &'static str,
    func: impl Into<String>,
    param: usize,
) -> Result<(), DukaRuntimeError> {
    if v.type_of() != t {
        return Err(DukaRuntimeError::ArgumentInvalidType(
            param,
            func.into(),
            t,
            v.type_of(),
        ));
    }
    Ok(())
}
fn optional(
    c: &mut CoState,
    idx: usize,
    default: RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    if !c.ensure_address(idx + 1) {
        return Ok(default);
    }
    c.get_stack(idx + 1).cloned()
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
