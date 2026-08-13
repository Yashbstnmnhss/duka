//! Commandline tool for duka
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
use colored::Colorize;
use duka_backend::{
    builtin, errors::DukaTraceError, vm::VM,
    DukaVM,
};
use duka_shared::errors::{DukaErrorKind, DukaSpannedError, Span};
use miette::{Diagnostic, LabeledSpan, NamedSource, Report, SourceOffset, SourceSpan};
use thiserror::Error;

const VERSION: &str = "0.1.0";

#[derive(Parser, Debug)]
#[command(
    version(VERSION),
    about("Test & package tools for duka language"),
    author("AogangSolang")
)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run duka scripts under a directory as unit tests
    Test {
        /// Directory to scan for `.duka` test scripts (default: `./tests`)
        path: Option<PathBuf>,

        /// Only list tests, do not run them
        #[arg(long, short)]
        list: bool,

        /// Only run tests whose path contains this substring
        #[arg(long)]
        filter: Option<String>,

        /// Disable colored output
        #[arg(long)]
        no_color: bool,
    },
}

#[derive(Debug)]
struct TestResult {
    path: PathBuf,
    passed: bool,
    detail: Option<String>,
    output: Option<Vec<u8>>,
    duration: Duration,
}

fn main() {
    let args = Args::parse();
    let exit = match args.cmd {
        Commands::Test {
            path,
            list,
            filter,
            no_color,
        } => {
            if no_color {
                colored::control::set_override(false);
            }
            run_test_cmd(
                path.unwrap_or_else(|| PathBuf::from("./tests")),
                list,
                filter.as_deref(),
            )
        }
    };
    std::process::exit(exit);
}

fn run_test_cmd(dir: PathBuf, list: bool, filter: Option<&str>) -> i32 {
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
            } else if path.extension().and_then(|e| e.to_str()) == Some("duka") {
                out.push(path);
            }
        }
    }
    out
}

fn run_test(path: &Path) -> TestResult {
    let start = Instant::now();

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let paths = duka_lib::module::search_paths(parent);
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    builtin::require::set_loader(duka_lib::module::file_loader_with_output(
        paths,
        Some(sink.clone()),
    ));

    let proto = match duka_lib::module::load_proto(path) {
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

    let mut vm = VM::new(duka_gc::Heap::new());
    vm.set_output(Some(sink.clone()));
    let result = vm.execute(&proto);
    let captured = vm.take_output().map(|c| c.lock().unwrap_or_else(|poison| poison.into_inner()).clone());

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

fn span_to_source_span(code: &str, span: Span) -> SourceSpan {
    SourceSpan::new(
        SourceOffset::from_location(code, span.start.line as usize, span.start.column as usize),
        span.char_len() as usize,
    )
}

fn render_compile_error(path: &Path, err: DukaSpannedError) -> String {
    let Ok(src) = std::fs::read_to_string(path) else {
        return err.to_string();
    };
    let code = src.clone();
    let span = span_to_source_span(&code, err.span);
    let relates = err
        .related
        .iter()
        .map(|(label, span)| LabeledSpan::at(span_to_source_span(&code, *span), label.clone()))
        .collect::<Vec<_>>();
    let diag = DukaSpannedDiagnose {
        source_code: NamedSource::new(path.to_string_lossy().into_owned(), code)
            .with_language("duka"),
        span,
        related_spans: relates,
        help: err.kind.get_help(),
        source: err.kind,
    };
    format!("{:?}", Report::new(diag))
}

#[derive(Debug, Error, Diagnostic)]
#[error("Duka error")]
#[diagnostic()]
struct DukaSpannedDiagnose {
    #[label(primary, "here")]
    span: SourceSpan,
    #[label(collection, "related to this")]
    related_spans: Vec<LabeledSpan>,
    #[help]
    help: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[source]
    source: DukaErrorKind,
}

fn render_runtime_error(e: &DukaTraceError) -> String {
    let mut out = format!("Runtime error: {}", e.kind);
    if !e.trace.frames.is_empty() {
        out.push('\n');
        out.push_str(e.trace.to_string().trim_end());
    }
    out
}
