use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use duka_backend::builtin::require::{self, LoadedModule};
use duka_backend::codegen::binary::{DukaBinary, Load};
use duka_backend::value::RuntimeValue;
use duka_lib::harness::run;
use duka_lib::module::{from_source, proto_to_bytes};
use duka_shared::config::DukaConfig;

static SERIAL: Mutex<()> = Mutex::new(());

fn s(src: &str) -> Result<String, String> {
    Ok(run(src)?
        .last()
        .cloned()
        .unwrap_or(RuntimeValue::Nil)
        .eval_to_string()
        .into_owned())
}

fn loader(
    modules: HashMap<String, String>,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    move |name, _caller_dir| {
        let src = modules
            .get(name)
            .ok_or_else(|| format!("no module '{name}'"))?;
        let proto = from_source(src, Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule { proto, path: None })
    }
}

#[test]
fn basic_loads() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    let modules = HashMap::from([(
        "greeter".to_string(),
        "return { hello = \"hi\", num = 7 }".to_string(),
    )]);
    require::set_loader(loader(modules));
    assert_eq!(s(r#"return require("greeter").hello"#).unwrap(), "hi");
    assert_eq!(s(r#"return require("greeter").num"#).unwrap(), "7");
}

static LOADS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn cached() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    LOADS.store(0, Ordering::SeqCst);
    require::set_loader(move |name, _caller_dir| {
        LOADS.fetch_add(1, Ordering::SeqCst);
        let proto = from_source("return 7", Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule { proto, path: None })
    });
    assert_eq!(
        s(r#"local a = require("m"); local b = require("m"); return a + b"#).unwrap(),
        "14"
    );
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);
}

#[test]
fn precompiled_dukac_loader() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    let proto = from_source(
        "return { hello = \"hi\" }",
        Some("greet".to_owned()),
        Default::default(),
    )
    .unwrap();
    let bytes = proto_to_bytes(&proto).unwrap();
    require::set_loader(move |name, _caller_dir| {
        if name != "greet" {
            return Err(format!("module '{name}' not registered"));
        }
        let proto = DukaBinary::load(&mut Cursor::new(bytes.as_slice()))
            .map(|b| b.into_proto())
            .map_err(|e| format!("binary error: {e}"))?;
        Ok(LoadedModule { proto, path: None })
    });
    assert_eq!(s(r#"return require("greet").hello"#).unwrap(), "hi");
}

#[test]
fn circular_require_errors() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    let modules = HashMap::from([
        ("A".to_string(), r#"return require("B")"#.to_string()),
        ("B".to_string(), r#"return require("A")"#.to_string()),
    ]);
    require::set_loader(loader(modules));
    let err = run(r#"return require("A")"#).unwrap_err();
    assert!(err.contains("circular require"), "got: {err}");
}

#[test]
fn self_require_errors() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    let modules = HashMap::from([("S".to_string(), r#"return require("S")"#.to_string())]);
    require::set_loader(loader(modules));
    let err = run(r#"return require("S")"#).unwrap_err();
    assert!(err.contains("circular require"), "got: {err}");
}

static FAIL: AtomicBool = AtomicBool::new(true);

#[test]
fn loader_error_recovered() {
    let _guard = SERIAL.lock().unwrap();
    require::reset();
    FAIL.store(true, Ordering::SeqCst);
    require::set_loader(move |name, _caller_dir| {
        if FAIL.load(Ordering::SeqCst) {
            return Err("boom".to_string());
        }
        let proto = from_source("return 1", Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        Ok(LoadedModule { proto, path: None })
    });
    let err = run(r#"return require("m")"#).unwrap_err();
    assert!(err.contains("boom"), "got: {err}");
    FAIL.store(false, Ordering::SeqCst);
    assert_eq!(s(r#"return require("m")"#).unwrap(), "1");
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
        Some(main_path.to_str().unwrap().to_owned()),
        DukaConfig::default(),
    )
    .unwrap();
    assert!(proto.instructions.len() > 0);
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
        Some(main_path.to_str().unwrap().to_owned()),
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
        Some(main_path.to_str().unwrap().to_owned()),
        DukaConfig::default(),
    )
    .unwrap();
    assert!(proto.instructions.len() > 0);
}

#[test]
fn cross_file_type_circular_errors() {
    let (_dir, main_path) = with_files(
        "local a: RequireType(\"a\").X = 1\nreturn a",
        &[
            ("modules/a.duka", "export type X = RequireType(\"b\").Y"),
            ("modules/b.duka", "export type Y = RequireType(\"a\").X"),
        ],
    );
    let source = std::fs::read_to_string(&main_path).unwrap();
    let err = from_source(
        &source,
        Some(main_path.to_str().unwrap().to_owned()),
        DukaConfig::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("circular require"), "got: {err}");
}
