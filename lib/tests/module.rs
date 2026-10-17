use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use duka_backend::builtin::require::{self, LoadedModule, default_vm_globals};
use duka_backend::codegen::binary::{DukaBinary, Load};
use duka_backend::value::RuntimeValue;
use duka_backend::vm::coroutine::VmGlobalsHandle;
use duka_lib::harness::{DukaError, Setup, run, run_with};
use duka_lib::module::{FileModuleSourceProvider, from_source, proto_to_bytes};
use duka_shared::config::DukaConfig;
use duka_shared::types::SourceName;

static SERIAL: Mutex<()> = Mutex::new(());

/// Take the lock even when a previous test panicked under it, so one failure
/// does not take the rest of the section with it
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn s(src: &str) -> Result<String, DukaError> {
    Ok(run(src)?
        .last()
        .cloned()
        .unwrap_or(RuntimeValue::Nil)
        .eval_to_string()
        .into_owned())
}

fn install<L>(l: L) -> VmGlobalsHandle
where
    L: Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static,
{
    let vm = default_vm_globals();
    require::set_loader(&vm, l);
    vm
}

fn s_in(vm: &VmGlobalsHandle, src: &str) -> Result<String, DukaError> {
    Ok(run_with(src, vm.clone())?
        .last()
        .cloned()
        .unwrap_or(RuntimeValue::Nil)
        .eval_to_string()
        .into_owned())
}

fn run_in(vm: &VmGlobalsHandle, src: &str) -> Result<Box<[RuntimeValue]>, DukaError> {
    run_with(src, vm.clone())
}

fn loader(
    modules: HashMap<String, String>,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    move |name, _caller_dir| {
        let src = modules
            .get(name)
            .ok_or_else(|| format!("no module '{name}'"))?;
        let proto = from_source(src, SourceName::Virtual(name.into()), Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule::Executable { proto, path: None })
    }
}

