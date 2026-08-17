use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use colored::Colorize;
use duka_lib::duka_shared::{constants::SOURCE_SUFFIX, errors::DukaSpannedError};
use duka_lib::kao::find_kao;
use duka_lib::{DukaVM, builtin, vm::VM};

use crate::diag::{render_compile_error, render_runtime_error};

pub fn run_test_cmd(dir: PathBuf, list: bool, filter: Option<&str>) -> i32 {
    if !dir.exists() {
        eprintln!(
            "{}: directory not found: {}",
            "error".red().bold(),
            dir.display()
        );
        return 2;
    }

    let mut files = collect_duka_files(&dir);
    files.sort();
    if let Some(f) = filter {
        files.retain(|p| p.to_string_lossy().contains(f));
    }

    if list {
        for f in &files {
            println!("{}", f.display());
        }
        println!("\n{} test(s)", files.len());
        return 0;
    }

    let mut results = Vec::with_capacity(files.len());
    let start = Instant::now();
    for f in &files {
        results.push(run_test(f)); // 逐个运行测试
    }
    let total = start.elapsed();

    let passed_count = results.iter().filter(|r| r.passed).count();
    let failed_count = results.len() - passed_count;

    for r in &results {
        if r.passed {
            println!(
                "[{}] {} ({:.1} ms)",
                "PASS".green().bold(),
                r.path.display().to_string().underline(),
                ms(r.duration)
            );
        } else {
            println!(
                "[{}] {} ({:.1} ms)",
                "FAIL".red().bold(),
                r.path.display(),
                ms(r.duration)
            );
            if let Some(out) = &r.output
                && !out.is_empty()
            {
                let text = String::from_utf8_lossy(out);
                for line in text.lines() {
                    println!("        {line}");
                }
            }
            if let Some(detail) = &r.detail {
                for line in detail.lines() {
                    println!("      {line}");
                }
            }
        }
        println!();
    }
    println!(
        "{}",
        format!(
            "=== {} passed, {} failed; {} test(s) ({:.1} ms) ===",
            passed_count,
            failed_count,
            results.len(),
            ms(total)
        )
        .bold()
    );

    if failed_count > 0 { 1 } else { 0 }
}

/// Duration to ms
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Recursively collect `.duka` sources, skipping `modules/` dirs & compiled files.
fn collect_duka_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if n == "modules" => {}
                    _ => stack.push(path),
                }
            } else if matches!(path.extension(), Some(t) if t == SOURCE_SUFFIX) {
                out.push(path);
            }
        }
    }
    out
}

fn run_test(path: &Path) -> TestResult {
    let kao = find_kao(path).ok();
    let start = Instant::now();

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let paths = duka_lib::module::search_paths(parent);
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    builtin::require::set_loader(duka_lib::module::file_loader(paths));

    let proto = match duka_lib::module::load_proto(
        path,
        kao.map(|i| i.manifest().map(|v| v.build.config.clone()).flatten())
            .flatten()
            .unwrap_or_default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            let detail = match e.downcast::<DukaSpannedError>() {
                Ok(spanned) => render_compile_error(path, *spanned),
                Err(e) => e.to_string(),
            };
            return TestResult {
                path: path.to_path_buf(),
                passed: false,
                detail: Some(detail),
                output: None,
                duration: start.elapsed(),
            };
        }
    };

    let mut vm = VM::new(duka_lib::duka_gc::Heap::new());
    vm.set_entry_path(path.to_path_buf());
    vm.set_stdout(Some(sink.clone()));
    let result = vm.execute(&proto);
    let captured = vm.take_stdout().map(|c| {
        c.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    });

    match result {
        Ok(_) => TestResult {
            path: path.to_path_buf(),
            passed: true,
            detail: None,
            output: captured,
            duration: start.elapsed(),
        },
        Err(e) => TestResult {
            path: path.to_path_buf(),
            passed: false,
            detail: Some(render_runtime_error(&e)),
            output: captured,
            duration: start.elapsed(),
        },
    }
}

#[derive(Debug)]
struct TestResult {
    path: PathBuf,
    passed: bool,
    detail: Option<String>,
    output: Option<Vec<u8>>,
    duration: Duration,
}
