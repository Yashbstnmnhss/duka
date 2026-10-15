#![allow(unused_assignments)] //IDK WHY THIS HAPPENED
//! YES THIS IS A PIPELINE-LIKE THING
//!
//!
//!
//!
//!
//!

use std::{
    any::{Any, TypeId},
    error::Error,
    fs::File,
    io::{self, BufReader, Read, Write},
    marker::PhantomData,
    path::{Path, PathBuf},
};

use crate::StepName;
use duka_lib::{codegen::binary::Load, module::FileModuleSourceProvider, value::RuntimeValue};
use duka_lib::{
    codegen::binary::{DukaBinary, Dump},
    value::DukaProto,
    vm::{VM, coroutine::VmGlobalsHandle},
};
use duka_lib::{
    duka_frontend::analyzer::AdaptedData,
    duka_shared::{
        config::{DukaAnalyzerConfig, DukaConfig, DukaLexerConfig, DukaParserConfig},
        errors::{DukaDiagnose, DukaErrorKind, DukaSpannedError, Severity, Span},
        ir::DukaIR,
        types::{
            DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser, SourceName,
            TokenStream,
        },
        utils::OrError,
    },
};
use duka_lib::{
    duka_frontend::{
        lexer::{Lexer, LexerWithMacro, token::Token},
        parser::{Parser, ast::DukaChunk},
    },
    duka_shared::errors::DukaErrorLevel,
};
use duka_pipeline::{Converter, Node};
use miette::{
    Diagnostic, IntoDiagnostic, LabeledSpan, NamedSource, SourceOffset, SourceSpan, miette,
};
use thiserror::Error;

macro_rules! converter {
    ($name: ident, $from: ty as $to: ty, ($($n: tt)+) $do: block) => {
        pub struct $name;
        impl Converter for $name {
            fn convert(&self, from: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
                let $($n)+ = downcast::<$from>(from)?;
                $do
            }
            fn from(&self) -> TypeId {
                TypeId::of::<$from>()
            }
            fn to(&self) -> TypeId {
                TypeId::of::<$to>()
            }
        }
    };
}

converter!(FileToChunk, DFile as DukaChunk, (from) {
    let chunk: DukaChunk = serde_json::from_reader(from.file).into_diagnostic()?;
    Ok(Box::new(chunk))
});
converter!(FileToProto, DFile as DukaProto, (mut from) {
    let binary = DukaBinary::load(&mut from.file).into_diagnostic()?;
    Ok(Box::new(binary.into_proto()))
});
converter!(FileToIR, DFile as DukaIR, (from) {
    let chunk: DukaIR = serde_json::from_reader(from.file).into_diagnostic()?;
    Ok(Box::new(chunk))
});

converter!(TokensToBytes, TokenStream<Token> as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&*from).into_diagnostic()?;
    Ok(Box::new(bytes))
});

converter!(ChunkToBytes, DukaChunk as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&*from).into_diagnostic()?;
    Ok(Box::new(bytes))
});
converter!(ProtoToBytes, DukaProto as Vec<u8>, (from) {
    let mut output = vec![];
    let binary = DukaBinary::new(*from);
    binary.dump(&mut output).into_diagnostic()?;
    Ok(Box::new(output))
});
converter!(IRToBytes, DukaIR as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&*from).into_diagnostic()?;
    Ok(Box::new(bytes))
});

converter!(ResultsToBytes, Vec<RuntimeValue> as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&format!("{from:?}")).into_diagnostic()?;
    Ok(Box::new(bytes))
});

converter!(StringToBytes, String as Vec<u8>, (from) {
    Ok(Box::new(from.into_bytes()))
});

fn downcast<T: 'static>(input: Box<dyn Any>) -> miette::Result<Box<T>> {
    input
        .downcast::<T>()
        .map_err(|_| miette!("Failed to convert type"))
}

