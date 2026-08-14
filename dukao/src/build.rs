use std::path::{Path, PathBuf};
use std::time::Instant;

use colored::Colorize;
use duka_lib::kao::{Kao, collect_sources, find_kao};
use duka_shared::config::DukaConfig;

pub fn run_build_cmd(root: PathBuf, list: bool) -> i32 {
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
            .with_extension(duka_shared::constants::COMPILED_SUFFIX);

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
