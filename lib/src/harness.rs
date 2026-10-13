//! Used for tests

use std::fmt::Display;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use duka_backend::DukaVM;
use duka_backend::builtin::require::default_vm_globals;
use duka_backend::codegen::DefaultGenerator;
use duka_backend::codegen::errors::DukaCodegenError;
use duka_backend::errors::{DukaRuntimeError, DukaTraceError};
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_backend::vm::coroutine::{InputCell, VmGlobalsHandle};
use duka_frontend::analyzer::modules::DukaSourceProvider;
use duka_frontend::analyzer::{BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_frontend::parser::ast::DukaChunk;
use duka_shared::config::DukaIRConfig;
use duka_shared::errors::{DukaErrorLevel, DukaIRError, DukaSpannedError};
use duka_shared::ir::DukaIR;
use duka_shared::types::{DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser, SourceName};
use duka_shared::utils::SymbolTable;

/// The symbol table as the editor sees it.
///
/// Stops after scope analysis, because that is where the table is filled and what
/// the language server reads: `hover` and `goto` resolve against
/// `analysis.scope.symbols`, not against the type checker's frames.
///
/// The prelude is injected, because the editor's copy of this table has it: without
/// it a name a builtin owns looks undeclared here and in the editor alike.
pub fn scope_symbols(src: &str) -> Result<SymbolTable, DukaError> {
    let lexer = Lexer::new(Cursor::new(src), SourceName::Unnamed, Default::default());
    let stream = lexer.tokenize().map_err(DukaError::Spanned)?;
    let chunk = Parser::parse(stream, Default::default()).map_err(DukaError::Spanned)?;
    let scope_pass = ScopeAnalyzer.chain(BasicAnalyzer);
    let (data, _reports) = scope_pass.analyze(&chunk, Default::default());
    let (_cfg, mut analysis) = data;
    crate::prelude::inject(&mut analysis);
    Ok(analysis.symbols)
}

/// What a run is given beyond the source.
///
/// A `RequireType("m")` is resolved by asking the provider, so a harness that
/// cannot hand one over cannot check anything that crosses a module boundary.
pub struct Setup<'a> {
    pub provider: Option<&'a dyn DukaSourceProvider>,
    pub vm_state: VmGlobalsHandle,
}

impl Default for Setup<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Setup<'a> {
    pub fn new() -> Self {
        Self {
            provider: None,
            vm_state: default_vm_globals(),
        }
    }

    pub fn provider(mut self, provider: &'a dyn DukaSourceProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn vm_state(mut self, vm_state: VmGlobalsHandle) -> Self {
        self.vm_state = vm_state;
        self
    }

    pub fn chunk(&self, src: &str) -> Result<DukaChunk, DukaError> {
        let lexer = Lexer::new(Cursor::new(src), SourceName::Unnamed, Default::default());
        let stream = lexer.tokenize().map_err(DukaError::Spanned)?;
        let mut chunk = Parser::parse(stream, Default::default()).map_err(DukaError::Spanned)?;

        // the same pipeline every other caller runs -- a harness that checked a
        // different way would pass tests the compiler would not. An unnamed
        // source has no file behind it to require from, which is what a provider
        // for no entry says
        let fallback;
        let provider = match self.provider {
            Some(provider) => provider,
            None => {
                fallback = crate::module::FileModuleSourceProvider::for_entry(None);
                &fallback
            }
        };
        let (_data, errs) = crate::module::Check::new(provider).run(&mut chunk);
        let errs: Vec<_> = errs
            .into_iter()
            .filter(|v| v.level == DukaErrorLevel::Error)
            .collect();
        if !errs.is_empty() {
            return Err(DukaError::Analysis(errs));
        }

        Ok(chunk)
    }

    pub fn ir(&self, src: &str) -> Result<DukaIR, DukaError> {
        let chunk = self.chunk(src)?;
        IRGenerator::generate(
            chunk,
            DukaIRConfig {
                var_default_local: false,
                ..DukaIRConfig::default()
            },
        )
        .map_err(DukaError::Generator)
    }

    pub fn run(&self, src: &str) -> Result<Box<[RuntimeValue]>, DukaError> {
        let proto = DefaultGenerator::generate(self.ir(src)?, ()).map_err(DukaError::Codegen)?;
        VM::run_from(&proto, None, Some(self.vm_state.clone())).map_err(DukaError::RuntimeTrace)
    }

    pub fn run_last(&self, src: &str) -> Result<RuntimeValue, DukaError> {
        Ok(self.run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
    }
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
    Setup::new().ir(src)
}

pub fn run(src: &str) -> Result<Box<[RuntimeValue]>, DukaError> {
    Setup::new().run(src)
}

/// Run with per-VM state the caller already set up, e.g. a module loader.
pub fn run_with(src: &str, vm_state: VmGlobalsHandle) -> Result<Box<[RuntimeValue]>, DukaError> {
    Setup::new().vm_state(vm_state).run(src)
}

/// Run a chunk from start to finish and keep only what the last `return`
/// produced, a chunk that returns nothing reads as `nil`
///
/// ```
/// use duka_lib::{harness::run_last, value::RuntimeValue};
///
/// let src = r#"
/// local total = 0
/// for i = 1, 4 do total = total + i end
/// return total
/// "#;
///
/// assert_eq!(run_last(src).unwrap(), RuntimeValue::Int(10));
/// assert_eq!(run_last("").unwrap(), RuntimeValue::Nil);
/// ```
pub fn run_last(src: &str) -> Result<RuntimeValue, DukaError> {
    run_last_with(src, default_vm_globals())
}

pub fn run_last_with(src: &str, vm_state: VmGlobalsHandle) -> Result<RuntimeValue, DukaError> {
    Setup::new().vm_state(vm_state).run_last(src)
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
