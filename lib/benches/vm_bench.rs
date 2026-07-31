//! Custom benchmark harness for the Duka VM (no external deps).
//!
//! Run with: `cargo bench -p duka-lib` (compiles in release mode).
//! Optional filter: `cargo bench -p duka-lib -- fib` runs only scripts whose
//! name contains "fib".

use std::path::{Path, PathBuf};
use std::time::Instant;

use duka_backend::value::{DukaProto, RuntimeValue};
use duka_lib::module::compile_file;

struct BenchCase {
    name: &'static str,
    script: &'static str,
    expected: fn(&[RuntimeValue]) -> bool,
}

fn is_int(vals: &[RuntimeValue], want: i64) -> bool {
    matches!(vals.last(), Some(RuntimeValue::Int(i)) if *i == want)
}

const CASES: &[BenchCase] = &[
    BenchCase {
        name: "fib",
        script: "fib.duka",
        expected: |v| is_int(v, 832040),
    },
    BenchCase {
        name: "strcat",
        script: "strcat.duka",
        expected: |v| is_int(v, 50000),
    },
    BenchCase {
        name: "arrayfill",
        script: "arrayfill.duka",
        expected: |v| is_int(v, 5000050000),
    },
    BenchCase {
        name: "gcstress",
        script: "gcstress.duka",
        expected: |v| is_int(v, 50001),
    },
    BenchCase {
        name: "native",
        script: "native.duka",
        expected: |v| is_int(v, 5888896),
    },
];

fn scripts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/scripts")
}

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default().to_lowercase();

    let mut failed = false;
    println!("{:<12} {:>10} {:>10} {:>12} {:>10}", "bench", "compile", "min(ms)", "avg(ms)", "runs");
    println!("{:-<58}", "");

    for case in CASES {
        if !filter.is_empty() && !case.name.contains(&filter) {
            continue;
        }

        let path = scripts_dir().join(case.script);

        // Compile once.
        let compile_start = Instant::now();
        let proto: DukaProto = match compile_file(&path) {
            Ok(p) => p,
            Err(e) => {
                println!("{:<12} COMPILE ERROR: {e}", case.name);
                failed = true;
                continue;
            }
        };
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;

        // Verify correctness on the first run (also warms caches).
        match duka_backend::vm::VM::run(&proto) {
            Ok(vals) => {
                if !(case.expected)(&vals) {
                    println!(
                        "{:<12} WRONG RESULT: got {:?} (compile {:.2}ms)",
                        case.name, vals, compile_ms
                    );
                    failed = true;
                    continue;
                }
            }
            Err(e) => {
                println!("{:<12} RUNTIME ERROR: {e}", case.name);
                failed = true;
                continue;
            }
        }

        // 5 timed runs.
        const RUNS: usize = 5;
        let mut times: Vec<f64> = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let start = Instant::now();
            duka_backend::vm::VM::run(&proto).expect("bench rerun failed");
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
        let avg = times.iter().sum::<f64>() / times.len() as f64;

        println!(
            "{:<12} {:>9.2}ms {:>9.2}ms {:>11.2}ms {:>8}",
            case.name, compile_ms, min, avg, RUNS
        );
    }

    if failed {
        std::process::exit(1);
    }
}
