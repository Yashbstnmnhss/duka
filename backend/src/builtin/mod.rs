use std::cmp::Ordering;

use duka_gc::{Gc, GcCell, Heap};
use duka_shared::builtin::Builtins;
use duka_shared::constants::MetaMethod;
#[cfg(feature = "docs")]
use duka_shared::docs::MetaInfo;
use duka_shared::types::ValueCount;
use duka_shared::value::DukaInt;

use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeDukaTable, RuntimeValue, RustClosure, UserData};
use crate::vm::VMContext;
use crate::vm::coroutine::{CoState, NativeApi, call_native_meta_sync};

pub mod prelude {
    pub use crate::builtin::{
        BuiltinFn, CoBuiltinFn, PlainBuiltinFn, arg::DukaIterator, arg::DukaResult, arg::err,
        arg::item, arg::items, arg::ok, arg::oks, arg::stop, call_compare_meta, call_meta_method,
        get_string, normalize,
    };
}

macro_rules! register_module {
    (global $module:ident [$heap: ident, $ctx: ident]) => {
        for (name, func) in $module::registry().into_inner() {
            $ctx.register_func(
                $heap,
                name,
                RustClosure::returns(func.into_closure(), Some(name.into())),
            );
        }
    };
    ($module:path [$heap: ident, $ctx: ident]) => {{
        use $module::{MODULE_NAME, get_registry_table};
        let tab = get_registry_table($heap);
        register_builtin_module(MODULE_NAME, tab, $heap, $ctx);
    }};
}

mod array;
mod core;
mod iter;
mod math;
mod regex;
pub mod require;
mod string;
mod table;

#[cfg(all(feature = "io", not(target_arch = "wasm32")))]
mod io;
#[cfg(all(feature = "os", not(target_arch = "wasm32")))]
mod os;

pub mod arg;

pub type PlainBuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;
pub type CoBuiltinFn =
    fn(&mut CoState, &mut Heap, &mut NativeApi) -> Result<ValueCount, DukaRuntimeError>;

pub enum BuiltinFn {
    Plain(PlainBuiltinFn),
    Co(CoBuiltinFn),
}

