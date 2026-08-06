use duka_gc::{Gc, GcCell, Heap};
use duka_shared::builtin::Builtins;
use duka_shared::builtin_meta::MetaInfo;
use duka_shared::constants::MetaMethod;
use duka_shared::types::ValueCount;

use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure};
use crate::vm::VMContext;
use crate::vm::coroutine::{CoState, call_native_meta_sync, sync_meta_call};

macro_rules! define_builtins {
    (
        fn:
        $( plain: $($pname:literal => $pfunc:ident),+ $(,)? ; )?
        $( meta: $($mname:literal => $mfunc:ident, $mmeta:ident),+ $(,)? ; )?
        const:
        $( plain: $($cpname:literal => $cpconst:expr),+ $(,)? ; )?
        $( meta: $($cmname:literal => $cmconst:expr, $cmmeta:ident),+ $(,)? ; )?
    ) => {
        pub fn registry() -> ::duka_shared::builtin::Builtins<$crate::builtin::BuiltinFn> {
            ::duka_shared::builtin::Builtins::new()
                $(
                    $( .register($pname, $pfunc as $crate::builtin::BuiltinFn) )+
                )?
                $(
                    $( .register_meta($mname, $mfunc as $crate::builtin::BuiltinFn, $mmeta) )+
                )?
        }
        pub fn consts_registry() -> ::duka_shared::builtin::Builtins<$crate::value::RuntimeValue> {
            ::duka_shared::builtin::Builtins::new()
                $(
                    $( .register($cpname, $cpconst) )+
                )?
                $(
                    $( .register_meta($cmname, $cmconst, $cmmeta) )+
                )?
        }
        pub(super) fn builtin_metas() -> Vec<::duka_shared::builtin_meta::MetaInfo> {
            let mut v = registry().into_metas();
            v.extend(consts_registry().into_metas());
            v
        }
    };
}
macro_rules! register_module {
    (global $module:ident [$heap: ident, $ctx: ident]) => {
        for (name, func) in $module::registry().into_inner() {
            $ctx.register_func($heap, name, RustClosure::returns(func));
        }
    };
    ($module:ident [$heap: ident, $ctx: ident]) => {
        register_builtin_module($module::registry(), None, stringify!($module), $heap, $ctx);
    };
    ($module:ident [$heap: ident, $ctx: ident] const) => {
        register_builtin_module(
            $module::registry(),
            Some($module::consts_registry()),
            stringify!($module),
            $heap,
            $ctx,
        );
    };
}

mod core;
mod math;
pub mod require;
mod string;
mod table;

pub mod arg;

type BuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;

pub fn all_builtin_metas() -> Vec<MetaInfo> {
    let mut metas = core::builtin_metas();
    metas.extend(table::builtin_metas());
    metas.extend(string::builtin_metas());
    metas.extend(math::builtin_metas());
    metas
}

pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    register_module!(global core [heap, ctx]);
    register_module!(table [heap, ctx]);
    register_module!(string [heap, ctx]);
    register_module!(math [heap, ctx] const);
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
// fn optional(
//     c: &mut CoState,
//     idx: usize,
//     default: RuntimeValue,
// ) -> Result<RuntimeValue, DukaRuntimeError> {
//     if !c.ensure_address(idx + 1) {
//         return Ok(default);
//     }
//     c.get_stack(idx + 1).cloned()
// }
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

fn call_meta(
    sv: &mut CoState,
    h: &mut Heap,
    t: Gc<GcCell<RuntimeDukaTable>>,
    meta: MetaMethod,
    params: &[RuntimeValue],
) -> Result<Option<RuntimeValue>, DukaRuntimeError> {
    let Some(m) = t.borrow().get_meta_method(h, &meta) else {
        return Ok(None);
    };
    if !m.is_function() {
        return Ok(None);
    }
    let ps = [&[RuntimeValue::Table(t)], params].concat();
    let r = match m {
        RuntimeValue::UserFunc(closure) => sync_meta_call(sv, h, closure, &ps)?,
        RuntimeValue::NativeFunc(closure) => call_native_meta_sync(sv, h, closure, &ps)?,
        _ => return Ok(None),
    };
    Ok(Some(r))
}
