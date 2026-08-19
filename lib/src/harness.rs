//! Used for tests

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use duka_backend::DukaVM;
use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_backend::vm::coroutine::InputCell;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer, TypeChecker, TypeEval};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_frontend::parser::ast::DukaChunk;
use duka_shared::config::DukaIRConfig;
use duka_shared::ir::DukaIR;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn to_chunk(src: &str) -> Result<DukaChunk, String> {
    let lexer = Lexer::new(Cursor::new(src), None, Default::default());
    let stream = lexer.tokenize().map_err(|e| format!("{e}"))?;
    let mut chunk = Parser::parse(stream, Default::default()).map_err(|e| format!("{e}"))?;
    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .chain(TypeEval)
        .chain(TypeChecker)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(format!("{err}"));
    }
    Adapter.adapt(&mut chunk);
    Ok(chunk)
}

pub fn to_ir(src: &str) -> Result<DukaIR, String> {
    let chunk = to_chunk(src)?;
    IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))
}

pub fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

pub fn run_last(src: &str) -> Result<RuntimeValue, String> {
    Ok(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
}

pub fn run_with_input(src: &str, input: &[u8]) -> Result<Box<[RuntimeValue]>, String> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    let mut vm = VM::new(duka_gc::Heap::new());
    let cell: InputCell = Arc::new(Mutex::new(input.to_vec()));
    vm.set_input(Some(cell));
    let count = vm.execute(&proto).map_err(|e| format!("{e}"))?;
    vm.main_coroutine_mut()
        .inner
        .take_stack_many(0, count)
        .map_err(|e| format!("{e}"))
}

pub fn run_results(src: &str) -> Result<Vec<RuntimeValue>, String> {
    Ok(run(src)?.to_vec())
}
