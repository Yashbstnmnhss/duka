use std::{
    cell::RefCell,
    collections::HashMap,
    io::Cursor,
    str,
    sync::{Arc, LazyLock, Mutex},
};

use duka_lib::duka_gc::{GcCell, Heap};
use duka_lib::duka_shared::types::ValueCount;
use duka_lib::{
    DukaVM,
    builtin::require,
    codegen::binary::{DukaBinary, FORMAT_VERSION, Load},
    value::{RuntimeValue, RustClosure},
    vm::{
        VM,
        coroutine::{InputCell, OutputCell},
    },
};

const SUCCESS: i32 = 0;
const BINARY_FAILURE: i32 = 1;
const RUNTIME_FAILURE: i32 = 2;
const NULLPTR_FAILURE: i32 = 3;

static INPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static COMMAND_BUFFER: Mutex<Vec<String>> = Mutex::new(vec![]);
static SCRIPT_INPUT: Mutex<Vec<u8>> = Mutex::new(vec![]);
static SCRIPT_ARGS: Mutex<Vec<String>> = Mutex::new(vec![]);
static MODULES: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SCRIPT_ENTRY: Mutex<Vec<u8>> = Mutex::new(vec![]);

thread_local! {
    static PERSISTENT_VM: RefCell<Option<VM>> = RefCell::new(None);
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_alloc(len: u32) -> *mut u8 {
    let mut i = INPUT.lock().expect("alloc");
    *i = vec![0u8; len as usize];
    i.as_mut_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_free() {
    *INPUT.lock().expect("free") = vec![];
}

fn write_buffer(data: Vec<u8>) {
    *OUTPUT.lock().expect("write_buffer") = data;
}

fn output(data: String) {
    write_buffer(data.into_bytes());
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_result_ptr() -> *const u8 {
    OUTPUT.lock().expect("result_ptr").as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_result_len() -> u32 {
    OUTPUT.lock().expect("result_len").len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_version() -> u32 {
    FORMAT_VERSION as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_add_module(
    name_ptr: *const u8,
    name_len: u32,
    data_ptr: *const u8,
    data_len: u32,
) -> i32 {
    if name_ptr.is_null() || data_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    unsafe {
        let name = String::from_utf8_lossy(std::slice::from_raw_parts(name_ptr, name_len as usize))
            .into_owned();
        let bytes = std::slice::from_raw_parts(data_ptr, data_len as usize).to_vec();
        MODULES.lock().expect("add_module").insert(name, bytes);
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_clear_modules() {
    MODULES.lock().expect("clear_modules").clear();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_set_input(data_ptr: *const u8, data_len: u32) -> i32 {
    if data_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(data_ptr, data_len as usize).to_vec();
        *SCRIPT_INPUT.lock().expect("set_input") = bytes;
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_clear_input() {
    *SCRIPT_INPUT.lock().expect("clear_input") = vec![];
}

fn install_module_loader() {
    let modules: HashMap<String, Vec<u8>> = MODULES.lock().expect("install_loader").clone();
    duka_lib::builtin::require::set_loader(duka_lib::module::memory_loader(
        Arc::new(modules),
        "modules",
    ));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_set_entry(name_ptr: *const u8, name_len: u32) -> i32 {
    if name_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(name_ptr, name_len as usize).to_vec();
        *SCRIPT_ENTRY.lock().expect("set_entry") = bytes;
    }
    SUCCESS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_set_args(json_ptr: *const u8, json_len: u32) -> i32 {
    if json_ptr.is_null() {
        return NULLPTR_FAILURE;
    }
    unsafe {
        let bytes = std::slice::from_raw_parts(json_ptr, json_len as usize);
        let args: Vec<String> = serde_json::from_slice(bytes).unwrap_or_default();
        *SCRIPT_ARGS.lock().expect("set_args") = args;
    }
    SUCCESS
}

fn value_to_json(val: &RuntimeValue, heap: &Heap) -> serde_json::Value {
    match val {
        RuntimeValue::Nil => serde_json::Value::Null,
        RuntimeValue::Int(v) => serde_json::json!(*v),
        RuntimeValue::Float(v) => serde_json::json!(*v),
        RuntimeValue::Bool(b) => serde_json::json!(*b),
        RuntimeValue::ShortString(len, buf) => {
            let s = str::from_utf8(&buf[..(*len as usize)]).unwrap_or("");
            serde_json::Value::String(s.to_owned())
        }
        RuntimeValue::MediumString(inner) => {
            let s = str::from_utf8(&inner.1[..(inner.0 as usize)]).unwrap_or("");
            serde_json::Value::String(s.to_owned())
        }
        RuntimeValue::LongString(inner) => {
            serde_json::Value::String(inner.0.clone())
        }
        RuntimeValue::Array(arr) => {
            let arr = arr.borrow();
            let items: Vec<serde_json::Value> = arr
                .items
                .iter()
                .map(|v| value_to_json(v, heap))
                .collect();
            serde_json::Value::Array(items)
        }
        RuntimeValue::Table(tab) => {
            let tab = tab.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in tab.inner.iter() {
                let key = match k {
                    RuntimeValue::Int(i) => i.to_string(),
                    RuntimeValue::Float(f) => f.to_string(),
                    RuntimeValue::Bool(b) => b.to_string(),
                    RuntimeValue::ShortString(len, buf) => {
                        str::from_utf8(&buf[..(*len as usize)]).unwrap_or("").to_owned()
                    }
                    RuntimeValue::MediumString(inner) => {
                        str::from_utf8(&inner.1[..(inner.0 as usize)])
                            .unwrap_or("")
                            .to_owned()
                    }
                    RuntimeValue::LongString(inner) => inner.0.clone(),
                    _ => continue,
                };
                map.insert(key, value_to_json(v, heap));
            }
            serde_json::Value::Object(map)
        }
        _ => serde_json::Value::String(val.to_string()),
    }
}

fn register_web_builtins(vm: &mut VM) {
    let push_patch = RustClosure::returning::<0, _>(|sv, heap, _api| {
        let val = sv.get_stack(1)?;
        let json_val = value_to_json(&val, heap);
        let json_str = serde_json::to_string(&json_val).unwrap_or_default();
        COMMAND_BUFFER.lock().expect("push_patch").push(json_str);
        Ok(())
    });
    let gc = vm.heap.alloc(GcCell::new(push_patch));
    vm.set_global("__push_patch", RuntimeValue::NativeFunc(gc));

    // __inject_css(content) - inject CSS into <head>
    let inject_css = RustClosure::returning::<0, _>(|sv, heap, _api| {
        let content = sv.get_stack(1)?;
        let json = serde_json::json!({
            "op": "inject_css",
            "content": value_to_json(&content, heap),
        });
        let json_str = serde_json::to_string(&json).unwrap_or_default();
        COMMAND_BUFFER.lock().expect("inject_css").push(json_str);
        Ok(())
    });
    let gc = vm.heap.alloc(GcCell::new(inject_css));
    vm.set_global("__inject_css", RuntimeValue::NativeFunc(gc));

    // __inject_html(selector, content) - inject HTML into element
    let inject_html = RustClosure::returning::<0, _>(|sv, heap, _api| {
        let selector = sv.get_stack(1)?;
        let content = sv.get_stack(2)?;
        let json = serde_json::json!({
            "op": "inject_html",
            "selector": value_to_json(&selector, heap),
            "content": value_to_json(&content, heap),
        });
        let json_str = serde_json::to_string(&json).unwrap_or_default();
        COMMAND_BUFFER.lock().expect("inject_html").push(json_str);
        Ok(())
    });
    let gc = vm.heap.alloc(GcCell::new(inject_html));
    vm.set_global("__inject_html", RuntimeValue::NativeFunc(gc));
}

fn init_vm() -> VM {
    let mut vm = VM::new(Heap::new());
    let entry = SCRIPT_ENTRY.lock().expect("init_vm entry").clone();
    if !entry.is_empty() {
        vm.set_entry_path(std::path::PathBuf::from(
            String::from_utf8_lossy(&entry).into_owned(),
        ));
    }
    let args: Vec<_> = SCRIPT_ARGS
        .lock()
        .expect("init_vm args")
        .iter()
        .map(|a| RuntimeValue::from_string(&mut vm.heap, a.clone()))
        .collect();
    vm.set_main_args(&args);
    let stdout: OutputCell = Arc::new(Mutex::new(vec![]));
    let stderr: OutputCell = Arc::new(Mutex::new(vec![]));
    vm.set_stdout(Some(stdout));
    vm.set_stderr(Some(stderr));
    let stdin: InputCell = Arc::new(Mutex::new(
        SCRIPT_INPUT.lock().expect("init_vm stdin").clone(),
    ));
    vm.set_input(Some(stdin));
    register_web_builtins(&mut vm);
    vm
}

fn take_result(vm: &mut VM) -> String {
    match vm
        .scheduler
        .main_mut()
        .inner
        .take_stack_many(0, ValueCount::VarArg)
    {
        Ok(vals) => vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(" "),
        Err(e) => format!("error: {e}"),
    }
}

fn ok_response(result: &str, patches: &str) -> String {
    let patches_arr: serde_json::Value =
        serde_json::from_str(patches).unwrap_or(serde_json::json!([]));
    serde_json::json!({
        "ok": true,
        "result": result,
        "patches": patches_arr,
    })
    .to_string()
}

fn err_response(msg: &str) -> String {
    serde_json::json!({
        "ok": false,
        "error": msg,
        "patches": "",
    })
    .to_string()
}

fn take_commands() -> String {
    let mut buf = COMMAND_BUFFER.lock().expect("take_commands");
    let result = format!("[{}]", buf.join(","));
    buf.clear();
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_run(data: *const u8, len: u32) -> i32 {
    if data.is_null() {
        return NULLPTR_FAILURE;
    }

    duka_reset();

    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let proto = match DukaBinary::load(&mut Cursor::new(slice)) {
        Ok(k) => k.into_proto(),
        Err(e) => {
            output(err_response(&format!("DukaBinary error: {e}")));
            return BINARY_FAILURE;
        }
    };

    require::reset();
    install_module_loader();

    let mut vm = init_vm();
    match vm.execute(&proto) {
        Ok(_) => {}
        Err(e) => {
            output(err_response(&format!("DukaVM error: {e}")));
            return RUNTIME_FAILURE;
        }
    };

    let result_str = take_result(&mut vm);
    let patches = take_commands();

    PERSISTENT_VM.with(|cell| {
        *cell.borrow_mut() = Some(vm);
    });

    output(ok_response(&result_str, &patches));
    SUCCESS
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_resume() -> i32 {
    PERSISTENT_VM.with(|cell| {
        let mut guard = cell.borrow_mut();
        let vm = match guard.as_mut() {
            Some(v) => v,
            None => {
                output(err_response("VM not initialized, call duka_run first"));
                return RUNTIME_FAILURE;
            }
        };

        match vm.scheduler.go(&mut vm.heap) {
            Ok(_vc) => {}
            Err(e) => {
                output(err_response(&format!("DukaVM error: {e}")));
                return RUNTIME_FAILURE;
            }
        }

        let result_str = take_result(vm);
        let patches = take_commands();

        output(ok_response(&result_str, &patches));
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duka_handle_event(
    evt_id_ptr: *const u8,
    evt_id_len: u32,
    data_ptr: *const u8,
    data_len: u32,
) -> i32 {
    if evt_id_ptr.is_null() || data_ptr.is_null() {
        return NULLPTR_FAILURE;
    }

    let evt_id = unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(evt_id_ptr, evt_id_len as usize))
            .into_owned()
    };
    let data = unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(data_ptr, data_len as usize))
            .into_owned()
    };

    PERSISTENT_VM.with(|cell| {
        let mut guard = cell.borrow_mut();
        let vm = match guard.as_mut() {
            Some(v) => v,
            None => {
                output(err_response("VM not initialized"));
                return RUNTIME_FAILURE;
            }
        };

        let event_json = serde_json::json!({
            "id": evt_id,
            "data": data,
        });
        let event_val = RuntimeValue::from_string(&mut vm.heap, event_json.to_string());
        vm.set_global("__current_event", event_val);

        match vm.scheduler.go(&mut vm.heap) {
            Ok(_vc) => {}
            Err(e) => {
                output(err_response(&format!("DukaVM error: {e}")));
                return RUNTIME_FAILURE;
            }
        }

        let result_str = take_result(vm);
        let patches = take_commands();

        output(ok_response(&result_str, &patches));
        SUCCESS
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn duka_reset() {
    PERSISTENT_VM.with(|cell| {
        *cell.borrow_mut() = None;
    });
    SCRIPT_ENTRY.lock().expect("reset entry").clear();
    SCRIPT_ARGS.lock().expect("reset args").clear();
    SCRIPT_INPUT.lock().expect("reset input").clear();
    COMMAND_BUFFER.lock().expect("reset commands").clear();
}
