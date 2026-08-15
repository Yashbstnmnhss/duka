use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use duka_gc::Heap;
use duka_macros::duka_builtin;

use crate::errors::DukaRuntimeError;
use crate::value::{DukaClosure, DukaProto, RuntimeValue, UpValue};
use crate::vm::coroutine::{CoState, NativeApi};

/// Function that loads a module by name and returns its compiled proto.
pub type ModuleLoader = dyn Fn(&str) -> Result<DukaProto, String> + Send + Sync;

struct ModuleStore {
    cache: UnsafeCell<HashMap<String, DukaProto>>,
    loading: UnsafeCell<HashSet<String>>,
    loader: UnsafeCell<Option<Box<ModuleLoader>>>,
}
// OK: single-threaded access only
unsafe impl Sync for ModuleStore {}
unsafe impl Send for ModuleStore {}

static MODULE_STORE: OnceLock<ModuleStore> = OnceLock::new();

fn store() -> &'static ModuleStore {
    MODULE_STORE.get_or_init(|| ModuleStore {
        cache: UnsafeCell::new(HashMap::new()),
        loading: UnsafeCell::new(HashSet::new()),
        loader: UnsafeCell::new(None),
    })
}

pub fn init() {
    store();
}

/// Clear the module cache, in-flight set and loader.
pub fn reset() {
    let s = store();
    unsafe {
        (*s.cache.get()).clear();
        (*s.loading.get()).clear();
        *s.loader.get() = None;
    }
}

/// Set the module loader used by `require()`.
///
/// Should be called once by the embedding program before
/// any `require()` call. The loader receives the module name and returns the
/// compiled module proto; `require()` executes it in the caller's VM.
pub fn set_loader<F>(loader: F)
where
    F: Fn(&str) -> Result<DukaProto, String> + Send + Sync + 'static,
{
    let s = store();
    unsafe {
        *s.loader.get() = Some(Box::new(loader));
    }
}

#[duka_builtin(name = "require", doc = "Import module by pattern", params(pattern: string))]
pub fn impl_require(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    pattern: String,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let name = pattern;
    let s = store();

    if let Some(cache) = api.module_cache() {
        let key = RuntimeValue::from_string(h, name.clone());
        if let Some(val) = cache.borrow().get(&key).cloned() {
            return Ok(val);
        }
    }

    if unsafe { (*s.loading.get()).contains(&name) } {
        return Err(DukaRuntimeError::ModuleError(format!(
            "circular require: {name}"
        )));
    }

    struct LoadingGuard<'a>(&'a ModuleStore, String);
    impl Drop for LoadingGuard<'_> {
        fn drop(&mut self) {
            unsafe {
                (*self.0.loading.get()).remove(&self.1);
            }
        }
    }
    unsafe {
        (*s.loading.get()).insert(name.clone());
    }
    let _guard = LoadingGuard(s, name.clone());

    let loader = unsafe { &*s.loader.get() };
    let proto = match loader {
        Some(f) => f(&name).map_err(DukaRuntimeError::ModuleError),
        None => Err(DukaRuntimeError::ModuleError(format!(
            "Module system not configured: no loader set (call `set_loader` first)"
        ))),
    }?;
    unsafe {
        (*s.cache.get()).insert(name.clone(), proto.clone());
    }

    let globals = api.globals().ok_or_else(|| {
        DukaRuntimeError::ModuleError("module system requires a running VM".to_owned())
    })?;
    let closure = DukaClosure::from_proto(h.alloc(proto))
        .set_up_value(h, UpValue::Closed(RuntimeValue::Table(globals)));
    let callee = RuntimeValue::UserFunc(h.alloc(closure));

    let results = match sv.protected_call(h, api, callee, &[])? {
        Ok(values) => values,
        Err(kind) => return Err(DukaRuntimeError::ModuleError(kind.to_string())),
    };
    let val = results.last().cloned().unwrap_or(RuntimeValue::Nil);

    if let Some(cache) = api.module_cache() {
        cache
            .borrow_mut()
            .set(RuntimeValue::from_string(h, name), val.clone());
    }
    Ok(val)
}
