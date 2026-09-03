use std::path::{Path, PathBuf};
use std::time::Instant;

use colored::Colorize;
use duka_app::binary::{DukaAppBinary, bundle};
use duka_lib::codegen::binary::Dump;
use duka_lib::duka_shared::config::DukaConfig;
use duka_lib::duka_shared::constants::COMPILED_SUFFIX;
use duka_lib::kao::{Kao, collect_sources, find_kao};
use duka_lib::module::{compile_to_bytes, is_resource};

const WASM_FILE_NAME: &str = "duka.wasm";
const WASM_SOURCES_NAME: &str = "compiled";
const APP_WRAPPER: &[u8] = include_bytes!("../res/duka-app.exe");
const WASM_RUNTIME: &[u8] = include_bytes!("../res/duka-backend-wasm.wasm");
const WASM_GLUE: &str = include_str!("../res/duka-glue.js");

#[derive(Debug, clap::Subcommand, Default)]
pub(super) enum BuildTarget {
    /// Compiled duka files (.dukac)
    #[default]
    Compiled,
    /// Executable binary file (.exe)
    Exe,
    /// WASM target for Web
    WASM,
}

pub fn run_build_cmd(root: PathBuf, list: bool, target: BuildTarget) -> i32 {
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
        .and_then(|i| i.build.config.clone())
        .unwrap_or_default();

    match target {
        BuildTarget::Compiled => {
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
                // Resources (kao.toml, README.md, ...) are not compilable source;
                // only the module-map targets (exe/wasm) embed them as raw bytes.
                if is_resource(f) {
                    continue;
                }
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
                        .and_then(|i| i.build.config.clone())
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
        BuildTarget::Exe => {
            let out = default_output(&kao, "exe", "exe");
            return build_exe(&kao, &files, config, out);
        }
        BuildTarget::WASM => {
            let out = default_output_dir(&kao, "wasm");
            return build_wasm(&kao, &files, config, out);
        }
    }
}

fn build_exe(kao: &Kao, files: &[PathBuf], config: DukaConfig, output: PathBuf) -> i32 {
    let modules = match compile_all(kao, files, config) {
        Ok(m) => m,
        Err(_) => return 1,
    };
    let entry = kao.entry().to_string_lossy().replace('\\', "/");
    let app = DukaAppBinary::new(entry, modules);
    let mut archive = vec![];
    if let Err(e) = app.dump(&mut archive) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }
    write_output(&output, &bundle(APP_WRAPPER, &archive))
}

fn build_wasm(kao: &Kao, files: &[PathBuf], config: DukaConfig, output_dir: PathBuf) -> i32 {
    if std::fs::create_dir_all(&output_dir).is_err() {
        return 2;
    }

    let mut modules = match compile_all(kao, files, config) {
        Ok(m) => m,
        Err(_) => return 1,
    };

    // Collect kao.toml files from modules directory (raw bytes, not compiled)
    let modules_dir = kao.root().join(kao.modules_dir());
    if modules_dir.is_dir() {
        collect_kao_manifests(kao, &modules_dir, &mut modules);
    }

    let entry = kao.entry().to_string_lossy().replace('\\', "/");

    // write wasm
    let wasm_path = output_dir.join(WASM_FILE_NAME);
    write_output(&wasm_path, WASM_RUNTIME); //TODO: ERROR HANDLE!

    // bundle web library (snabbdom + duka-web) via esbuild
    let web_dir = kao.root().join("web");
    let web_entry = web_dir.join("entry.js");
    let web_dist = web_dir.join("dist").join("duka-web.js");
    let bundled_js = output_dir.join("duka-web.js");
    if web_entry.exists() {
        let status = std::process::Command::new("npx")
            .args(["esbuild", web_entry.to_str().unwrap()])
            .args(["--bundle", "--format=esm", "--minify"])
            .args(["--outfile", bundled_js.to_str().unwrap()])
            .current_dir(&web_dir)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // fallback: copy pre-built bundle if it exists
                if web_dist.exists() {
                    let _ = std::fs::copy(&web_dist, &bundled_js);
                }
            }
        }
    }

    // copy index.html from web dir if it exists at project root
    let index_html = web_dir.join("index.html");
    if index_html.exists() {
        let _ = std::fs::copy(&index_html, output_dir.join("index.html"));
    }

    // write dukac and resources
    let compiled_path = PathBuf::from(WASM_SOURCES_NAME);
    let compiled_real_path = output_dir.join(compiled_path.clone());
    let mut modules_mapper = vec![];
    for (name, bytes) in modules {
        let path = PathBuf::from(&name);
        let is_resource = duka_lib::module::is_resource(&path);

        if is_resource {
            // Resources: copy raw bytes, keep original filename in output
            let out_file = compiled_real_path.join(&name);
            write_output(&out_file, &bytes);
            modules_mapper.push(format!(
                "\"{name}\": \"{}/{}\"",
                WASM_SOURCES_NAME,
                name.replace('\\', "/")
            ));
        } else {
            // Compiled Duka files: rename to .dukac
            let file_name = format!(
                "{}.{}",
                name.replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_"),
                COMPILED_SUFFIX
            );
            let path = compiled_path.join(file_name.clone());
            modules_mapper.push(format!(
                "\"{name}\": \"{}\"",
                path.to_string_lossy().replace('\\', "/")
            ));
            write_output(&compiled_real_path.join(file_name), &bytes);
        }
    }

    // write js glue
    let js_code = format!(
        r#"// Generated by `dukao build`
const __ENTRY = "{entry}";
const __RUNTIME = "./{WASM_FILE_NAME}";
const __MODULES = {{{}}};
{WASM_GLUE}
"#,
        modules_mapper.join(",")
    );
    let js_path = output_dir.join("index.js");
    write_output(&js_path, js_code.as_bytes()); //TODO: error handle!

    0
}

