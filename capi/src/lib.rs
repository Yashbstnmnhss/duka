#![allow(non_snake_case)]

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use duka_gc::Heap;
use duka_lib::builtin::require::LoadedModule;
use duka_lib::value::{RuntimeDukaTable, RuntimeValue, RustClosure};
use duka_shared::types::ValueCount;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DukaType {
    Nil = 0,
    Bool = 1,
    Int = 2,
    Number = 3,
    String = 4,
    Table = 5,
    Array = 6,
    Function = 7,
    Userdata = 8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DukaStr {
    pub data: *const c_char,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union DukaValueData {
    pub boolean: c_int,
    pub integer: i64,
    pub number: f64,
    pub string: DukaStr,
    pub handle: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DukaValue {
    pub value_type: DukaType,
    pub as_: DukaValueData,
}

pub type DukaCFunc = extern "C" fn(*mut DukaCall) -> c_int;
pub type DukaOpenFunc = extern "C" fn(*mut DukaModule) -> c_int;

struct Export {
    name: String,
    func: DukaCFunc,
}

pub struct DukaModule {
    exports: Vec<Export>,
    error: Option<String>,
}

pub struct DukaCall {
    args: Box<[RuntimeValue]>,
    returns: Vec<RuntimeValue>,
    error: Option<String>,
    heap: *mut Heap,
}

struct Plugin {
    _library: libloading::Library,
}

pub struct CPluginLoader {
    paths: HashMap<String, PathBuf>,
    functions: HashMap<String, DukaOpenFunc>,
    loaded: Mutex<Vec<Plugin>>,
}

impl CPluginLoader {
    pub fn new() -> Self {
        Self {
            paths: HashMap::new(),
            functions: HashMap::new(),
            loaded: Mutex::new(Vec::new()),
        }
    }

    pub fn add(mut self, name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        self.paths.insert(name.into(), path.into());
        self
    }

    pub fn add_function(mut self, name: impl Into<String>, func: DukaOpenFunc) -> Self {
        self.functions.insert(name.into(), func);
        self
    }

    pub fn loader(
        self,
    ) -> impl Fn(&str, Option<&Path>, &mut Heap) -> Result<LoadedModule, String> + Send + Sync {
        move |name: &str, _caller: Option<&Path>, heap: &mut Heap| {
            let mut module = DukaModule {
                exports: Vec::new(),
                error: None,
            };
            let mut library = None;
            let code = if let Some(func) = self.functions.get(name).copied() {
                func(&mut module as *mut _)
            } else {
                let path = self
                    .paths
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("no C plugin registered as '{name}'"))?;
                let symbol = format!("dukaopen_{name}");
                let loaded = unsafe { libloading::Library::new(&path) }
                    .map_err(|e| format!("cannot load '{}': {e}", path.display()))?;
                let entry: libloading::Symbol<DukaOpenFunc> = unsafe {
                    loaded.get(symbol.as_bytes())
                }
                .map_err(|e| format!("cannot find '{symbol}' in '{}': {e}", path.display()))?;
                let code = entry(&mut module as *mut _);
                library = Some(loaded);
                code
            };
            if code != 0 {
                return Err(format!("plugin '{name}' returned {code}"));
            }
            if let Some(error) = &module.error {
                return Err(format!("plugin '{name}': {error}"));
            }
            let mut table = RuntimeDukaTable::new(0);
            for export in module.exports {
                let closure = make_closure(export.func);
                let value = RuntimeValue::from_rust_closure(heap, closure);
                table.set_by_key(heap, export.name.into(), value);
            }
            let value = RuntimeValue::Table(heap.alloc(duka_gc::GcCell::new(table)));
            if let Some(library) = library {
                self.loaded
                    .lock()
                    .unwrap()
                    .push(Plugin { _library: library });
            }
            Ok(LoadedModule::Value(value))
        }
    }
}

fn make_closure(func: DukaCFunc) -> RustClosure {
    RustClosure::returns(
        move |co, heap, _api| {
            let args = co.take_stack_many(1, ValueCount::VarArg)?;
            let mut call = DukaCall {
                args,
                returns: Vec::new(),
                error: None,
                heap: heap as *mut _,
            };
            let result = catch_unwind(AssertUnwindSafe(|| func(&mut call as *mut _)));
            match result {
                Ok(count) => {
                    if let Some(error) = call.error {
                        return Err(duka_lib::errors::DukaRuntimeError::Custom(error));
                    }
                    if count < 0 {
                        return Err(duka_lib::errors::DukaRuntimeError::Custom(
                            "export failed".to_owned(),
                        ));
                    }
                    for (i, value) in call.returns.into_iter().take(count as usize).enumerate() {
                        co.set_stack(i, value)?;
                    }
                    Ok(ValueCount::Exact(count as usize))
                }
                Err(_) => Err(duka_lib::errors::DukaRuntimeError::Custom(
                    "export panicked".to_owned(),
                )),
            }
        },
        None,
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_export(m: *mut DukaModule, name: *const c_char, func: Option<DukaCFunc>) {
    if m.is_null() || name.is_null() || func.is_none() {
        return;
    }
    let name = unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    unsafe {
        (*m).exports.push(Export {
            name,
            func: func.unwrap(),
        })
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_arg_count(call: *mut DukaCall) -> c_int {
    if call.is_null() {
        return 0;
    }
    let call = unsafe { &*call };
    call.args.len() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_arg(call: *mut DukaCall, index: c_int) -> DukaValue {
    let nil = DukaValue {
        value_type: DukaType::Nil,
        as_: DukaValueData { integer: 0 },
    };
    if call.is_null() || index < 0 {
        return nil;
    }
    let call = unsafe { &*call };
    let Some(value) = call.args.get(index as usize) else {
        return nil;
    };
    match value {
        RuntimeValue::Nil => nil,
        RuntimeValue::Bool(b) => DukaValue {
            value_type: DukaType::Bool,
            as_: DukaValueData {
                boolean: *b as c_int,
            },
        },
        RuntimeValue::Int(i) => DukaValue {
            value_type: DukaType::Int,
            as_: DukaValueData { integer: *i },
        },
        RuntimeValue::Float(f) => DukaValue {
            value_type: DukaType::Number,
            as_: DukaValueData { number: *f },
        },
        _ => nil,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_ret(call: *mut DukaCall, value: DukaValue) {
    if call.is_null() {
        return;
    }
    let call = unsafe { &mut *call };
    let value = match value.value_type {
        DukaType::Nil => Some(RuntimeValue::Nil),
        DukaType::Bool => Some(RuntimeValue::Bool(unsafe { value.as_.boolean } != 0)),
        DukaType::Int => Some(RuntimeValue::Int(unsafe { value.as_.integer })),
        DukaType::Number => Some(RuntimeValue::Float(unsafe { value.as_.number })),
        DukaType::String => {
            let s = unsafe { value.as_.string };
            if s.data.is_null() {
                None
            } else {
                let bytes = unsafe { std::slice::from_raw_parts(s.data as *const u8, s.len) };
                std::str::from_utf8(bytes).ok().map(|text| {
                    let heap = unsafe { &mut *call.heap };
                    RuntimeValue::from_string(heap, text.to_owned())
                })
            }
        }
        _ => None,
    };
    if let Some(value) = value {
        call.returns.push(value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_ret_int(call: *mut DukaCall, value: i64) {
    duka_ret(
        call,
        DukaValue {
            value_type: DukaType::Int,
            as_: DukaValueData { integer: value },
        },
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_ret_number(call: *mut DukaCall, value: f64) {
    duka_ret(
        call,
        DukaValue {
            value_type: DukaType::Number,
            as_: DukaValueData { number: value },
        },
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_ret_bool(call: *mut DukaCall, value: c_int) {
    duka_ret(
        call,
        DukaValue {
            value_type: DukaType::Bool,
            as_: DukaValueData { boolean: value },
        },
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_ret_string(call: *mut DukaCall, data: *const c_char, len: usize) {
    if call.is_null() || data.is_null() {
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, len) };
    if let Ok(text) = std::str::from_utf8(bytes) {
        let call = unsafe { &mut *call };
        let heap = unsafe { &mut *call.heap };
        let value = RuntimeValue::from_string(heap, text.to_owned());
        call.returns.push(value);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_error(call: *mut DukaCall, message: *const c_char) {
    if call.is_null() || message.is_null() {
        return;
    }
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    unsafe { (*call).error = Some(message) };
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_value_int(value: i64) -> DukaValue {
    DukaValue {
        value_type: DukaType::Int,
        as_: DukaValueData { integer: value },
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_value_number(value: f64) -> DukaValue {
    DukaValue {
        value_type: DukaType::Number,
        as_: DukaValueData { number: value },
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_value_bool(value: c_int) -> DukaValue {
    DukaValue {
        value_type: DukaType::Bool,
        as_: DukaValueData { boolean: value },
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_value_string(data: *const c_char, len: usize) -> DukaValue {
    DukaValue {
        value_type: DukaType::String,
        as_: DukaValueData {
            string: DukaStr { data, len },
        },
    }
}
