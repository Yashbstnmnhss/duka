//! YES THIS IS A PIPELINE-LIKE THING
//!
//!
//!
//!
//!

use std::{
    any::{Any, TypeId},
    fmt::Display,
    fs::File,
    io::{self, Read, Write},
    marker::PhantomData,
    path::PathBuf,
};

use anyhow::anyhow;
use duka_backend::{codegen::binary::Dumplings, value::DukaProto};
use duka_pipeline::{Converter, Node};
use duka_shared::{
    types::{DukaAdapter, DukaAnalyzer, DukaChunk, DukaGenerator, DukaLexer, DukaParser},
    utils::OrError,
};
// use serde::{Deserialize, Serialize};

// pub struct ToJsonConverter<T: Serialize>(PhantomData<T>);
// impl<T: Serialize + 'static> Converter for ToJsonConverter<T> {
//     fn from(&self) -> TypeId {
//         TypeId::of::<T>()
//     }
//     fn to(&self) -> TypeId {
//         TypeId::of::<Vec<u8>>()
//     }
//     fn convert(&self, from: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
//         let from = downcast::<T>(from)?;
//         let bytes = serde_json::to_vec(&*from)?;
//         Ok(Box::new(bytes))
//     }
// }

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

converter!(ChunkToBytes, DukaChunk as Vec<u8>, (from) {
    let bytes = serde_json::to_vec(&*from)?;
    Ok(Box::new(bytes))
});
converter!(ProtoToBytes, DukaProto as Vec<u8>, (from) {
    let mut output = vec![];
    from.dl_write(&mut output)?;
    Ok(Box::new(output))
});

fn downcast<T: 'static>(input: Box<dyn Any>) -> anyhow::Result<Box<T>> {
    input
        .downcast::<T>()
        .map_err(|_| anyhow!("Failed to convert type"))
}

pub struct OutNode(Option<PathBuf>);
impl OutNode {
    pub fn from(path: Option<PathBuf>) -> Self {
        Self(path)
    }
}
impl Node for OutNode {
    fn from(&self) -> TypeId {
        TypeId::of::<Vec<u8>>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<Vec<u8>>()
    }
    fn name(&self) -> &'static str {
        "output"
    }
    fn process(&mut self, val: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let buf = *downcast::<Vec<u8>>(val)?;
        if let Some(ref path) = self.0 {
            File::create(path)?.write(&buf)?;
        } else {
            io::stdout().write(&buf)?;
        }
        Ok(Box::new(buf))
    }
}

pub struct FileNode;
impl Node for FileNode {
    fn from(&self) -> TypeId {
        TypeId::of::<PathBuf>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<File>()
    }
    fn name(&self) -> &'static str {
        "file"
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = downcast::<PathBuf>(input)?;
        let file = File::open(*input)?;
        Ok(Box::new(file))
    }
}

pub struct LexerNode<Source: Read, L: DukaLexer<Source>>(PhantomData<(Source, L)>);
impl<Source: Read, L: DukaLexer<Source>> LexerNode<Source, L> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<Source: Read + 'static, L: DukaLexer<Source> + 'static> Node for LexerNode<Source, L> {
    fn from(&self) -> std::any::TypeId {
        TypeId::of::<Source>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<L>()
    }
    fn name(&self) -> &'static str {
        "lexer"
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let input = downcast::<Source>(input)?;
        Ok(Box::new(L::from_source(*input)))
    }
}
pub struct ParserNode<S: Read, L: DukaLexer<S>, P: DukaParser<S, L>>(PhantomData<(S, L, P)>);
impl<S: Read, L: DukaLexer<S>, P: DukaParser<S, L>> ParserNode<S, L, P> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<C: 'static, S: Read, L: DukaLexer<S> + 'static, P: DukaParser<S, L, ChunkType = C>> Node
    for ParserNode<S, L, P>
{
    fn from(&self) -> TypeId {
        TypeId::of::<L>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }

    fn name(&self) -> &'static str {
        "parser"
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let input = downcast::<L>(input)?;
        Ok(Box::new(P::from_lexer(*input).parse()?))
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

impl<C: 'static, A: DukaAnalyzer<InputType = C>> Node for AnalyzerNode<A> {
    fn from(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn name(&self) -> &'static str {
        "analyzer"
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let input = downcast::<C>(input)?;
        errors2one(self.0.analyze(&*input).collect())?;
        Ok(input)
    }
}
impl<C: 'static, A: DukaAdapter<InputType = C>> Node for AdapterNode<A> {
    fn from(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<C>()
    }
    fn name(&self) -> &'static str {
        "adapter"
    }
    fn process(&mut self, input: Box<dyn std::any::Any>) -> anyhow::Result<Box<dyn std::any::Any>> {
        let mut input = *downcast::<C>(input)?;
        self.0.adapt(&mut input);
        Ok(Box::new(input))
    }
}

pub struct CodegenNode<G: DukaGenerator<O>, O>(PhantomData<(G, O)>);
impl<G: DukaGenerator<O>, O> CodegenNode<G, O> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<G: DukaGenerator<O> + 'static, O: 'static> Node for CodegenNode<G, O> {
    fn from(&self) -> TypeId {
        TypeId::of::<G::InputType>()
    }
    fn to(&self) -> TypeId {
        TypeId::of::<O>()
    }

    fn name(&self) -> &'static str {
        "compiler"
    }
    fn process(&mut self, input: Box<dyn Any>) -> anyhow::Result<Box<dyn Any>> {
        let input = downcast::<G::InputType>(input)?;
        let cg = G::new();
        Ok(Box::new(cg.generate(*input)?))
    }
}
