use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, LazyLock, Mutex},
};

use duka_backend::{
    DukaVM,
    builtin::require,
    codegen::binary::{DukaBinary, FORMAT_VERSION, Load},
    vm::{VM, coroutine::{InputCell, OutputCell}},
};
use duka_gc::Heap;

const SUCCESS: i32 = 0;
const BINARY_FAILURE: i32 = 1;
const RUNTIME_FAILURE: i32 = 2;
const NULLPTR_FAILURE: i32 = 3;

static INPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static SCRIPT_INPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static MODULES: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Allocate memory for input (proto)
#[unsafe(no_mangle)]
pub extern "C" fn duka_alloc(len: u32) -> *mut u8 {
    let mut i = INPUT.lock().expect("Failed to alloc memory");
    *i = vec![0u8; len as usize];
    i.as_mut_ptr()
}
/// Free the input memory
#[unsafe(no_mangle)]
pub extern "C" fn duka_free() {
    *INPUT.lock().expect("Failed to free memory") = vec![];
}

fn write_buffer(data: Vec<u8>) {
    let mut b = OUTPUT.lock().expect("Failed to write buffer");
    *b = data;
}

fn output(data: String) {
    write_buffer(data.into_bytes());
}
fn failed(msg: &str) {
    write_buffer(msg.to_owned().into_bytes());
}

/// Get a pointer pointing to result
#[unsafe(no_mangle)]
pub extern "C" fn duka_result_ptr() -> *const u8 {
    OUTPUT.lock().as_ref().unwrap().as_ptr()
}
/// Get the length of result
#[unsafe(no_mangle)]
pub extern "C" fn duka_result_len() -> u32 {
    OUTPUT.lock().unwrap().len() as u32
}

/// Duka binary format version
#[unsafe(no_mangle)]
pub extern "C" fn duka_version() -> u32 {
    FORMAT_VERSION as u32
}

/// Register a pre-compiled module (`*.dukac` binary) under `name` for `require()`.
///
/// The bytes are copied, so the caller may free them afterwards.
#[unsafe(no_mangle)]
pub extern "C" fn duka_add_module(
    name_ptr: *const u8,
    name_len: u32,
    data_ptr: *const u8,
    data_len: u32,
) -> i32 {
    if name_ptr.is_null() || data_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    let name_bytes = unsafe { std::slice::from_raw_parts(name_ptr, name_len as usize) };
    let name = String::from_utf8_lossy(name_bytes).into_owned();
    let bytes = unsafe { std::slice::from_raw_parts(data_ptr, data_len as usize) }.to_vec();
    MODULES
        .lock()
        .expect("Failed to modify modules")
        .insert(name, bytes);
    SUCCESS
}

/// Clear all registered modules.
#[unsafe(no_mangle)]
pub extern "C" fn duka_clear_modules() {
    MODULES.lock().expect("Failed to clear modules").clear();
}

/// Set the script's standard input bytes, consumed by `io.stdin`.
#[unsafe(no_mangle)]
pub extern "C" fn duka_set_input(data_ptr: *const u8, data_len: u32) -> i32 {
    if data_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data_ptr, data_len as usize) }.to_vec();
    *SCRIPT_INPUT.lock().expect("Failed to set input") = bytes;
    SUCCESS
}

/// Clear the script's standard input.
#[unsafe(no_mangle)]
pub extern "C" fn duka_clear_input() {
    *SCRIPT_INPUT.lock().expect("Failed to clear input") = vec![];
}

fn install_module_loader() {
    use duka_backend::builtin::require::{LoadedModule, set_loader};
    set_loader(|name, _caller_dir| {
        let bytes = MODULES
            .lock()
            .expect("module registry lock poisoned")
            .get(name)
            .cloned()
            .ok_or_else(|| format!("module '{name}' not registered"))?;
        let proto = DukaBinary::load(&mut Cursor::new(bytes.as_slice()))
            .map(|b| b.into_proto())
            .map_err(|e| format!("module '{name}' binary error: {e}"))?;
        Ok(LoadedModule { proto, path: None })
    });
}

/// Run duka binary with given pointer and length
#[unsafe(no_mangle)]
pub extern "C" fn duka_run(data: *const u8, len: u32) -> i32 {
    if data.is_null() {
        return NULLPTR_FAILURE;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let proto = match DukaBinary::load(&mut Cursor::new(slice)) {
        Ok(k) => k.into_proto(),
        Err(e) => {
            failed(&format!("DukaBinary error: {e}"));
            return BINARY_FAILURE;
        }
    };
    require::reset();
    install_module_loader();
    let mut vm = VM::new(Heap::new());
    let stdout: OutputCell = Arc::new(Mutex::new(vec![]));
    let stderr: OutputCell = Arc::new(Mutex::new(vec![]));
    vm.set_stdout(Some(stdout.clone()));
    vm.set_stderr(Some(stderr.clone()));
    let stdin: InputCell = Arc::new(Mutex::new(SCRIPT_INPUT.lock().expect("Failed to read input").clone()));
    vm.set_input(Some(stdin));

    let vc = match vm.execute(&proto) {
        Ok(v) => v,
        Err(e) => {
            failed(&format!("DukaVM error: {e}"));
            return RUNTIME_FAILURE;
        }
    };

    let result = match vm.scheduler.main_mut().inner.take_stack_many(0, vc) {
        Ok(v) => v,
        Err(e) => {
            failed(&format!("DukaVM error: {e}"));
            return RUNTIME_FAILURE;
        }
    }
    .iter()
    .map(|v| v.to_string())
    .collect::<Vec<_>>()
    .join(" ");
    let stdout = String::from_utf8_lossy(&*stdout.lock().unwrap()).to_string();
    let stderr = String::from_utf8_lossy(&*stderr.lock().unwrap()).to_string();

    let json = serde_json::json!(
        {
            "result": result,
            "stdout": stdout,
            "stderr": stderr,
        }
    );
    output(json.to_string());
    SUCCESS
}
