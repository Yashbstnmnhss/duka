use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_dukao")
}

fn tmp_kao(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dukao-bundle-{name}-{}", std::process::id()));
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

fn demo_app(root: &Path) {
    write(&root, "kao.toml", "[kao]\nname = \"demo\"\n");
    write(
        &root,
        "src/main.duka",
        "local util = require(\"./util\")\nlocal a = require(\"a\")\nprint(util.hello() .. \" \" .. a.name() .. \" \" .. ...)\n",
    );
    write(
        &root,
        "src/util.duka",
        "local M = {}\nfunction M.hello() return \"hi\" end\nreturn M\n",
    );
    write(
        &root,
        "modules/a.duka",
        "local M = {}\nfunction M.name() return \"a\" end\nreturn M\n",
    );
}

fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in s.chars() {
        if c == '=' {
            break;
        }
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => continue,
        };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

#[test]
fn bundle_exe_runs_with_requires_and_args() {
    let root = tmp_kao("exe");
    demo_app(&root);
    let out = run(&root, &["build", "--exe"]);
    assert!(out.status.success());
    let exe_path = root.join("build").join("demo.exe");
    assert!(exe_path.is_file());
    let out = Command::new(&exe_path).arg("world").output().unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("hi a world"));
}

#[test]
fn bundle_exe_custom_output() {
    let root = tmp_kao("exeout");
    demo_app(&root);
    let out = run(&root, &["build", "--exe", "dist/app.exe"]);
    assert!(out.status.success());
    assert!(root.join("dist/app.exe").is_file());
}

#[test]
fn bundle_wasm_produces_self_contained_js() {
    let root = tmp_kao("wasm");
    demo_app(&root);
    let out = run(&root, &["build", "--wasm"]);
    assert!(out.status.success());
    let js_path = root.join("build").join("demo.js");
    let js = fs::read_to_string(&js_path).unwrap();
    assert!(js.contains("const ENTRY = \"src/main.duka\""));
    assert!(js.contains("src/util.duka"));
    assert!(js.contains("modules/a.duka"));
    assert!(js.contains("export async function run"));
    let runtime = js
        .split("const RUNTIME = \"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let bytes = base64_decode(runtime);
    assert!(bytes.len() > 1000);
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn bundle_wasm_runs_on_node() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("node not available, skipping");
        return;
    }
    let root = tmp_kao("wasmrun");
    demo_app(&root);
    let out = run(&root, &["build", "--wasm"]);
    assert!(out.status.success());
    write(
        &root,
        "run.mjs",
        "import { run } from './build/demo.js';\nconst r = await run(['world']);\nprocess.stdout.write(r.stdout);\nprocess.exit(r.stderr ? 1 : 0);\n",
    );
    let out = Command::new("node").arg("run.mjs").current_dir(&root).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("hi a world"));
}