fn compile_all(
    kao: &Kao,
    files: &[PathBuf],
    config: DukaConfig,
) -> Result<Vec<(String, Vec<u8>)>, ()> {
    let mut modules = vec![];
    let mut failed = 0;
    for f in files {
        let rel = f.strip_prefix(kao.root()).unwrap_or(f);
        let key = rel.to_string_lossy().replace('\\', "/");
        if is_resource(f) {
            if let Ok(bytes) = std::fs::read(f) {
                modules.push((key, bytes));
            }
            continue;
        }
        match compile_to_bytes(f, config.clone()) {
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

/// Scan `modules_dir` for `kao.toml` files and add their raw bytes to the
/// modules list so that `memory_loader` can resolve package entry points
/// via kao-based fallback.
///
/// Also adds a direct alias entry: `{pkg_root}` → the compiled entry bytecode,
/// so `require("pkg")` can find the module via the flat lookup without needing
/// the kao fallback (which depends on kao.toml being fetchable from the browser).
fn collect_kao_manifests(kao: &Kao, modules_dir: &Path, modules: &mut Vec<(String, Vec<u8>)>) {
    let mut stack = vec![modules_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip build/output directories
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == "build" || name == "node_modules" {
                    continue;
                }
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("kao.toml") {
                if let Ok(rel) = path.strip_prefix(kao.root()) {
                    let key = rel.to_string_lossy().replace('\\', "/");
                    if let Ok(bytes) = std::fs::read(&path) {
                        // Only add if not already present (avoid duplicates)
                        if !modules.iter().any(|(k, _)| k == &key) {
                            modules.push((key, bytes.clone()));
                        }
                        // Parse kao.toml to find entry and add direct alias
                        if let Ok(kao_str) = std::str::from_utf8(&bytes) {
                            if let Ok(manifest) =
                                toml::from_str::<duka_lib::kao::KaoManifest>(kao_str)
                            {
                                let pkg_root = rel.parent().unwrap_or(&rel);
                                let pkg_key = pkg_root.to_string_lossy().replace('\\', "/");
                                if !modules.iter().any(|(k, _)| k == &pkg_key) {
                                    let entry =
                                        manifest.build.entry.as_deref().unwrap_or("src/init.duka");
                                    let entry_path = format!("{}/{}", pkg_key, entry);
                                    if let Some((_, entry_bytes)) =
                                        modules.iter().find(|(k, _)| k == &entry_path)
                                    {
                                        modules.push((pkg_key, entry_bytes.clone()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn get_kao_name(kao: &Kao) -> String {
    kao.name()
        .map(|s| s.to_owned())
        .or_else(|| {
            kao.root()
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "app".to_owned())
}
fn default_output_dir(kao: &Kao, folder: &str) -> PathBuf {
    kao.root().join(kao.out_dir()).join(folder)
}
fn default_output(kao: &Kao, folder: &str, ext: &str) -> PathBuf {
    let name = get_kao_name(kao);
    kao.root()
        .join(kao.out_dir())
        .join(folder)
        .join(format!("{name}.{ext}"))
}

fn write_output(path: &Path, bytes: &[u8]) -> i32 {
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

fn collect_build_files(kao: &Kao) -> Result<Vec<PathBuf>, String> {
    let mut files = vec![];
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
