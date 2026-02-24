use std::io::Cursor;

use criterion::{Criterion, criterion_group, criterion_main};
use duka_frontend::{
    analyzer::ScopeAnalyzer,
    ir::IRGenerator,
    lexer::LexerWithMacro,
    parser::Parser,
    prelude::{Adapter, BasicAnalyzer},
};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

pub fn benchmark(c: &mut Criterion) {
    let input = "function foo(x) return x*2 end";

    c.bench_function("lexer", |b| {
        b.iter(|| {
            let _: Vec<_> = LexerWithMacro::new(Cursor::new(input), None).collect();
        })
    });

    c.bench_function("parser", |b| {
        let tokens = LexerWithMacro::new(Cursor::new(input), None)
            .tokenize()
            .unwrap();
        b.iter(|| Parser::parse(tokens.clone(), Default::default()))
    });

    c.bench_function("ir", |b| {
        let stream = LexerWithMacro::new(Cursor::new(input), None)
            .tokenize()
            .unwrap();
        let mut chunk = Parser::parse(stream, Default::default()).unwrap();
        let (data, _) = ScopeAnalyzer.analyze(&chunk, Default::default());
        let _ = BasicAnalyzer.analyze(&chunk, data);
        Adapter.adapt(&mut chunk);
        b.iter(|| IRGenerator::generate(chunk.clone(), Default::default()).unwrap())
    });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