#[test]
fn basic_loads() {
    let _guard = serial();
    let modules = HashMap::from([(
        "greeter".to_string(),
        "return { hello = \"hi\", num = 7 }".to_string(),
    )]);
    let vm = install(loader(modules));
    assert_eq!(
        s_in(&vm, r#"return require("greeter").hello"#).unwrap(),
        "hi"
    );
    assert_eq!(s_in(&vm, r#"return require("greeter").num"#).unwrap(), "7");
}

static LOADS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn cached() {
    let _guard = serial();
    LOADS.store(0, Ordering::SeqCst);
    let vm = install(move |_, _| {
        LOADS.fetch_add(1, Ordering::SeqCst);
        let proto = from_source("return 7", SourceName::Unnamed, Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule::Executable { proto, path: None })
    });
    assert_eq!(
        s_in(
            &vm,
            r#"local a = require("m"); local b = require("m"); return a + b"#
        )
        .unwrap(),
        "14"
    );
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);
}

#[test]
fn precompiled_dukac_loader() {
    let _guard = serial();
    let proto = from_source(
        "return { hello = \"hi\" }",
        SourceName::Unnamed,
        Default::default(),
    )
    .unwrap();
    let bytes = proto_to_bytes(&proto).unwrap();
    let vm = install(move |name, _caller_dir| {
        if name != "greet" {
            return Err(format!("module '{name}' not registered"));
        }
        let proto = DukaBinary::load(&mut Cursor::new(bytes.as_slice()))
            .map(|b| b.into_proto())
            .map_err(|e| format!("binary error: {e}"))?;
        Ok(LoadedModule::Executable { proto, path: None })
    });
    assert_eq!(s_in(&vm, r#"return require("greet").hello"#).unwrap(), "hi");
}

#[test]
fn circular_require_errors() {
    let _guard = serial();
    let modules = HashMap::from([
        ("A".to_string(), r#"return require("B")"#.to_string()),
        ("B".to_string(), r#"return require("A")"#.to_string()),
    ]);
    let vm = install(loader(modules));
    let err = run_in(&vm, r#"return require("A")"#).unwrap_err();
    assert!(err.to_string().contains("circular require"), "got: {err}");
}

#[test]
fn self_require_errors() {
    let _guard = serial();
    let modules = HashMap::from([("S".to_string(), r#"return require("S")"#.to_string())]);
    let vm = install(loader(modules));
    let err = run_in(&vm, r#"return require("S")"#).unwrap_err();
    assert!(err.to_string().contains("circular require"), "got: {err}");
}

static FAIL: AtomicBool = AtomicBool::new(true);

#[test]
fn loader_error_recovered() {
    let _guard = serial();
    FAIL.store(true, Ordering::SeqCst);
    let vm = install(move |_, _| {
        if FAIL.load(Ordering::SeqCst) {
            return Err("boom".to_string());
        }
        let proto = from_source("return 1", SourceName::Unnamed, Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule::Executable { proto, path: None })
    });
    let err = run_in(&vm, r#"return require("m")"#).unwrap_err();
    assert!(err.to_string().contains("boom"), "got: {err}");
    FAIL.store(false, Ordering::SeqCst);
    assert_eq!(s_in(&vm, r#"return require("m")"#).unwrap(), "1");
}

static DIRS: AtomicUsize = AtomicUsize::new(0);

fn with_files(main: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "duka_ft_{}_{}",
        std::process::id(),
        DIRS.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("modules")).unwrap();
    for (rel, content) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let main_path = dir.join("main.duka");
    std::fs::write(&main_path, main).unwrap();
    (dir, main_path)
}

#[test]
fn cross_file_type_requires_ok() {
    let (_dir, main_path) = with_files(
        "local a: RequireType(\"m\").Alias = { x = 1, y = \"s\" }\nreturn a.x",
        &[(
            "modules/m.duka",
            "export type Alias = { x: int, y: string }",
        )],
    );
    let source = std::fs::read_to_string(&main_path).unwrap();
    let proto = from_source(
        &source,
        SourceName::File("main".into(), main_path.into()),
        DukaConfig::default(),
    )
    .unwrap();
    assert!(!proto.instructions.is_empty());
}

#[test]
fn harness_resolves_a_require_type_when_given_a_provider() {
    let (_dir, main_path) = with_files(
        "local a: RequireType(\"m\").Alias = { x = 1, y = \"s\" }\nreturn a.x",
        &[(
            "modules/m.duka",
            "export type Alias = { x: int, y: string }",
        )],
    );
    let source = std::fs::read_to_string(&main_path).unwrap();
    let provider = FileModuleSourceProvider::for_entry(Some(&main_path));
    let ir = Setup::new()
        .provider(&provider)
        .ir(&source)
        .unwrap_or_else(|e| panic!("harness should ask the provider: {e}"));
    assert!(!ir.instructions.is_empty());
}

#[test]
fn cross_file_type_requires_rejects_mismatch() {
    let (_dir, main_path) = with_files(
        "local a: RequireType(\"m\").Alias = { x = 1 }\nreturn a",
        &[(
            "modules/m.duka",
            "export type Alias = { x: int, y: string }",
        )],
    );
    let source = std::fs::read_to_string(&main_path).unwrap();
    let err = from_source(
        &source,
        SourceName::File("main".into(), main_path.into()),
        DukaConfig::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("Type"), "got: {err}");
}

#[test]
fn cross_file_type_missing_module_is_any() {
    let (_dir, main_path) = with_files("local a: RequireType(\"nope\").X = 1\nreturn a", &[]);
    let source = std::fs::read_to_string(&main_path).unwrap();
    let proto = from_source(
        &source,
        SourceName::File("".into(), main_path.into()),
        DukaConfig::default(),
    )
    .unwrap();
    assert!(proto.instructions.len() > 0);
}

#[test]
fn cross_file_type_circular_errors() {
    let (_dir, main_path) = with_files(
        "local a: RequireType(\"a\").X = RequireType(\"b\").Y\nreturn a",
        &[
            ("modules/a.duka", "export type X = RequireType(\"b\").Y"),
            ("modules/b.duka", "export type Y = RequireType(\"a\").X"),
        ],
    );
    let source = std::fs::read_to_string(&main_path).unwrap();
    let err = from_source(
        &source,
        SourceName::File("TEST".into(), main_path.into()),
        DukaConfig::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("circular require"), "got: {err}");
}

/// `duka-ui`'s vnode serializer leans on a `return` inside a `do` leaving the
/// function from a `match` arm, which is the one path `gen_expr_block` must not
/// turn into the block's value. Inlined rather than read off disk: the module
/// tree it came from is gitignored, so a path-based copy only ever ran on the
/// machine that happened to have it.
#[test]
fn match_arm_return_inside_do_leaves_the_function() {
    let src = |arg: &str| {
        format!(
            r#"
local function describe(node)
    local t = typeof(node)
    match t then
        "string" -> do return "str" end
        "int" -> do return "int" end
        "float" -> do return "float" end
    end
    return "obj"
end
return describe({arg})
"#
        )
    };
    for (arg, want) in [
        (r#""a""#, "str"),
        ("7", "int"),
        ("1.5", "float"),
        (r#"{ tag = "t" }"#, "obj"),
    ] {
        assert_eq!(s(&src(arg)).unwrap(), want, "arg: {arg}");
    }
}
