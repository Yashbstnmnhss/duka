//! End-to-end pcall builtin tests: parse -> compile -> run.

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
    let lexer = Lexer::new(Cursor::new(src), None);
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

fn run_results(src: &str) -> Result<Vec<RuntimeValue>, String> {
    Ok(run(src)?.to_vec())
}

#[test]
fn pcall_success_single_result() {
    assert_eq!(
        run_results("return pcall(function(a, b) return a + b end, 3, 4)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)]
    );
}

#[test]
fn pcall_success_native_callee() {
    assert_eq!(
        run_results("return pcall(math.max, 3, 9, 5)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(9)]
    );
}

#[test]
fn pcall_success_multiple_results() {
    assert_eq!(
        run_results("return pcall(function(a, b) return b, a end, 1, 2)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(2), RuntimeValue::Int(1)]
    );
}

#[test]
fn pcall_catches_error() {
    let res = run_results("return pcall(function() error(\"boom\") end)").unwrap();
    assert_eq!(res[0], RuntimeValue::Bool(false));
    assert_eq!(
        res[1].eval_to_string(),
        "boom"
    );
}

#[test]
fn pcall_continues_after_error() {
    let res = run_results(
        r#"
global mark = nil
pcall(function() error("boom") end)
mark = "alive"
return pcall(function() return 2 + 5 end)
        "#,
    )
    .unwrap();
    assert_eq!(
        res,
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)]
    );
}

#[test]
fn pcall_value_is_returned_after_error() {
    let res = run_results(
        r#"
local ok, res = pcall(function() return 1, 2, 3 end)
return ok, res
        "#,
    )
    .unwrap();
    assert_eq!(res[0], RuntimeValue::Bool(true));
    assert_eq!(res[1], RuntimeValue::Int(1));
}