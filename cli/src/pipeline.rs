//! YES THIS IS A PIPELINE-LIKE THING
//!
//!
//!
//!
//!

use std::{
    any::{Any, TypeId},
    collections::VecDeque,
    fmt::Display,
    fs::File,
    io::{self, BufReader, Read, Write},
    marker::PhantomData,
    path::PathBuf,
};

use anyhow::anyhow;
use duka_backend::{
    codegen::{binary::Dumplings, types::DukaIR},
    value::DukaProto,
};
use duka_frontend::lexer::{Lexer, LexerWithMacro};
use duka_pipeline::{Converter, Node};
use duka_shared::{
    token::Token,
    types::{DukaAdapter, DukaAnalyzer, DukaChunk, DukaGenerator, DukaLexer, DukaParser, RawToken},
    utils::OrError,
};

use crate::StepName;

macro_rules! converter {
    ($name: ident, $from: ty as $to: ty, ($($n: tt)+) $do: block) => {
        pub struct $name;
        impl Converter for $name {
            fn convert(&self, from: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
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

converter!(FileToChunk, File as DukaChunk, (from) {
    let chunk: DukaChunk = serde_json::from_reader(*from)?;
    Ok(Box::new(chunk))
});
converter!(FileToProto, File as DukaProto, (mut from) {
    let chunk = DukaProto::dl_read(&mut *from)?;
    Ok(Box::new(chunk))
});

converter!(TokensToBytes, Tokens as Vec<u8>, (from) {
    let bytes = match *from {
        Tokens::Vec(v) => serde_json::to_vec(&v),
        Tokens::Lexer(l) => serde_json::to_vec(&l.collect::<Result<Vec<_>,_>>()?),
        Tokens::MacroLexer(l) => serde_json::to_vec(&l.collect::<Result<Vec<_>,_>>()?)
    }?;
    Ok(Box::new(bytes))
});

converter!(ChunkToBytes, DukaChunk as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&*from)?;
    Ok(Box::new(bytes))
});
converter!(ProtoToBytes, DukaProto as Vec<u8>, (from) {
    let mut output = vec![];
    from.dl_write(&mut output)?;
    Ok(Box::new(output))
});
converter!(IRToBytes, DukaIR as Vec<u8>, (from) {
    let mut output = vec![];
    write!(output, "{}", from)?;
    Ok(Box::new(output))
});

fn downcast<T: 'static>(input: Box<dyn Any>) -> anyhow::Result<Box<T>> {
    input
        .downcast::<T>()
        .map_err(|_| anyhow!("Failed to convert type"))
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
    fn process(&mut self, val: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let buf = *downcast::<Vec<u8>>(val)?;
        if let Some(ref path) = self.0 {
            File::create(path)?.write_all(&buf)?;
        } else {
            io::stdout().write_all(&buf)?;
        }
        Ok(Box::new(buf))
    }
}

pub struct FileNode;
impl Node<StepName> for FileNode {
    fn from(&self) -> TypeId {
        TypeId::of::<PathBuf>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<File>()
    }
    fn name(&self) -> StepName {
        StepName::File
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = *downcast::<PathBuf>(input)?;
        let file = File::open(&input).map_err(|e| anyhow!("For path {input:?}").context(e))?;
        Ok(Box::new(file))
    }
}

converter!(FileToTokens, File as Tokens, (from) {
    let tokens: VecDeque<Token> = serde_json::from_reader(*from)?;
    Ok(Box::new(Tokens::Vec(tokens)))
});

converter!(FileToRaw, File as Raw, (from) {
    Ok(Box::new(Raw::BufReader(BufReader::new(*from))))
});

pub enum Raw {
    BufReader(BufReader<File>),
}
impl Read for Raw {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::BufReader(r) => r.read(buf),
        }
    }
}

pub struct LexerNode;
impl Node<StepName> for LexerNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Raw>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<Tokens>()
    }
    fn name(&self) -> StepName {
        StepName::Lexer
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = downcast::<Raw>(input)?;
        Ok(Box::new(Tokens::Lexer(Lexer::<Raw>::from_source(*input))))
    }
}

pub struct MacroLexerNode;
impl Node<StepName> for MacroLexerNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Raw>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<Tokens>()
    }
    fn name(&self) -> StepName {
        StepName::MacroLexer
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = downcast::<Raw>(input)?;
        Ok(Box::new(Tokens::MacroLexer(
            LexerWithMacro::<Raw>::from_source(*input),
        )))
    }
}

pub enum Tokens {
    Vec(VecDeque<Token>),
    Lexer(Lexer<Raw>),
    MacroLexer(LexerWithMacro<Raw>),
}
impl Iterator for Tokens {
    type Item = RawToken<Token>;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Lexer(l) => l.next(),
            Self::MacroLexer(l) => l.next(),
            Self::Vec(v) => v.pop_front().map(Ok),
        }
    }
}

pub struct ParserNode<P: DukaParser<Tokens>>(PhantomData<P>);
impl<P: DukaParser<Tokens>> ParserNode<P> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<C: 'static, P: DukaParser<Tokens, ChunkType = C>> Node<StepName> for ParserNode<P> {
    fn from(&self) -> TypeId {
        TypeId::of::<Tokens>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn name(&self) -> StepName {
        StepName::Parser
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let input = downcast::<Tokens>(input)?;
        Ok(Box::new(P::parse(*input)?))
    }
}

pub struct AnalyzerNode<A: DukaAnalyzer>(A);
pub struct AdapterNode<A: DukaAdapter>(A);

impl<A: DukaAnalyzer> AnalyzerNode<A> {
    pub const fn new(a: A) -> Self {
        Self(a)
    }
}
impl<A: DukaAdapter> AdapterNode<A> {
    pub const fn new(a: A) -> Self {
        Self(a)
    }
}

#[inline(always)]
pub(crate) fn errors2one(errors: Vec<impl Send + Sync + Display + 'static>) -> anyhow::Result<()> {
    (!errors.is_empty()).then_error(|| {
        errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
    })
}

impl<C: 'static, A: DukaAnalyzer<InputType = C>> Node<StepName> for AnalyzerNode<A> {
    fn from(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn name(&self) -> StepName {
        StepName::Analyzer
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let input = downcast::<C>(input)?;
        errors2one(self.0.analyze(&*input).collect())?;
        Ok(input)
    }
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
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let mut input = *downcast::<C>(input)?;
        self.0.adapt(&mut input);
        Ok(Box::new(input))
    }
}

pub struct CodegenNode<G: DukaGenerator<O>, O>(StepName, PhantomData<(G, O)>);
impl<G: DukaGenerator<O>, O> CodegenNode<G, O> {
    pub const fn new(name: StepName) -> Self {
        Self(name, PhantomData)
    }
}
impl<G: DukaGenerator<O> + 'static, O: 'static> Node<StepName> for CodegenNode<G, O> {
    fn from(&self) -> TypeId {
        TypeId::of::<G::InputType>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<O>()
    }

    fn name(&self) -> StepName {
        self.0
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = downcast::<G::InputType>(input)?;
        Ok(Box::new(G::generate(*input)?))
    }
}