pub struct WriterNode(Option<PathBuf>);
impl WriterNode {
    pub fn to(path: Option<PathBuf>) -> Self {
        Self(path)
    }
}
impl Node<StepName> for WriterNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Vec<u8>>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<Vec<u8>>()
    }
    fn name(&self) -> StepName {
        StepName::Output
    }
    fn process(&mut self, val: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let buf = *downcast::<Vec<u8>>(val)?;
        if let Some(ref path) = self.0 {
            File::create(path)
                .into_diagnostic()?
                .write_all(&buf)
                .into_diagnostic()?;
        } else {
            io::stdout().write_all(&buf).into_diagnostic()?;
        }
        Ok(Box::new(buf))
    }
}

struct DFile {
    file: File,
    path: PathBuf,
}
pub struct FileNode;
impl Node<StepName> for FileNode {
    fn from(&self) -> TypeId {
        TypeId::of::<PathBuf>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<DFile>()
    }
    fn name(&self) -> StepName {
        StepName::File
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = *downcast::<PathBuf>(input)?;
        let file = File::open(&input).map_err(|e| miette!("For path {input:?}").context(e))?;
        Ok(Box::new(DFile { file, path: input }))
    }
}

converter!(FileToTokens, DFile as TokenStream<Token>, (from) {
    let tokens: TokenStream<Token> = serde_json::from_reader(from.file).into_diagnostic()?;
    Ok(Box::new(tokens))
});

converter!(FileToRaw, DFile as Raw, (from) {
    Ok(Box::new(Raw {
        reader: RawReader::BufReader(BufReader::new(from.file)),
        name: from.path.file_name().map(|v| v.to_string_lossy().to_string()),
        path: Some(from.path)
    }))
});

pub enum RawReader {
    BufReader(BufReader<File>),
}
impl Read for RawReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::BufReader(r) => r.read(buf),
        }
    }
}
pub struct Raw {
    reader: RawReader,
    name: Option<String>,
    path: Option<PathBuf>,
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
pub struct DukaSpannedDiagnose {
    severity: miette::Severity,
    #[label(primary, "here")]
    span: SourceSpan,
    #[label(collection, "related to this")]
    related_spans: Vec<LabeledSpan>,
    #[help]
    help: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[source]
    source: DukaErrorKind,
}

/// The same diagnostic at a level that is not a failure.
#[derive(Debug, Diagnostic, Error)]
#[error("Duka warning")]
pub struct DukaSpannedWarn {
    severity: miette::Severity,
    #[label(primary, "here")]
    span: SourceSpan,
    #[label(collection, "related to this")]
    related_spans: Vec<LabeledSpan>,
    #[help]
    help: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[source]
    source: DukaErrorKind,
}

/// And at a level that is neither: worth knowing, nothing to do.
#[derive(Debug, Diagnostic, Error)]
#[error("Duka info")]
pub struct DukaSpannedInfo {
    severity: miette::Severity,
    #[label(primary, "here")]
    span: SourceSpan,
    #[label(collection, "related to this")]
    related_spans: Vec<LabeledSpan>,
    #[help]
    help: String,
    #[source_code]
    source_code: NamedSource<String>,
    #[source]
    source: DukaErrorKind,
}

fn span_to_source_span(code: impl AsRef<str>, span: Span) -> SourceSpan {
    SourceSpan::new(
        SourceOffset::from_location(code, span.start.line as usize, span.start.column as usize),
        span.char_len() as usize,
    )
}

/// The three shapes above are the same fields, so what each one is built from is
/// written once rather than three times.
macro_rules! diagnose_parts {
    ($err:expr) => {{
        let code = String::from_utf8($err.source_info().source.to_vec()).unwrap();
        let span = span_to_source_span(code.as_str(), $err.span());
        let relates = $err
            .related()
            .iter()
            .map(|(label, span)| {
                LabeledSpan::at(span_to_source_span(code.as_str(), *span), label.to_string())
            })
            .collect();
        (
            span,
            relates,
            $err.kind().get_help(),
            NamedSource::new($err.source_info().name.to_string(), code).with_language("duka"),
            $err.kind().clone(),
        )
    }};
}

pub(crate) fn to_diagnose(err: &DukaDiagnose) -> DukaSpannedDiagnose {
    let (span, related_spans, help, source_code, source) = diagnose_parts!(err);
    DukaSpannedDiagnose {
        severity: miette::Severity::Error,
        span,
        related_spans,
        help,
        source_code,
        source,
    }
}

fn to_warn(err: &DukaDiagnose) -> DukaSpannedWarn {
    let (span, related_spans, help, source_code, source) = diagnose_parts!(err);
    DukaSpannedWarn {
        severity: miette::Severity::Warning,
        span,
        related_spans,
        help,
        source_code,
        source,
    }
}

fn to_info(err: &DukaDiagnose) -> DukaSpannedInfo {
    let (span, related_spans, help, source_code, source) = diagnose_parts!(err);
    DukaSpannedInfo {
        severity: miette::Severity::Advice,
        span,
        related_spans,
        help,
        source_code,
        source,
    }
}

/// A stage that only ever fails -- the lexer, the parser -- reports in its own
/// shape rather than as a diagnostic, so it goes through here to reach the same
/// renderer.
pub(crate) fn to_spanned(err: &DukaSpannedError) -> DukaSpannedDiagnose {
    let code = String::from_utf8(err.source_info.source.to_vec()).unwrap();
    let span = span_to_source_span(code.as_str(), err.span);
    let relates = err
        .related
        .iter()
        .map(|(label, span)| {
            LabeledSpan::at(span_to_source_span(code.as_str(), *span), label.to_string())
        })
        .collect();
    DukaSpannedDiagnose {
        severity: miette::Severity::Error,
        span,
        related_spans: relates,
        help: err.kind.get_help(),
        source_code: NamedSource::new(err.source_info.name.to_string(), code).with_language("duka"),
        source: err.kind.clone(),
    }
}

pub struct LexerNode;
impl Node<StepName> for LexerNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Raw>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<TokenStream<Token>>()
    }
    fn name(&self) -> StepName {
        StepName::Lexer
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = *downcast::<Raw>(input)?;
        Ok(Box::new(
            Lexer::<RawReader>::from_source(
                input.reader,
                match (input.name, input.path) {
                    (Some(name), Some(path)) => SourceName::File(name.into(), path.into()),
                    (Some(name), _) => SourceName::Virtual(name.into()),
                    _ => SourceName::Unnamed,
                },
                Default::default(),
            )
            .tokenize()
            .map_err(|e| to_spanned(&e))?,
        ))
    }
}

