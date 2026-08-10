//! End-to-end linq sugar tests: `linq!(from ... where ... select ...)` -> parse -> compile -> run.

use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
    let lexer = Lexer::new(Cursor::new(src), None, Default::default());
    let stream = lexer.tokenize().map_err(|e| format!("{e}"))?;
    let chunk = Parser::parse(stream, Default::default()).map_err(|e| format!("{e}"))?;
    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(format!("{err}"));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);
    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

fn run_last(src: &str) -> Result<RuntimeValue, String> {
    Ok(dbg!(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil)))
}

#[test]
fn linq_from_select() {
    // 从表选出值,结果按序写入 out[1..n]
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    select x * 10
)
return out[1] + out[2] + out[3]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn linq_where_filter() {
    // where 过滤后 select 映射
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    where x > 1
    select x * 10
)
return out[1] + out[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(50));
}

#[test]
fn linq_empty_result() {
    // 全部被过滤:out 为空表,out[1] = nil
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    where x > 100
    select x
)
return out[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Nil);
}

#[test]
fn linq_multiple_clauses() {
    // 多 from 组合 + where 链
    let r = run_last(
        r#"
global a = {1, 2}
global b = {10, 20}
global out = linq!(
    from x in a
    from y in b
    where x + y > 15
    select x + y
)
return out[1] + out[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(43));
}
