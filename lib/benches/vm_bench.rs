use std::path::{Path, PathBuf};

use criterion::{Criterion, criterion_group, criterion_main};
use duka_backend::value::{DukaProto, RuntimeValue};
use duka_backend::vm::VM;
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

fn bench_vm(c: &mut Criterion) {
    for case in CASES {
        let path = scripts_dir().join(case.script);
        let proto: DukaProto = match compile_file(&path) {
            Ok(p) => p,
            Err(e) => panic!("{}: compile error: {e}", case.name),
        };

        let vals = match VM::run(&proto) {
            Ok(v) => v,
            Err(e) => panic!("{}: runtime error: {e}", case.name),
        };
        assert!(
            (case.expected)(&vals),
            "{}: wrong result {:?}",
            case.name,
            vals
        );

        c.bench_function(&format!("vm/{}", case.name), |b| {
            b.iter(|| VM::run(&proto).expect("bench rerun failed"))
        });
    }
}

criterion_group!(benches, bench_vm);
criterion_main!(benches);