pub struct MacroLexerNode;
impl Node<StepName> for MacroLexerNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Raw>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<TokenStream<Token>>()
    }
    fn name(&self) -> StepName {
        StepName::MacroLexer
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = *downcast::<Raw>(input)?;
        Ok(Box::new(
            LexerWithMacro::<RawReader>::from_source(
                input.reader,
                match (input.name, input.path) {
                    (Some(name), Some(path)) => SourceName::File(name.into(), path.into()),
                    (Some(name), _) => SourceName::Virtual(name.into()),
                    _ => SourceName::Unnamed,
                },
                Default::default(),
            )
            .tokenize()
            .map_err(|e| to_spanned(&e))?,
        ))
    }
}

/// The bang names the expander stage answers to. The parser collects a `!`
/// body before that stage runs, so it has to be told which ones exist.
pub struct ParserNode<P: DukaParser<Token>>(
    DukaParserConfig,
    duka_lib::duka_frontend::expander::BangNames,
    PhantomData<P>,
);
impl<P: DukaParser<Token>> ParserNode<P> {
    pub const fn new(
        config: DukaParserConfig,
        bangs: duka_lib::duka_frontend::expander::BangNames,
    ) -> Self {
        Self(config, bangs, PhantomData)
    }
}
impl<C: 'static, P: DukaParser<Token, ChunkType = C>> Node<StepName> for ParserNode<P> {
    fn from(&self) -> TypeId {
        TypeId::of::<TokenStream<Token>>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn name(&self) -> StepName {
        StepName::Parser
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = downcast::<TokenStream<Token>>(input)?;
        let parser = self
            .1
            .register_with(Parser::<Token>::from_tokens(*input, self.0.clone()));
        Ok(Box::new(
            parser.parse_into_chunk().map_err(|e| to_spanned(&e))?,
        ))
    }
}

