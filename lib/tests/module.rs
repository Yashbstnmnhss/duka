use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use duka_backend::builtin::require;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_lib::harness::run;
use duka_lib::module::from_source;

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
) -> impl Fn(&str) -> Result<RuntimeValue, String> + Send + Sync + 'static {
    move |name| {
        let src = modules
            .get(name)
            .ok_or_else(|| format!("no module '{name}'"))?;
        let proto = from_source(src, Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        let results = VM::run(&proto).map_err(|e| format!("{e}"))?;
        Ok(results.last().cloned().unwrap_or(RuntimeValue::Nil))
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
    require::set_loader(move |name| {
        LOADS.fetch_add(1, Ordering::SeqCst);
        let proto = from_source("return 7", Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        let results = VM::run(&proto).map_err(|e| format!("{e}"))?;
        Ok(results.last().cloned().unwrap_or(RuntimeValue::Nil))
    });
    assert_eq!(
        s(r#"local a = require("m"); local b = require("m"); return a + b"#).unwrap(),
        "14"
    );
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);
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
    require::set_loader(move |name| {
        if FAIL.load(Ordering::SeqCst) {
            return Err("boom".to_string());
        }
        let proto = from_source("return 1", Some(name.to_owned()), Default::default())
            .map_err(|e| format!("{e}"))?;
        let results = VM::run(&proto).map_err(|e| format!("{e}"))?;
        Ok(results.last().cloned().unwrap_or(RuntimeValue::Nil))
    });
    let err = run(r#"return require("m")"#).unwrap_err();
    assert!(err.contains("boom"), "got: {err}");
    FAIL.store(false, Ordering::SeqCst);
    assert_eq!(s(r#"return require("m")"#).unwrap(), "1");
}
