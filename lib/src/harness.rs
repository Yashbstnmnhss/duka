//! Used for tests

use std::error::Error;
use std::fmt::Display;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use duka_backend::DukaVM;
use duka_backend::builtin::require::default_vm_globals;
use duka_backend::codegen::errors::DukaCodegenError;
use duka_backend::errors::{DukaRuntimeError, DukaTraceError};
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_backend::vm::coroutine::{InputCell, VmGlobalsHandle};
use duka_frontend::analyzer::modules::DukaSourceProvider;
use duka_frontend::analyzer::{AnalyzerData, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::expander::{BangExpanderError, BangExpanderRegistry};
use duka_frontend::lexer::Lexer;
use duka_frontend::lexer::macros::LexerMacroRegistry;
use duka_frontend::parser::Parser;
use duka_frontend::parser::ast::DukaChunk;
use duka_shared::config::{DukaIRConfig, DukaParsingConfig};
use duka_shared::errors::{DukaDiagnose, DukaIRError, DukaSpannedError};
use duka_shared::ir::DukaIR;
use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser, SourceName};
use duka_shared::utils::SymbolTable;

use crate::module::FileModuleSourceProvider;

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
    crate::prelude::inject(&mut analysis, &Default::default());
    Ok(analysis.symbols)
}

/// What a run is given beyond the source.
///
/// A `RequireType("m")` is resolved by asking the provider, so a harness that
/// cannot hand one over cannot check anything that crosses a module boundary.
pub struct Setup<'a, 'b, 'c> {
    pub provider: Option<&'a dyn DukaSourceProvider>,
    pub bangs: Option<&'b BangExpanderRegistry>,
    pub macros: Option<&'c LexerMacroRegistry>,
    pub vm_state: VmGlobalsHandle,
    pub parsing_config: DukaParsingConfig,
    pub ir_config: DukaIRConfig,
}

impl Default for Setup<'_, '_, '_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, 'b, 'c> Setup<'a, 'b, 'c> {
    pub fn new() -> Self {
        Self {
            provider: None,
            vm_state: default_vm_globals(),
            bangs: None,
            macros: None,
            parsing_config: Default::default(),
            ir_config: Default::default(),
        }
    }

    pub fn bangs(mut self, bangs: &'b BangExpanderRegistry) -> Self {
        self.bangs = Some(bangs);
        self
    }

    pub fn macros(mut self, macros: &'c LexerMacroRegistry) -> Self {
        self.macros = Some(macros);
        self
    }

    pub fn provider(mut self, provider: &'a dyn DukaSourceProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn vm_state(mut self, vm_state: VmGlobalsHandle) -> Self {
        self.vm_state = vm_state;
        self
    }

    pub fn chunk(
        &self,
        src: &str,
        name: SourceName,
    ) -> Result<(DukaChunk, AnalyzerData), DukaError> {
        let default_macros = Default::default();
        let default_bangs = Default::default();

        let mut chunk = crate::module::parse_source(
            src,
            name,
            &self.parsing_config,
            self.macros.unwrap_or(&default_macros),
            self.bangs.unwrap_or(&default_bangs),
        )?;
        let provider = FileModuleSourceProvider::for_entry(chunk.source_info.name.path());
        let data =
            crate::module::analyze_chunk(&mut chunk, &self.parsing_config, &provider, None, None)?;

        Ok((chunk, data))
    }

    pub fn ir(&self, src: &str, name: SourceName) -> Result<(DukaIR, AnalyzerData), DukaError> {
        let (chunk, data) = self.chunk(src, name)?;
        let globals = data.1.symbols.global_names();
        Ok((
            crate::module::gen_ir(chunk, &self.ir_config, globals)?,
            data,
        ))
    }

    pub fn run(&self, src: &str, name: SourceName) -> Result<Box<[RuntimeValue]>, DukaError> {
        let (ir, data) = self.ir(src, name)?;
        let proto = crate::module::gen_proto(ir, Some(data))?;
        VM::run_from(&proto, None, Some(self.vm_state.clone())).map_err(DukaError::RuntimeTrace)
    }

    pub fn run_last(&self, src: &str, name: SourceName) -> Result<RuntimeValue, DukaError> {
        Ok(self
            .run(src, name)?
            .last()
            .cloned()
            .unwrap_or(RuntimeValue::Nil))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DukaError {
    Spanned(DukaSpannedError),
    Analysis(Vec<DukaDiagnose>),
    Generator(DukaIRError),
    Codegen(DukaCodegenError),
    RuntimeTrace(DukaTraceError),
    Runtime(DukaRuntimeError),
    Expander(BangExpanderError),
}

impl Error for DukaError {}

impl Display for DukaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DukaError::Expander(e) => write!(f, "{e}"),
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

#[inline]
pub fn simple_to_ir(src: &str) -> Result<DukaIR, DukaError> {
    Setup::new().ir(src, SourceName::Unnamed).map(|i| i.0)
}

#[inline]
pub fn simple_run(src: &str) -> Result<Box<[RuntimeValue]>, DukaError> {
    Setup::new().run(src, SourceName::Unnamed)
}

#[inline]
pub fn simple_run_with(
    src: &str,
    vm_state: VmGlobalsHandle,
) -> Result<Box<[RuntimeValue]>, DukaError> {
    Setup::new()
        .vm_state(vm_state)
        .run(src, SourceName::Unnamed)
}

#[inline]
/// Run a chunk from start to finish and keep only what the last `return`
/// produced, a chunk that returns nothing reads as `nil`
///
/// ```
/// use duka_lib::{harness::simple_run_last, value::RuntimeValue};
///
/// let src = r#"
/// local total = 0
/// for i = 1, 4 do total = total + i end
/// return total
/// "#;
///
/// assert_eq!(simple_run_last(src).unwrap(), RuntimeValue::Int(10));
/// assert_eq!(simple_run_last("").unwrap(), RuntimeValue::Nil);
/// ```
pub fn simple_run_last(src: &str) -> Result<RuntimeValue, DukaError> {
    simple_run_last_with(src, default_vm_globals())
}

#[inline]
pub fn simple_run_last_with(
    src: &str,
    vm_state: VmGlobalsHandle,
) -> Result<RuntimeValue, DukaError> {
    Setup::new()
        .vm_state(vm_state)
        .run_last(src, SourceName::Unnamed)
}

pub fn simple_run_with_input(src: &str, input: &[u8]) -> Result<Box<[RuntimeValue]>, DukaError> {
    let ir = simple_to_ir(src)?;
    let proto = crate::module::gen_proto(ir, None)?;
    let mut vm = VM::new(duka_gc::Heap::new());
    let cell: InputCell = Arc::new(Mutex::new(input.to_vec()));
    vm.set_input(Some(cell));
    let count = vm.execute(&proto).map_err(DukaError::RuntimeTrace)?;
    let values = vm
        .main_coroutine_mut()
        .inner
        .take_stack_many(0, count)
        .map_err(DukaError::Runtime);
    std::mem::forget(vm);
    values
}

#[inline]
pub fn simple_run_results(src: &str) -> Result<Vec<RuntimeValue>, DukaError> {
    Ok(simple_run(src)?.to_vec())
}