impl BuiltinFn {
    pub fn into_closure(
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

#[cfg(feature = "docs")]
pub fn all_builtin_metas() -> Vec<MetaInfo> {
    let mut metas = vec![
        core::MODULE_META,
        table::MODULE_META,
        array::MODULE_META,
        string::MODULE_META,
        math::MODULE_META,
        iter::MODULE_META,
        regex::MODULE_META,
    ];
    #[cfg(all(feature = "os", not(target_arch = "wasm32")))]
    metas.push(os::MODULE_META);
    #[cfg(all(feature = "io", not(target_arch = "wasm32")))]
    metas.push(io::MODULE_META);
    metas
}

/// # All Standard Library for Duka
pub fn register_all(heap: &mut Heap, ctx: &mut VMContext) {
    register_core(heap, ctx);
    register_std(heap, ctx);
}

/// # Platform-based Standard Library for Duka
/// **depends on platform**
pub fn register_std(heap: &mut Heap, ctx: &mut VMContext) {
    #[cfg(all(feature = "os", not(target_arch = "wasm32")))]
    register_module!(os [heap, ctx]);
    #[cfg(all(feature = "io", not(target_arch = "wasm32")))]
    register_module!(io [heap, ctx]);
}

/// # Core Library for Duka
/// **unrelated to platform**
pub fn register_core(heap: &mut Heap, ctx: &mut VMContext) {
    require::init();
    register_module!(global core [heap, ctx]);
    register_module!(table [heap, ctx]);
    register_module!(string [heap, ctx]);
    register_module!(math [heap, ctx]);
    register_module!(array [heap, ctx]);
    register_module!(iter [heap, ctx]);
    register_module!(regex [heap, ctx]);
}

pub fn make_module_table(
    module_funcs: Builtins<BuiltinFn>,
    module_consts: Builtins<RuntimeValue>,
    sub_modules: Builtins<RuntimeDukaTable>,
    name: impl Into<String>,
    heap: &mut Heap,
) -> RuntimeDukaTable {
    let name = name.into();
    let mut table = RuntimeDukaTable::new(module_funcs.len());
    for (k, v) in module_funcs.into_inner() {
        let func = heap.alloc(GcCell::new(RustClosure::returns(
            v.into_closure(),
            Some(format!("{}.{}", name, k).into_boxed_str()),
        )));
        table.set_by_key(heap, k.into(), RuntimeValue::NativeFunc(func));
    }
    for (k, v) in module_consts.into_inner() {
        table.set_by_key(heap, k.into(), v);
    }
    for (k, v) in sub_modules.into_inner() {
        let v = RuntimeValue::Table(heap.alloc(GcCell::new(v)));
        table.set_by_key(heap, k.into(), v);
    }
    table
}

fn register_builtin_module(
    name: impl Into<String>,
    table: RuntimeDukaTable,
    heap: &mut Heap,
    ctx: &mut VMContext,
) {
    ctx.register_table(heap, name.into(), table);
}

pub fn ensure_type(
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

pub fn format_arg(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    val: &RuntimeValue,
) -> Result<String, DukaRuntimeError> {
    match val {
        rv if rv.is_metamethod() => {
            Ok(
                call_meta_method(sv, h, api, &rv, MetaMethod::ToString, &[], true)?
                    .map(|v| v.into_iter().next().map(|v| v.to_string()))
                    .flatten()
                    .unwrap_or_else(|| rv.to_string()),
            )
        }
        _ => Ok(format!("{}", val)),
    }
}

/// 规范化索引:非负 clamp 到 len;负值按尾部回绕(小于 len 的负数即 `len+i`)
///
/// See docs/stdlib.md
pub fn normalize(i: DukaInt, len: usize) -> usize {
    if i >= 0 {
        (i as usize).min(len)
    } else {
        len.saturating_sub(i.unsigned_abs() as usize)
    }
}

pub fn call_compare_meta(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    x: &RuntimeValue,
    y: &RuntimeValue,
) -> Result<Option<Ordering>, DukaRuntimeError> {
    if !x.is_metamethod() || !y.is_metamethod() {
        return Ok(None);
    }
    Ok(Some(
        if call_meta_method(sv, h, api, x, MetaMethod::LT, std::slice::from_ref(y), true)?
            .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
            .flatten()
            .unwrap_or_default()
        {
            Ordering::Less
        } else if call_meta_method(sv, h, api, x, MetaMethod::Eq, std::slice::from_ref(y), true)?
            .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
            .flatten()
            .unwrap_or_default()
        {
            Ordering::Equal
        } else if call_meta_method(sv, h, api, y, MetaMethod::LT, std::slice::from_ref(x), true)?
            .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
            .flatten()
            .unwrap_or_default()
        {
            Ordering::Greater
        } else if call_meta_method(sv, h, api, y, MetaMethod::Eq, std::slice::from_ref(x), true)?
            .map(|v| v.into_iter().next().map(|i| i.eval_to_bool()))
            .flatten()
            .unwrap_or_default()
        {
            Ordering::Equal
        } else {
            Ordering::Greater
        },
    ))
}

pub fn get_string(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    who: RuntimeValue,
) -> Result<String, DukaRuntimeError> {
    Ok(match who {
        rv if rv.is_string() => rv.eval_to_string().to_string(),
        rv if rv.is_metamethod() => {
            call_meta_method(sv, h, api, &rv, MetaMethod::ToString, &[], true)?
                .map(|v| v.into_iter().next().map(|v| v.to_string()))
                .flatten()
                .unwrap_or_else(|| {
                    if matches!(rv, RuntimeValue::Table(..)) {
                        "table".to_owned()
                    } else {
                        rv.to_string()
                    }
                })
        }
        _ => who.to_string(),
    })
}

pub fn call_meta_method(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    val: &RuntimeValue,
    meta: MetaMethod,
    params: &[RuntimeValue],
    with_self: bool,
) -> Result<Option<Vec<RuntimeValue>>, DukaRuntimeError> {
    if !val.is_metamethod() {
        return Ok(None);
    }
    match val {
        RuntimeValue::Table(t) => {
            call_table_meta_method(sv, h, api, t.clone(), meta, params, with_self)
        }
        RuntimeValue::UserData(ud) => {
            call_user_data_meta_method(sv, h, api, ud.clone(), meta, params, with_self)
        }
        _ => unimplemented!(),
    }
}

pub fn call_user_data_meta_method(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    ud: Gc<GcCell<UserData>>,
    meta: MetaMethod,
    params: &[RuntimeValue],
    with_self: bool,
) -> Result<Option<Vec<RuntimeValue>>, DukaRuntimeError> {
    let Some(m) = ud.borrow().get_meta_method(h, &meta) else {
        return Ok(None);
    };
    if !m.is_function() {
        return Ok(None);
    }
    let ps = if with_self {
        [&[RuntimeValue::UserData(ud)], params].concat()
    } else {
        params.to_vec()
    };
    let r = match m {
        RuntimeValue::UserFunc(closure) => {
            sv.call_user_protected(h, api, RuntimeValue::UserFunc(closure), &ps)?
        }
        RuntimeValue::NativeFunc(closure) => call_native_meta_sync(sv, h, api, closure, &ps)?,
        _ => return Ok(None),
    };
    Ok(Some(r))
}

pub fn call_table_meta_method(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    t: Gc<GcCell<RuntimeDukaTable>>,
    meta: MetaMethod,
    params: &[RuntimeValue],
    with_self: bool,
) -> Result<Option<Vec<RuntimeValue>>, DukaRuntimeError> {
    let Some(m) = t.borrow().get_meta_method(h, &meta) else {
        return Ok(None);
    };
    if !m.is_function() {
        return Ok(None);
    }
    let ps = if with_self {
        [&[RuntimeValue::Table(t)], params].concat()
    } else {
        params.to_vec()
    };
    let r = match m {
        RuntimeValue::UserFunc(closure) => {
            sv.call_user_protected(h, api, RuntimeValue::UserFunc(closure), &ps)?
        }
        RuntimeValue::NativeFunc(closure) => call_native_meta_sync(sv, h, api, closure, &ps)?,
        _ => return Ok(None),
    };
    Ok(Some(r))
}
