use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use duka_app::binary::{DukaAppBinary, split};
use duka_lib::builtin::require;
use duka_lib::codegen::binary::{DukaBinary, Load};
use duka_lib::duka_gc::Heap;
use duka_lib::module;
use duka_lib::value::RuntimeValue;
use duka_lib::vm::VM;
use duka_lib::DukaVM;

fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(run() as u8)
}

fn run() -> i32 {
    let exe_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("duka-app: cannot locate self: {e}");
            return 2;
        }
    };
    let exe = match std::fs::read(&exe_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("duka-app: cannot read self: {e}");
            return 2;
        }
    };
    let Some((start, len)) = split(&exe) else {
        eprintln!("duka-app: no embedded application");
        eprintln!("usage: build an executable with `dukao build --exe`");
        return 2;
    };
    let app = match DukaAppBinary::load(&mut Cursor::new(&exe[start..start + len])) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("duka-app: bad archive: {e}");
            return 2;
        }
    };
    let entry = app.entry().to_owned();
    let modules: HashMap<String, Vec<u8>> = app.modules().iter().cloned().collect();
    let entry_bytes = match modules.get(&entry) {
        Some(bytes) => bytes,
        None => {
            eprintln!("duka-app: entry '{entry}' not found in archive");
            return 2;
        }
    };
    let proto = match DukaBinary::load(&mut Cursor::new(entry_bytes.as_slice())) {
        Ok(b) => b.into_proto(),
        Err(e) => {
            eprintln!("duka-app: entry binary error: {e}");
            return 2;
        }
    };

    require::reset();
    require::set_loader(module::memory_loader(Arc::new(modules)));

    let mut vm = VM::new(Heap::new());
    vm.set_entry_path(PathBuf::from(&entry));
    let args: Vec<_> = std::env::args()
        .skip(1)
        .map(|a| RuntimeValue::from_string(&mut vm.heap, a))
        .collect();
    vm.set_main_args(&args);
    match vm.execute(&proto) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
