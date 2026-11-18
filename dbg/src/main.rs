mod diag;
mod helper;
mod render;
mod session;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use clap::Parser;
use duka_lib::builtin::require::new_vm_globals;
use duka_lib::duka_gc::Heap;
use duka_lib::duka_shared::types::SourceName;
use duka_lib::harness::DukaError;
use duka_lib::module::{compile_file, file_loader, search_paths};
use duka_lib::value::RuntimeValue;
use duka_lib::vm::coroutine::DebuggerCell;
use duka_lib::vm::debugger::Debugger;
use duka_lib::vm::{Outcome, VM};
use miette::{IntoDiagnostic, MietteHandlerOpts, Report, Result, miette};
use rustyline::{Editor, history::FileHistory};
use syntect::highlighting::ThemeSet;

use crate::helper::DbgHelper;
use crate::session::{Flow, Session};

#[derive(Parser)]
#[command(name = "duka-dbg", version, about = "Interactive debugger for duka")]
struct Args {
    file: PathBuf,
    #[arg(short = 'b', long = "break", value_name = "LINE")]
    breaks: Vec<u32>,
    #[arg(long)]
    no_break_on_error: bool,
    #[arg(last = true, allow_hyphen_values = true, value_name = "SCRIPT_ARGS")]
    script_args: Vec<String>,
}

fn main() -> Result<()> {
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .name("duka-vm".into())
        .spawn(real_main)
        .into_diagnostic()?;
    match handle.join() {
        Ok(inner) => inner,
        Err(_) => Err(miette!("duka vm worker thread panicked")),
    }
}

fn real_main() -> Result<()> {
    miette::set_hook(Box::new(|_| {
        let set = render::syntax_set();
        let theme = ThemeSet::load_defaults().themes["base16-ocean.dark"].clone();
        let highlighter = miette::highlighters::SyntectHighlighter::new(set, theme, false);
        Box::new(
            MietteHandlerOpts::new()
                .with_syntax_highlighting(highlighter)
                .build(),
        )
    }))?;
    run(Args::parse())
}

fn run(args: Args) -> Result<()> {
    let path = args.file;
    if !path.is_file() {
        return Err(diag::report_plain(format!(
            "no such file: {}",
            path.display()
        )));
    }

    let (module_root, modules_dir) = module_root(&path);
    let paths = search_paths(&module_root, &modules_dir);
    let vm_state = new_vm_globals(file_loader(paths, Default::default()));

    let proto = match compile_file(&path, Default::default()) {
        Ok(proto) => proto,
        Err(err) => return Err(compile_error(err)),
    };

    let code = std::fs::read_to_string(&path).into_diagnostic()?;
    let entry = SourceName::File(
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        path.clone().into(),
    );

    let mut vm = VM::with_vm_globals(Heap::new(), vm_state);
    vm.set_entry_path(path.clone());
    let script_args: Vec<_> = args
        .script_args
        .iter()
        .map(|arg| RuntimeValue::from_string(&mut vm.heap, arg.clone()))
        .collect();
    vm.set_main_args(&script_args);

    let debugger: DebuggerCell = Rc::new(RefCell::new(Debugger::default()));
    debugger
        .borrow_mut()
        .break_on_error(!args.no_break_on_error);
    for line in args.breaks {
        debugger.borrow_mut().add_breakpoint(line, entry.clone());
    }
    vm.scheduler.with_debugger(Some(debugger.clone()));

    let mut rl: Editor<DbgHelper, FileHistory> = Editor::new().into_diagnostic()?;
    rl.set_helper(Some(DbgHelper));

    let mut session = Session::new(vm, debugger, entry, code);

    let mut first = true;
    loop {
        let outcome = if first {
            first = false;
            session.vm_mut().execute_debug(&proto)
        } else {
            let vm = session.vm_mut();
            vm.scheduler.run(&mut vm.heap)
        };
        match outcome {
            Ok(Outcome::Paused) => {
                let reason = session.reason();
                match session.on_pause(reason, &mut rl) {
                    Flow::Resume => {}
                    Flow::Quit => break,
                }
            }
            Ok(Outcome::Finished(count)) => {
                let values = session.finish(count).map_err(diag::report_plain)?;
                if !values.is_empty() {
                    println!(
                        "{}",
                        values
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                break;
            }
            Err(err) => return Err(diag::report_runtime(err.kind, err.trace)),
        }
    }
    Ok(())
}

fn compile_error(err: Box<dyn std::error::Error + Send + Sync>) -> Report {
    let err = match err.downcast::<DukaError>() {
        Ok(err) => *err,
        Err(err) => return diag::report_plain(err),
    };
    match err {
        DukaError::Spanned(err) => diag::report_spanned(&err),
        DukaError::Analysis(list) => diag::report_analysis(list),
        other => diag::report_plain(other),
    }
}

fn module_root(script: &Path) -> (PathBuf, String) {
    if let Ok(kao) = duka_lib::kao::find_kao(script) {
        return (
            kao.root().to_path_buf(),
            kao.modules_dir().to_string_lossy().into_owned(),
        );
    }
    let fallback = script
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut dir = script.parent().filter(|p| !p.as_os_str().is_empty());
    while let Some(at) = dir {
        if at.join("modules").is_dir() {
            return (at.to_path_buf(), "modules".to_owned());
        }
        dir = at.parent();
    }
    (fallback, "modules".to_owned())
}