/// The type stage, and the only node holding a source provider: reading a
/// dependency needs a lexer and parser config, and resolving a relative
/// `RequireType` needs the provider. Mirrors `duka_lib::module::from_source`.
pub struct TypeCheckerNode {
    analyzer: DukaAnalyzerConfig,
    lexer: DukaLexerConfig,
    parser: DukaParserConfig,
    adapted: AdaptedSlot,
}
impl TypeCheckerNode {
    pub fn new(
        analyzer: DukaAnalyzerConfig,
        lexer: DukaLexerConfig,
        parser: DukaParserConfig,
        adapted: AdaptedSlot,
    ) -> Self {
        Self {
            analyzer,
            lexer,
            parser,
            adapted,
        }
    }
}
impl Node<StepName> for TypeCheckerNode {
    fn from(&self) -> TypeId {
        TypeId::of::<DukaChunk>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<DukaChunk>()
    }
    fn name(&self) -> StepName {
        StepName::TypeChecker
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let mut chunk = *downcast::<DukaChunk>(input)?;
        // a relative `require` resolves against the entry file's directory,
        // the only path the chunk records
        let provider = FileModuleSourceProvider::for_entry(chunk.source_info.name.path());

        let (_data, all) = duka_lib::module::analyze_and_check(
            &mut chunk,
            &DukaConfig {
                analyzer: self.analyzer.clone(),
                lexer: self.lexer.clone(),
                parser: self.parser.clone(),
                ..Default::default()
            },
            &provider,
            None,
            self.adapted.take(),
        );

        // The three levels are drawn by three renderers that share one shape, so
        // a warning reads like an error with a different word on it rather than a
        // different format. Only errors stop the build.
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut infos = Vec::new();
        for diagnose in all {
            match diagnose.severity() {
                Severity::Error => errors.push(diagnose),
                Severity::Warning => warnings.push(diagnose),
                Severity::Info => infos.push(diagnose),
            }
        }
        if !errors.is_empty() {
            return Err(miette::Report::new(DukaSpannedDiagnoses {
                relates: errors.iter().map(to_diagnose).collect(),
            })
            .into());
        }
        for warning in &warnings {
            println!("{:?}", to_warn(warning));
        }
        for info in &infos {
            println!("{:?}", to_info(info));
        }

        Ok(Box::new(chunk))
    }
}

pub struct AnalyzerNode<A: DukaAnalyzer>(A, DukaAnalyzerConfig);
pub struct AdapterNode<A: DukaAdapter>(A, AdaptedSlot);

/// What the adapter handed back, parked for the type stage to read.
///
/// The adapter's data names the `object` and `export` sugar it unwrote, and the
/// analyzer has to be told about those before it walks a tree that no longer
/// carries them. Every step downstream of the adapter only ever sees the chunk,
/// so this travels beside the payload rather than inside it.
#[derive(Default, Clone)]
pub struct AdaptedSlot(std::rc::Rc<std::cell::RefCell<Option<AdaptedData>>>);

impl AdaptedSlot {
    fn put(&self, adapted: AdaptedData) {
        *self.0.borrow_mut() = Some(adapted);
    }
    fn take(&self) -> Option<AdaptedData> {
        self.0.borrow_mut().take()
    }
}

impl<A: DukaAnalyzer> AnalyzerNode<A> {
    pub const fn new(a: A, c: DukaAnalyzerConfig) -> Self {
        Self(a, c)
    }
}
impl<A: DukaAdapter> AdapterNode<A> {
    pub fn new(a: A) -> (Self, AdaptedSlot) {
        let slot = AdaptedSlot::default();
        (Self(a, slot.clone()), slot)
    }
}

