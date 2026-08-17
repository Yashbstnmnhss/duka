use std::path::{Path, PathBuf};
use std::time::Instant;

use colored::Colorize;
use duka_app::binary::{DukaAppBinary, bundle};
use duka_lib::codegen::binary::Dump;
use duka_lib::duka_shared::config::DukaConfig;
use duka_lib::kao::{Kao, collect_sources, find_kao};

const GLUE_JS: &str = include_str!("../res/duka-glue.js");
const APP_WRAPPER: &[u8] = include_bytes!("../res/duka-app.exe");
const WASM_RUNTIME: &[u8] = include_bytes!("../res/duka-backend-wasm.wasm");

pub fn run_build_cmd(
    root: PathBuf,
    list: bool,
    exe: Option<Option<PathBuf>>,
    wasm: Option<Option<PathBuf>>,
) -> i32 {
    let kao = match find_kao(&root) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
    };

    let files = match collect_build_files(&kao) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
    };

    if list {
        for f in &files {
            println!("{}", f.display());
        }
        println!("\n{} file(s)", files.len());
        return 0;
    }

    let config = kao
        .manifest()
        .map(|i| i.build.config.clone())
        .flatten()
        .unwrap_or_default();

    if let Some(out) = exe {
        let out = out.unwrap_or_else(|| default_output(&kao, "exe"));
        return build_exe(&kao, &files, config, out);
    }
    if let Some(out) = wasm {
        let out = out.unwrap_or_else(|| default_output(&kao, "js"));
        return build_wasm(&kao, &files, config, out);
    }

    let out_root = kao.root().join(kao.out_dir());
    if let Err(e) = std::fs::create_dir_all(&out_root) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }

    let start = Instant::now();
    let mut compiled = 0;
    let mut up_to_date = 0;
    let mut failed = 0;

    for f in &files {
        let rel = f.strip_prefix(kao.root()).unwrap_or(f);
        let out_path = out_root
            .join(rel)
            .with_extension(duka_lib::duka_shared::constants::COMPILED_SUFFIX);

        if let Some(cur) = out_path.metadata().ok().and_then(|m| m.modified().ok()) {
            if let Ok(src) = f.metadata().and_then(|m| m.modified()) {
                if cur >= src {
                    println!("{} {}", "up-to-date".yellow(), out_path.display());
                    up_to_date += 1;
                    continue;
                }
            }
        }

        match compile_one(
            f,
            &out_path,
            kao.manifest()
                .map(|i| i.build.config.clone())
                .flatten()
                .unwrap_or_default(),
        ) {
            Ok(()) => {
                compiled += 1;
                println!("{} {}", "compiled".green(), out_path.display());
            }
            Err(e) => {
                failed += 1;
                println!("{} {}: {}", "failed".red(), f.display(), e.trim_end());
            }
        }
    }

    println!(
        "{}",
        format!(
            "=== {} compiled, {} up-to-date, {} failed ({} ms) ===",
            compiled,
            up_to_date,
            failed,
            start.elapsed().as_secs_f64() * 1000.0
        )
        .bold()
    );

    if failed > 0 { 1 } else { 0 }
}

fn build_exe(kao: &Kao, files: &[PathBuf], config: DukaConfig, output: PathBuf) -> i32 {
    let modules = match compile_all(kao, files, config) {
        Ok(m) => m,
        Err(_) => return 1,
    };
    let entry = kao.entry().to_string_lossy().replace('\\', "/");
    let app = DukaAppBinary::new(entry, modules);
    let mut archive = Vec::new();
    if let Err(e) = app.dump(&mut archive) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }
    write_output(&output, bundle(APP_WRAPPER, &archive))
}

fn build_wasm(kao: &Kao, files: &[PathBuf], config: DukaConfig, output: PathBuf) -> i32 {
    let modules = match compile_all(kao, files, config) {
        Ok(m) => m,
        Err(_) => return 1,
    };
    let entry = kao.entry().to_string_lossy().replace('\\', "/");
    let mut js = String::new();
    js.push_str(&format!(r#"const RUNTIME = "{}";"#, base64(WASM_RUNTIME)));
    js.push_str(&format!(r#"const ENTRY = "{entry}";"#));
    js.push_str("const MODULES = {");
    for (key, bytes) in &modules {
        js.push_str(&format!(r#"  "{key}": "{}","#, base64(bytes)));
    }
    js.push_str("};");
    js.push_str(GLUE_JS);
    write_output(&output, js.into_bytes())
}

fn compile_all(
    kao: &Kao,
    files: &[PathBuf],
    config: DukaConfig,
) -> Result<Vec<(String, Vec<u8>)>, ()> {
    let mut modules = Vec::new();
    let mut failed = 0;
    for f in files {
        let rel = f.strip_prefix(kao.root()).unwrap_or(f);
        let key = rel.to_string_lossy().replace('\\', "/");
        match duka_lib::module::compile_to_bytes(f, config.clone()) {
            Ok(bytes) => modules.push((key, bytes)),
            Err(e) => {
                failed += 1;
                eprintln!(
                    "{} {}: {}",
                    "failed".red(),
                    f.display(),
                    e.to_string().trim_end()
                );
            }
        }
    }
    if failed > 0 { Err(()) } else { Ok(modules) }
}

fn default_output(kao: &Kao, ext: &str) -> PathBuf {
    let name = kao
        .name()
        .map(|s| s.to_owned())
        .or_else(|| {
            kao.root()
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "app".to_owned());
    kao.root().join(kao.out_dir()).join(format!("{name}.{ext}"))
}

fn write_output(path: &Path, bytes: Vec<u8>) -> i32 {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
    }
    match std::fs::write(path, bytes) {
        Ok(_) => {
            println!("{} {}", "built".green().bold(), path.display());
            0
        }
        Err(e) => {
            eprintln!("{}: {}", "error".red().bold(), e);
            2
        }
    }
}

/// Convert data into base64 format to inject it into javascript
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn collect_build_files(kao: &Kao) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let src_dir = kao.root().join(kao.src_dir());
    if src_dir.is_dir() {
        files.extend(collect_sources(kao, &src_dir)?);
    }
    let modules_dir = kao.root().join(kao.modules_dir());
    if modules_dir.is_dir() {
        files.extend(collect_sources(kao, &modules_dir)?);
    }
    Ok(files)
}

fn compile_one(src: &Path, out: &PathBuf, config: DukaConfig) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = duka_lib::module::compile_to_bytes(src, config).map_err(|e| e.to_string())?;
    std::fs::write(out, bytes).map_err(|e| e.to_string())?;
    Ok(())
}
