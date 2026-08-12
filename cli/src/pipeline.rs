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
    path::PathBuf,
};

use crate::StepName;
use duka_backend::{codegen::binary::Load, value::RuntimeValue};
use duka_backend::{
    codegen::binary::{DukaBinary, Dump},
    value::DukaProto,
    vm::VM,
};
use duka_frontend::{
    lexer::{Lexer, LexerWithMacro, token::Token},
    parser::ast::DukaChunk,
};
use duka_pipeline::{Converter, Node};
use duka_shared::{
    config::{DukaAnalyzerConfig, DukaParserConfig},
    errors::{DukaErrorKind, DukaSpannedError, Span},
    ir::DukaIR,
    types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser, TokenStream},
    utils::OrError,
};
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
        name: from.path.to_str().map(|v| v.to_owned())
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
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
#[diagnostic()]
pub struct DukaSpannedDiagnose {
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
pub(crate) fn to_diagnose(err: DukaSpannedError) -> DukaSpannedDiagnose {
    let info = err.source_info;
    let code = String::from_utf8(info.source.to_vec()).unwrap();
    let span = span_to_source_span(code.as_str(), err.span);
    let relates = err
        .related
        .into_iter()
        .map(|(label, span)| LabeledSpan::at(span_to_source_span(code.as_str(), span), label))
        .collect();
    DukaSpannedDiagnose {
        source_code: NamedSource::new(
            info.as_ref().name.clone().unwrap_or("<UNNAMED>".into()),
            code,
        )
        .with_language("duka"),
        span,
        related_spans: relates,
        help: err.kind.get_help(),
        source: err.kind,
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
            Lexer::<RawReader>::from_source(input.reader, input.name, Default::default())
                .tokenize()
                .map_err(to_diagnose)?,
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
            LexerWithMacro::<RawReader>::from_source(input.reader, input.name, Default::default())
                .tokenize()
                .map_err(to_diagnose)?,
        ))
    }
}

pub struct ParserNode<P: DukaParser<Token>>(DukaParserConfig, PhantomData<P>);
impl<P: DukaParser<Token>> ParserNode<P> {
    pub const fn new(config: DukaParserConfig) -> Self {
        Self(config, PhantomData)
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
        Ok(Box::new(
            P::parse(*input, self.0.clone()).map_err(to_diagnose)?,
        ))
    }
}

pub struct AnalyzerNode<A: DukaAnalyzer>(A, DukaAnalyzerConfig);
pub struct AdapterNode<A: DukaAdapter>(A);

impl<A: DukaAnalyzer> AnalyzerNode<A> {
    pub const fn new(a: A, c: DukaAnalyzerConfig) -> Self {
        Self(a, c)
    }
}
impl<A: DukaAdapter> AdapterNode<A> {
    pub const fn new(a: A) -> Self {
        Self(a)
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
        let errors: Vec<_> = self
            .0
            .analyze(&*input, self.1.clone())
            .1
            .map(to_diagnose)
            .collect();
        (!errors.is_empty()).then_error(|| DukaSpannedDiagnoses { relates: errors })?;
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

impl<C: 'static, A: DukaAdapter<InputType = C>> Node<StepName> for AdapterNode<A> {
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
        self.0.adapt(&mut input);
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

pub struct RunNode;

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
        let vals = VM::run(&proto).into_diagnostic()?.into_vec();
        Ok(Box::new(vals))
    }
}
