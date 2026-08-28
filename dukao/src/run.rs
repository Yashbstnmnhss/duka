use std::path::PathBuf;

use colored::Colorize;
use duka_lib::duka_gc::Heap;
use duka_lib::duka_shared::errors::DukaSpannedError;
use duka_lib::kao::find_kao;
use duka_lib::module;
use duka_lib::value::RuntimeValue;
use duka_lib::vm::VM;
use duka_lib::{DukaVM, builtin};

use crate::diag::{render_compile_error, render_runtime_error};

pub fn run_run_cmd(path: PathBuf, entry: Option<String>, script_args: Vec<String>) -> i32 {
    let kao = match find_kao(&path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
    };
    let root = kao.root().to_path_buf();

    let entry_rel = entry.map(PathBuf::from).unwrap_or_else(|| kao.entry());
    let entry_path = root.join(entry_rel);
    if !entry_path.is_file() {
        eprintln!(
            "{}: entry not found: {}",
            "error".red().bold(),
            entry_path.display()
        );
        return 2;
    }

    let config = kao
        .manifest()
        .and_then(|i| i.build.config.clone())
        .unwrap_or_default();

    let paths = module::search_paths(&root, "modules");
    builtin::require::set_loader(module::file_loader(paths, config.clone()));

    let proto = match module::load_proto(&entry_path, config) {
        Ok(p) => p,
        Err(e) => {
            let detail = match e.downcast::<DukaSpannedError>() {
                Ok(spanned) => render_compile_error(&entry_path, *spanned),
                Err(e) => e.to_string(),
            };
            eprintln!("{}: {}", "error".red().bold(), detail.trim_end());
            return 1;
        }
    };

    let mut vm = VM::new(Heap::new());
    vm.set_entry_path(entry_path.clone());
    let args: Vec<_> = script_args
        .into_iter()
        .map(|a| RuntimeValue::from_string(&mut vm.heap, a))
        .collect();
    vm.set_main_args(&args);
    match vm.execute(&proto) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{}", render_runtime_error(&e));
            1
        }
    }
}
