use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use duka_frontend::{
    ir::IRGenerator,
    lexer::LexerWithMacro,
    parser::Parser,
    prelude::{Adapter, Analyzer},
};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaParser};

pub fn benchmark(c: &mut Criterion) {
    let input = "function foo(x) return x*2 end";

    c.bench_function("lexer", |b| {
        b.iter(|| {
            let _: Vec<_> = LexerWithMacro::new(Cursor::new(input)).collect();
        })
    });

    c.bench_function("parser", |b| {
        let tokens: Vec<_> = LexerWithMacro::new(Cursor::new(input)).collect();
        b.iter(|| Parser::parse(tokens.clone().into_iter()))
    });

    c.bench_function("ir", |b| {
        let tokens: Vec<_> = LexerWithMacro::new(Cursor::new(input)).collect();
        let mut chunk = Parser::parse(tokens.clone().into_iter()).unwrap();
        let _ = Analyzer.analyze(&chunk);
        Adapter.adapt(&mut chunk);
        b.iter(|| IRGenerator::generate(chunk.clone()).unwrap())
    });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
