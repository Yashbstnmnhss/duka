//! Used for tests

use std::fmt::Display;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use duka_backend::DukaVM;
use duka_backend::codegen::DefaultGenerator;
use duka_backend::codegen::errors::DukaCodegenError;
use duka_backend::errors::{DukaRuntimeError, DukaTraceError};
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_backend::vm::coroutine::InputCell;
use duka_frontend::analyzer::prelude::inject_type_prelude;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer, TypeChecker, TypeEval};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_frontend::parser::ast::DukaChunk;
use duka_shared::config::DukaIRConfig;
use duka_shared::errors::{DukaErrorLevel, DukaIRError, DukaSpannedError};
use duka_shared::ir::DukaIR;
use duka_shared::types::{
    DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser, SourceName,
};

fn to_chunk(src: &str) -> Result<DukaChunk, DukaError> {
    let lexer = Lexer::new(Cursor::new(src), SourceName::Unnamed, Default::default());
    let stream = lexer.tokenize().map_err(DukaError::Spanned)?;
    let mut chunk = Parser::parse(stream, Default::default()).map_err(DukaError::Spanned)?;
    let scope_pass = ScopeAnalyzer.chain(BasicAnalyzer);
    let (d0, e0) = scope_pass.analyze(&chunk, Default::default());
    let (cfg, mut analysis) = d0;
    let prelude_errs = inject_type_prelude(&mut analysis);
    let (data, rest) = TypeEval.analyze(&chunk, (cfg, analysis));
    let (_data, e1) = TypeChecker.analyze(&chunk, data);

    let errs: Vec<_> = e0
        .chain(prelude_errs)
        .chain(rest)
        .chain(e1)
        .filter(|v| v.level == DukaErrorLevel::Error)
        .collect();
    if !errs.is_empty() {
        return Err(DukaError::Analysis(errs));
    }

    Adapter.adapt(&mut chunk);
    Ok(chunk)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DukaError {
    Spanned(DukaSpannedError),
    Analysis(Vec<DukaSpannedError>),
    Generator(DukaIRError),
    Codegen(DukaCodegenError),
    RuntimeTrace(DukaTraceError),
    Runtime(DukaRuntimeError),
}

impl Display for DukaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DukaError::Spanned(e) => write!(f, "{e}"),
            DukaError::Analysis(es) => write!(
                f,
                "{}",
                es.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            DukaError::Generator(e) => write!(f, "{e}"),
            DukaError::Codegen(e) => write!(f, "{e}"),
            DukaError::RuntimeTrace(e) => write!(f, "{e}"),
            DukaError::Runtime(e) => write!(f, "{e}"),
        }
    }
}

pub fn to_ir(src: &str) -> Result<DukaIR, DukaError> {
    let chunk = to_chunk(src)?;
    IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(DukaError::Generator)
}

pub fn run(src: &str) -> Result<Box<[RuntimeValue]>, DukaError> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(DukaError::Codegen)?;
    VM::run(&proto).map_err(DukaError::RuntimeTrace)
}

pub fn run_last(src: &str) -> Result<RuntimeValue, DukaError> {
    Ok(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
}

pub fn run_with_input(src: &str, input: &[u8]) -> Result<Box<[RuntimeValue]>, DukaError> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(DukaError::Codegen)?;
    let mut vm = VM::new(duka_gc::Heap::new());
    let cell: InputCell = Arc::new(Mutex::new(input.to_vec()));
    vm.set_input(Some(cell));
    let count = vm.execute(&proto).map_err(DukaError::RuntimeTrace)?;
    vm.main_coroutine_mut()
        .inner
        .take_stack_many(0, count)
        .map_err(DukaError::Runtime)
}

pub fn run_results(src: &str) -> Result<Vec<RuntimeValue>, DukaError> {
    Ok(run(src)?.to_vec())
}
