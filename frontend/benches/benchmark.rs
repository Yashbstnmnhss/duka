use std::io::Cursor;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use duka_frontend::{lexer::LexerWithMacro, parser::Parser};
use rand::seq::IndexedRandom;

pub fn benchmark(c: &mut Criterion) {
    let inputs: Vec<&str> = vec![
        "local a = 1",
        "function foo(x) return x*2 end",
        "b = {1,2,'three',{key=value}}",
        "d = a and b or not c",
        "print('长字符串测试'..tostring(42))",
    ];

    c.bench_function("lexer", |b| {
        b.iter_batched(
            || inputs.choose(&mut rand::rng()).unwrap(),
            |input| {
                let _: Vec<_> = LexerWithMacro::new(Cursor::new(input)).collect();
            },
            BatchSize::SmallInput,
        )
    });

    c.bench_function("parser", |b| {
        b.iter_batched(
            || inputs.choose(&mut rand::rng()).unwrap(),
            |input| {
                let lexer = LexerWithMacro::new(Cursor::new(input));
                Parser::new(lexer).parse_chunk().unwrap()
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
