use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use duka_gc::Heap;
use duka_macros::duka_builtin;

use crate::errors::DukaRuntimeError;
use crate::value::{DukaClosure, DukaProto, RuntimeValue, UpValue};
use crate::vm::coroutine::{CoState, NativeApi};

/// Function that loads a module by name and returns its compiled proto.
///
/// The second argument is the directory of the module that called `require()`
/// (from the module-path stack); relative patterns resolve against it.
pub type ModuleLoader = dyn Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync;

/// Result of loading a module: its proto plus the resolved file path.
///
/// `path` anchors nested relative `require()` calls inside the module and is
/// pushed onto the module-path stack while the module executes.
pub struct LoadedModule {
    pub proto: DukaProto,
    pub path: Option<PathBuf>,
}

/// Whether a require pattern is a relative path reference (`./x`, `../x`)
/// Relative path means a file in same kao, (in `/src`), otherwise it is importing a module from `/modules`
pub fn is_relative_name(name: &str) -> bool {
    duka_shared::module::is_relative_name(name)
}

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
/// compiled module proto; `require()` executes it in the caller's VM
///
/// See **duka_lib**
pub fn set_loader<F>(loader: F)
where
    F: Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static,
{
    let s = store();
    unsafe {
        *s.loader.get() = Some(Box::new(loader));
    }
}

#[duka_builtin(name = "require", doc = "Import module by pattern", params(pattern: string), flags(@returns(module)))]
pub fn impl_require(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    pattern: String,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let name = pattern;
    let caller_dir = sv.current_module_dir().map(|p| p.to_path_buf());
    let cache_key = if is_relative_name(&name) {
        match &caller_dir {
            Some(dir) => format!("{}::{}", dir.display(), name),
            None => {
                return Err(DukaRuntimeError::ModuleError(format!(
                    "relative require '{name}' has no module base path"
                )));
            }
        }
    } else {
        name.clone()
    };
    let s = store();

    if let Some(cache) = api.module_cache() {
        let key = RuntimeValue::from_string(h, cache_key.clone());
        if let Some(val) = cache.borrow().get(&key).cloned() {
            return Ok(val);
        }
    }

    if unsafe { (*s.loading.get()).contains(&cache_key) } {
        return Err(DukaRuntimeError::ModuleError(format!(
            "circular require: {cache_key}"
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
        (*s.loading.get()).insert(cache_key.clone());
    }
    let _guard = LoadingGuard(s, cache_key.clone());

    let loader = unsafe { &*s.loader.get() };
    let loaded = match loader {
        Some(f) => f(&name, caller_dir.as_deref()).map_err(DukaRuntimeError::ModuleError),
        None => Err(DukaRuntimeError::ModuleError(format!(
            "Module system not configured: no loader set (call `set_loader` first)"
        ))),
    }?;
    let LoadedModule { proto, path } = loaded;
    unsafe {
        (*s.cache.get()).insert(cache_key.clone(), proto.clone());
    }

    let globals = api.globals().ok_or_else(|| {
        DukaRuntimeError::ModuleError("module system requires a running VM".to_owned())
    })?;
    let closure = DukaClosure::from_proto(h.alloc(proto))
        .set_up_value(h, UpValue::Closed(RuntimeValue::Table(globals)));
    let callee = RuntimeValue::UserFunc(h.alloc(closure));

    let has_path = path.is_some();
    if let Some(p) = path {
        sv.push_module_path(p);
    }
    let call_result = sv.protected_call(h, api, callee, &[]);
    if has_path {
        sv.pop_module_path();
    }
    let results = call_result?;
    match results {
        Ok(values) => {
            let val = values.last().cloned().unwrap_or(RuntimeValue::Nil);
            if let Some(cache) = api.module_cache() {
                cache
                    .borrow_mut()
                    .set(RuntimeValue::from_string(h, cache_key), val.clone());
            }
            Ok(val)
        }
        Err(kind) => Err(DukaRuntimeError::ModuleError(kind.to_string())),
    }
}
