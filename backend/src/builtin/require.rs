use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use duka_gc::Heap;
use duka_macros::duka_builtin;
#[cfg(feature = "json")]
use serde_json::{Map, Value, to_value};

use crate::errors::DukaRuntimeError;
use crate::value::{DukaClosure, DukaProto, RuntimeValue, UpValue};
use crate::vm::coroutine::{CoState, NativeApi, VmGlobals, VmGlobalsHandle};

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

/// Per-VM module state, living in the VM's globals bag.
#[derive(Default)]
pub struct ModuleStore {
    loader: Option<Rc<ModuleLoader>>,
    loading: Rc<RefCell<HashSet<String>>>,
}

impl ModuleStore {
    fn loader(&self) -> Option<Rc<ModuleLoader>> {
        self.loader.clone()
    }
    fn loading(&self) -> Rc<RefCell<HashSet<String>>> {
        self.loading.clone()
    }
    fn clear(&mut self) {
        self.loader = None;
        self.loading.borrow_mut().clear();
    }
    fn set_loader(&mut self, loader: Box<ModuleLoader>) {
        self.loader = Some(Rc::from(loader));
    }
}

pub fn new_vm_globals<F>(loader: F) -> VmGlobalsHandle
where
    F: Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static,
{
    let mut globals = VmGlobals::default();
    let mut store = ModuleStore::default();
    store.set_loader(Box::new(loader));
    globals.put(store);
    Rc::new(RefCell::new(globals))
}

pub fn default_vm_globals() -> VmGlobalsHandle {
    let mut globals = VmGlobals::default();
    globals.put(ModuleStore::default());
    Rc::new(RefCell::new(globals))
}

pub fn reset(globals: &VmGlobalsHandle) {
    if let Some(store) = globals.borrow_mut().get_mut::<ModuleStore>() {
        store.clear();
    }
}

pub fn set_loader<F>(globals: &VmGlobalsHandle, loader: F)
where
    F: Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static,
{
    let mut borrowed = globals.borrow_mut();
    if borrowed.get::<ModuleStore>().is_none() {
        borrowed.put(ModuleStore::default());
    }
    if let Some(store) = borrowed.get_mut::<ModuleStore>() {
        store.set_loader(Box::new(loader));
    }
}

#[duka_builtin(name = "require", doc = "Import module by pattern", params(pattern: string), returns(any), flags(@returns(module), @keywordish()))]
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

    if let Some(cache) = api.module_cache() {
        let key = RuntimeValue::from_string(h, cache_key.clone());
        if let Some(val) = cache.borrow().get(&key).cloned() {
            return Ok(val);
        }
    }

    let handle = api.vm_globals.as_ref().ok_or_else(|| {
        DukaRuntimeError::ModuleError(
            "Module system not configured: this VM has no module store".to_string(),
        )
    })?;
    let (loading, loader) = {
        let globals = handle.borrow();
        let store = globals.get::<ModuleStore>().ok_or_else(|| {
            DukaRuntimeError::ModuleError(
                "Module system not configured: this VM has no module store".to_string(),
            )
        })?;
        (store.loading(), store.loader())
    };

    struct LoadingGuard(Rc<RefCell<HashSet<String>>>, String);
    impl Drop for LoadingGuard {
        fn drop(&mut self) {
            self.0.borrow_mut().remove(&self.1);
        }
    }

    if loading.borrow().contains(&cache_key) {
        return Err(DukaRuntimeError::ModuleError(format!(
            "circular require: {cache_key}"
        )));
    }
    loading.borrow_mut().insert(cache_key.clone());
    let _guard = LoadingGuard(loading, cache_key.clone());

    let loaded = match loader {
        Some(f) => f(&name, caller_dir.as_deref()).map_err(DukaRuntimeError::ModuleError),
        None => Err(DukaRuntimeError::ModuleError(
            "Module system not configured: no loader set (call `set_loader` first)".to_string(),
        )),
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
                        .set(RuntimeValue::from_string(h, cache_key), val);
                }
                return Ok(val);
            }
            // TODO: toml

            // All other resources simply return string (maybe path or content, depend on its type)
            let val = RuntimeValue::from_string(h, String::from_utf8_lossy(&bytes).into_owned());
            if let Some(cache) = api.module_cache() {
                cache
                    .borrow_mut()
                    .set(RuntimeValue::from_string(h, cache_key), val);
            }
            return Ok(val);
        }
        LoadedModule::Executable { proto, path } => {
            let globals_tbl = api.globals().ok_or_else(|| {
                DukaRuntimeError::ModuleError("module system requires a running VM".to_owned())
            })?;
            let closure = DukaClosure::from_proto(h.alloc(proto))
                .set_up_value(h, UpValue::Closed(RuntimeValue::Table(globals_tbl)));
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
                            .set(RuntimeValue::from_string(h, cache_key.clone()), val);
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
                    .map(runtime_to_json)
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
