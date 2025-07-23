use std::io::Cursor;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use duka::{Lexer, frontend::token::TokenKind};
use rand::seq::IndexedRandom;

pub fn benchmark(c: &mut Criterion) {
    let inputs: Vec<&str> = vec![
        "local a = 1",
        "function foo(x) return x*2 end",
        "{1,2,'three',{key=value}}",
        "a and b or not c",
        "print('长字符串测试'..tostring(42))",
    ];

    c.bench_function("lexer", |b| {
        b.iter_batched(
            || inputs.choose(&mut rand::rng()).unwrap(),
            |input| {
                let mut lexer = Lexer::new(Cursor::new(input));
                loop {
                    match lexer.next_kind().unwrap() {
                        t if t.is_terminator() => break,
                        _ => continue,
                    }
                }
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
