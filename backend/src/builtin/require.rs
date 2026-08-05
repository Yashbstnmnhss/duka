use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use duka_gc::Heap;
use duka_shared::types::ValueCount;

use crate::errors::DukaRuntimeError;
use crate::value::RuntimeValue;
use crate::vm::coroutine::CoState;

/// Function that loads a module by name and returns its value.
pub type ModuleLoader = dyn Fn(&str) -> Result<RuntimeValue, String> + Send + Sync;

struct ModuleStore {
    cache: UnsafeCell<HashMap<String, RuntimeValue>>,
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
/// module value (usually the module table).
pub fn set_loader<F>(loader: F)
where
    F: Fn(&str) -> Result<RuntimeValue, String> + Send + Sync + 'static,
{
    let s = store();
    unsafe {
        *s.loader.get() = Some(Box::new(loader));
    }
}

/// Pre-populate a module cache entry, skipping the loader.
pub fn set_cache(name: String, value: RuntimeValue) {
    let s = store();
    unsafe {
        (*s.cache.get()).insert(name, value);
    }
}

pub fn impl_require(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let name_val = sv.get_stack(1)?.clone();
    let name = format!("{}", name_val);
    let s = store();

    if let Some(val) = unsafe { (*s.cache.get()).get(&name).cloned() } {
        sv.set_stack(0, val)?;
        return Ok(ValueCount::Exact(1));
    }

    if unsafe { (*s.loading.get()).contains(&name) } {
        return Err(DukaRuntimeError::ModuleError(format!(
            "circular require: {name}" //循环依赖检测
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
    let val = match loader {
        Some(f) => f(&name).map_err(DukaRuntimeError::ModuleError),
        None => Err(DukaRuntimeError::ModuleError(format!(
            "Module system not configured: no loader set (call `set_loader` first)"
        ))),
    }?;
    unsafe {
        (*s.cache.get()).insert(name, val.clone());
    }
    sv.set_stack(0, val)?;
    Ok(ValueCount::Exact(1))
}