impl<C: 'static, A: DukaAnalyzer<InputType = C, InputData = DukaAnalyzerConfig>> Node<StepName>
    for AnalyzerNode<A>
{
    fn from(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn name(&self) -> StepName {
        StepName::Analyzer
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = downcast::<C>(input)?;
        let (errors, warnings): (Vec<_>, Vec<_>) = self
            .0
            .analyze(&*input, self.1.clone())
            .1
            .partition(|i| i.level == DukaErrorLevel::Error);
        (!errors.is_empty()).then_error(|| DukaSpannedDiagnoses {
            relates: errors.iter().map(to_spanned).collect(),
        })?;

        for warning in warnings {
            println!("Warning {:?}", to_spanned(&warning));
        }

        Ok(input)
    }
}

#[derive(Debug, Error, Diagnostic)]
#[diagnostic()]
#[error("Duka errors")]
pub struct DukaSpannedDiagnoses {
    #[related]
    pub relates: Vec<DukaSpannedDiagnose>,
}

impl<C: 'static, A: DukaAdapter<InputType = C, OutputData = AdaptedData>> Node<StepName>
    for AdapterNode<A>
{
    fn from(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn name(&self) -> StepName {
        StepName::Adapter
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let mut input = *downcast::<C>(input)?;
        let (adapted, errs) = self.0.adapt(&mut input);
        self.1.put(adapted);
        let (errors, warnings): (Vec<_>, Vec<_>) = errs
            .collect::<Vec<_>>()
            .into_iter()
            .partition(|i| i.level == DukaErrorLevel::Error);
        (!errors.is_empty()).then_error(|| DukaSpannedDiagnoses {
            relates: errors.iter().map(to_spanned).collect(),
        })?;

        for warning in warnings {
            println!("Warning {:?}", to_spanned(&warning));
        }
        Ok(Box::new(input))
    }
}

pub struct CodegenNode<G: DukaGenerator<O, E>, O, E>(
    StepName,
    G::ConfigType,
    PhantomData<(G, O, E)>,
)
where
    G::ConfigType: Clone;
impl<G: DukaGenerator<O, E>, O, E> CodegenNode<G, O, E>
where
    G::ConfigType: Clone,
{
    pub const fn new(name: StepName, config: G::ConfigType) -> Self {
        Self(name, config, PhantomData)
    }
}
impl<G: DukaGenerator<O, E> + 'static, O: 'static, E: 'static + Error + Send + Sync> Node<StepName>
    for CodegenNode<G, O, E>
where
    G::ConfigType: Clone,
{
    fn from(&self) -> TypeId {
        TypeId::of::<G::InputType>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<O>()
    }

    fn name(&self) -> StepName {
        self.0
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let input = downcast::<G::InputType>(input)?;
        Ok(Box::new(
            G::generate(*input, self.1.clone()).into_diagnostic()?,
        ))
    }
}

/// Carries the entry file down to the executor, which is what a relative
/// `require` in the entry chunk resolves against.
pub struct RunNode(Option<PathBuf>, VmGlobalsHandle);

impl RunNode {
    pub fn from(path: &Path, vm_state: VmGlobalsHandle) -> Self {
        Self(Some(path.to_path_buf()), vm_state)
    }
}

impl Node<StepName> for RunNode {
    fn from(&self) -> TypeId {
        TypeId::of::<DukaProto>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<Vec<RuntimeValue>>()
    }
    fn name(&self) -> StepName {
        StepName::Executor
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let proto = *downcast::<DukaProto>(input)?;
        let vals = VM::run_from(&proto, self.0.clone(), Some(self.1.clone()))
            .into_diagnostic()?
            .into_vec();
        Ok(Box::new(vals))
    }
}

pub struct BangExpanderNode(pub duka_lib::duka_frontend::expander::BangExpanderRegistry);

impl Node<StepName> for BangExpanderNode {
    fn from(&self) -> TypeId {
        TypeId::of::<DukaChunk>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<DukaChunk>()
    }
    fn name(&self) -> StepName {
        StepName::BangExpander
    }
    fn process(&mut self, input: Box<dyn Any>) -> miette::Result<Box<dyn Any>> {
        let mut input = *downcast::<DukaChunk>(input)?;
        self.0
            .expand_chunk(&mut input)
            .map_err(|e| miette::miette!("{e}"))?;
        Ok(Box::new(input))
    }
}
