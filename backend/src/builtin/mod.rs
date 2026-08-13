use duka_gc::{Gc, GcCell, Heap};
use duka_shared::builtin::Builtins;
use duka_shared::constants::MetaMethod;
use duka_shared::docs::MetaInfo;
use duka_shared::types::ValueCount;

use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure};
use crate::vm::VMContext;
use crate::vm::coroutine::{CoState, NativeApi, call_native_meta_sync};

macro_rules! register_module {
    (global $module:ident [$heap: ident, $ctx: ident]) => {
        for (name, func) in $module::registry().into_inner() {
            $ctx.register_func(
                $heap,
                name,
                RustClosure::returns(func.as_closure(), Some(name.into())),
            );
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

mod array;
mod core;
mod iter;
mod math;
mod os;
pub mod require;
mod string;
mod table;

pub mod arg;

type PlainBuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;
type CoBuiltinFn =
    fn(&mut CoState, &mut Heap, &mut NativeApi) -> Result<ValueCount, DukaRuntimeError>;

pub enum BuiltinFn {
    Plain(PlainBuiltinFn),
    Co(CoBuiltinFn),
}

impl BuiltinFn {
    pub fn as_closure(
        self,
    ) -> Box<
        dyn FnMut(&mut CoState, &mut Heap, &mut NativeApi) -> Result<ValueCount, DukaRuntimeError>,
    > {
        match self {
            BuiltinFn::Plain(f) => Box::new(move |c, h, _n| f(c, h)),
            BuiltinFn::Co(f) => Box::new(f),
        }
    }
}

pub fn all_builtin_metas() -> Vec<MetaInfo> {
    let mut metas = core::builtin_metas();
    metas.extend(table::builtin_metas());
    metas.extend(string::builtin_metas());
    metas.extend(math::builtin_metas());
    metas.extend(array::builtin_metas());
    metas.extend(iter::builtin_metas());
    metas
}

/// # All Standard Library for Duka
pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    register_core(heap, ctx);
}

/// # Core Library for Duka
/// **unrelated to platform**
pub fn register_core(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    register_module!(global core [heap, ctx]);
    register_module!(table [heap, ctx]);
    register_module!(string [heap, ctx]);
    register_module!(math [heap, ctx] const);
    register_module!(array [heap, ctx]);
    register_module!(iter [heap, ctx]);
}

fn register_builtin_module(
    module_funcs: Builtins<BuiltinFn>,
    module_consts: Option<Builtins<RuntimeValue>>,
    name: impl Into<String>,
    heap: &mut Heap,
    ctx: &mut VMContext,
) {
    let name = name.into();
    let mut table = RuntimeDukaTable::new(module_funcs.len());
    for (k, v) in module_funcs.into_inner() {
        let func = heap.alloc(GcCell::new(RustClosure::returns(
            v.as_closure(),
            Some(format!("{}.{}", &name, k).into_boxed_str()),
        )));
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
    if v.type_name_of() != t {
        return Err(DukaRuntimeError::ArgumentInvalidType(
            param,
            func.into(),
            t,
            v.type_name_of(),
        ));
    }
    Ok(())
}

fn call_meta(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
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
        RuntimeValue::UserFunc(closure) => {
            sv.call_user_sync(h, api, RuntimeValue::UserFunc(closure), &ps)?
        }
        RuntimeValue::NativeFunc(closure) => call_native_meta_sync(sv, h, api, closure, &ps)?,
        _ => return Ok(None),
    };
    Ok(Some(r))
}
