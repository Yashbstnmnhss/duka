use std::cell::UnsafeCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use duka_gc::Heap;
use duka_macros::duka_builtin;
#[cfg(feature = "json")]
use serde_json::{Map, Value, to_value};

use crate::errors::DukaRuntimeError;
use crate::value::{DukaClosure, DukaProto, RuntimeValue, UpValue};
use crate::vm::coroutine::{CoState, NativeApi};

/// Function that loads a module by name and returns its compiled proto.
///
/// The second argument is the directory of the module that called `require()`
/// (from the module-path stack); relative patterns resolve against it.
pub type ModuleLoader = dyn Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync;

/// Result of loading a module.
pub enum LoadedModule {
    /// Compiled Duka bytecode, ready to execute
    Executable {
        proto: DukaProto,
        path: Option<PathBuf>,
    },
    /// Raw resource bytes with file extension (e.g. "html", "css", "txt")
    /// The extension tells require() how to interpret the bytes.
    Resource { bytes: Vec<u8>, ext: Box<str> },
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
    let (_, val) = match loaded {
        LoadedModule::Resource { bytes, ext } => {
            #[cfg(feature = "json")] // JSON special
            if ext.as_ref() == "json" {
                let parsed: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| DukaRuntimeError::ModuleError(format!("JSON parse error: {e}")))?;
                let val = json_to_runtime(h, &parsed)?;
                if let Some(cache) = api.module_cache() {
                    cache
                        .borrow_mut()
                        .set(RuntimeValue::from_string(h, cache_key), val.clone());
                }
                return Ok(val);
            }
            // TODO: toml

            // All other resources simply return string (maybe path or content, depend on its type)
            let val = RuntimeValue::from_string(h, String::from_utf8_lossy(&bytes).into_owned());
            if let Some(cache) = api.module_cache() {
                cache
                    .borrow_mut()
                    .set(RuntimeValue::from_string(h, cache_key), val.clone());
            }
            return Ok(val);
        }
        LoadedModule::Executable { proto, path } => {
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
                            .set(RuntimeValue::from_string(h, cache_key.clone()), val.clone());
                    }
                    (cache_key, val)
                }
                Err(kind) => return Err(DukaRuntimeError::ModuleError(kind.to_string())),
            }
        }
    };
    Ok(val)
}

#[cfg(feature = "json")]
pub fn runtime_to_json(val: &RuntimeValue) -> Result<Value, DukaRuntimeError> {
    Ok(match val {
        RuntimeValue::Nil => Value::Null,
        RuntimeValue::Int(i) => Value::Number((*i).into()),
        RuntimeValue::Float(f) => {
            to_value(f).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?
        }
        RuntimeValue::Bool(b) => Value::Bool(*b),
        rv if rv.is_string() => Value::String(rv.eval_to_string().to_string()),
        RuntimeValue::Table(gc) => {
            let tab = &gc.borrow().inner;
            let mut map = Map::with_capacity(tab.len());
            for (k, v) in tab {
                let kv = runtime_to_json(k)?.to_string();
                let vv = runtime_to_json(v)?;
                map.insert(kv, vv);
            }
            Value::Object(map)
        }
        RuntimeValue::Array(gc) => {
            let arr = &gc.borrow().items;
            Value::Array(
                arr.iter()
                    .map(|v| runtime_to_json(v))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }

        _ => return Err(DukaRuntimeError::InvalidValueType("serializable")),
    })
}

#[cfg(feature = "json")]
pub fn json_to_runtime(
    h: &mut Heap,
    val: &serde_json::Value,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(match val {
        Value::Null => RuntimeValue::Nil,
        Value::Bool(b) => RuntimeValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RuntimeValue::Int(i)
            } else {
                RuntimeValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => RuntimeValue::from_string(h, s.clone()),
        Value::Array(arr) => {
            let mut table = crate::value::RuntimeDukaTable::new(arr.len());
            for (i, item) in arr.iter().enumerate() {
                table.array_set(i, json_to_runtime(h, item)?);
            }
            RuntimeValue::Table(h.alloc(duka_gc::GcCell::new(table)))
        }
        Value::Object(map) => {
            let mut table = crate::value::RuntimeDukaTable::new(map.len());
            for (k, v) in map {
                table.set(
                    RuntimeValue::from_string(h, k.clone()),
                    json_to_runtime(h, v)?,
                );
            }
            RuntimeValue::Table(h.alloc(duka_gc::GcCell::new(table)))
        }
    })
}
