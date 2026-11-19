use std::path::PathBuf;

use duka::CPluginLoader;
use duka_capi_plugin_fixture::dukaopen_fixture;
use duka_lib::builtin::require::new_vm_globals;
use duka_lib::harness::Setup;
use duka_lib::value::RuntimeValue;
use duka_shared::types::SourceName;

#[test]
fn plugin_is_reachable_through_require() {
    let loader = CPluginLoader::new().add_function("fixture", dukaopen_fixture);
    let globals = new_vm_globals(loader.loader());
    let source = "local m = require(\"fixture\") return m.tri_add(10, 32)";
    let result = Setup::new()
        .vm_state(globals)
        .run_last(source, SourceName::Unnamed)
        .expect("run succeed");
    assert_eq!(result, RuntimeValue::Int(42));
}

#[test]
fn unknown_plugin_reports_error() {
    let loader = CPluginLoader::new();
    let globals = new_vm_globals(loader.loader());
    let source = "return require(\"no_such_plugin\")";
    let result = Setup::new()
        .vm_state(globals)
        .run_last(source, SourceName::Unnamed);
    assert!(result.is_err(), "require of unknown plugin must fail");
}

fn fixture_library() -> PathBuf {
    let ext = if cfg!(windows) { "dll" } else { "so" };
    let names = [
        format!("libduka_capi_plugin_fixture.{ext}"),
        format!("duka_capi_plugin_fixture.{ext}"),
    ];
    let mut dir = std::env::current_exe().expect("current exe");
    dir.pop();
    for _ in 0..8 {
        for name in &names {
            let candidate = dir.join(name);
            if candidate.exists() {
                return candidate;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    panic!(
        "fixture library not found near {:?}",
        std::env::current_exe()
    );
}

#[test]
#[ignore = "real dlopen; path-dependent, and cdylib feature boundaries can be unsafe in full workspace runs"]
fn plugin_loads_from_shared_library() {
    let path = fixture_library();
    let loader = CPluginLoader::new().add("fixture", path);
    let globals = new_vm_globals(loader.loader());
    let source = "local m = require(\"fixture\") return m.tri_add(10, 32)";
    let result = Setup::new()
        .vm_state(globals)
        .run_last(source, SourceName::Unnamed)
        .expect("run succeed");
    assert_eq!(result, RuntimeValue::Int(42));
}
