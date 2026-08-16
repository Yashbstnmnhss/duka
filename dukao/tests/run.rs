use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_dukao")
}

fn tmp_kao(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dukao-run-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("modules")).unwrap();
    dir
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin()).args(args).current_dir(root).output().unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn run_default_entry() {
    let root = tmp_kao("default");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "print(\"Hello from run\")\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("Hello from run"));
}

#[test]
fn run_passes_script_args() {
    let root = tmp_kao("args");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(
        &root,
        "src/main.duka",
        "local args = {...}\nprint(#args)\nfor i = 0, #args - 1 do print(args[i]) end\n",
    );
    let out = run(&root, &["run", "--", "alpha", "beta"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("2"));
    assert!(text.contains("alpha"));
    assert!(text.contains("beta"));
}

#[test]
fn run_entry_override() {
    let root = tmp_kao("override");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "print(\"main\")\n");
    write(&root, "src/tool.duka", "print(\"tool\")\n");
    let out = run(&root, &["run", "--entry", "src/tool.duka"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("tool"));
    assert!(!stdout(&out).contains("main"));
}

#[test]
fn run_require_project_module() {
    let root = tmp_kao("module");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "local greet = require(\"greet\")\nprint(greet())\n");
    write(&root, "modules/greet.duka", "return function() return \"mod-ok\" end\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("mod-ok"));
}

#[test]
fn run_relative_require_sibling() {
    let root = tmp_kao("rel-sibling");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "local u = require(\"./utils\")\nprint(u.value)\n");
    write(&root, "src/utils.duka", "return { value = \"utils-ok\" }\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("utils-ok"));
}

#[test]
fn run_relative_require_updir() {
    let root = tmp_kao("rel-updir");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(
        &root,
        "src/main.duka",
        "local h = require(\"./net/http\")\nprint(h.value)\n",
    );
    write(
        &root,
        "src/net/http.duka",
        "local common = require(\"../common\")\nreturn { value = common.v }\n",
    );
    write(&root, "src/common.duka", "return { v = \"common-ok\" }\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("common-ok"));
}

#[test]
fn run_relative_require_init_dir() {
    let root = tmp_kao("rel-init");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "local d = require(\"./widget\")\nprint(d.value)\n");
    write(&root, "src/widget/init.duka", "return { value = \"widget-ok\" }\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("widget-ok"));
}

#[test]
fn run_relative_require_explicit_extension() {
    let root = tmp_kao("rel-ext");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(
        &root,
        "src/main.duka",
        "local u = require(\"./util.duka\")\nprint(u.value)\n",
    );
    write(&root, "src/util.duka", "return { value = \"ext-ok\" }\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("ext-ok"));
}

#[test]
fn run_package_module_relative_internal() {
    let root = tmp_kao("pkg-rel");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "local a = require(\"a\")\nprint(a.value)\n");
    write(&root, "modules/a.duka", "local b = require(\"./b\")\nreturn { value = b.v }\n");
    write(&root, "modules/b.duka", "return { v = \"inner-ok\" }\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("inner-ok"));
}

#[test]
fn run_relative_require_missing_errors() {
    let root = tmp_kao("rel-missing");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "require(\"./nope\")\n");
    let out = run(&root, &["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nope"));
}

#[test]
fn run_manifest_entry_config() {
    let root = tmp_kao("config");
    write(
        &root,
        "kao.toml",
        "[kao]\nname = \"demo\"\n\n[build]\nentry = \"src/start.duka\"\n",
    );
    write(&root, "src/start.duka", "print(\"started\")\n");
    let out = run(&root, &["run"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("started"));
}

#[test]
fn run_missing_entry_returns_2() {
    let root = tmp_kao("missing");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    let out = run(&root, &["run"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn run_compile_error_returns_1() {
    let root = tmp_kao("compile");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "local = 1\n");
    let out = run(&root, &["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error"));
}

#[test]
fn run_runtime_error_returns_1() {
    let root = tmp_kao("runtime");
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(&root, "src/main.duka", "error(\"boom\")\n");
    let out = run(&root, &["run"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("boom"));
}
