//! YES THIS IS A PIPELINE-LIKE THING

use std::{
    borrow::Cow,
    fmt::Display,
    fs::{self, File},
    io::{self, BufReader, Read, Stdout, Write},
    marker::PhantomData,
    path::Path,
};

use anyhow::Error;
use duka_shared::{
    token::Token,
    types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser},
    utils::OrError,
};

pub trait Node: Sized {
    type Input;
    type Output;

    fn name(&self) -> Cow<'static, str>;
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error>;

    fn then<N>(self, next: N) -> Chain<Self, N>
    where
        N: Node<Input = Self::Output>,
        Self: Sized,
    {
        Chain {
            first: self,
            second: next,
        }
    }
}

/// Chain for two [`Node`]
///
/// [`Node`]: Node
pub struct Chain<A, B> {
    first: A,
    second: B,
}

impl<A, B> Node for Chain<A, B>
where
    A: Node,
    B: Node<Input = A::Output>,
{
    type Input = A::Input;
    type Output = B::Output;

    fn name(&self) -> Cow<'static, str> {
        Cow::Owned(format!("{} -> {}", self.first.name(), self.second.name()))
    }
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error> {
        let first = self.first.process(input)?;
        self.second.process(first)
    }
}

pub struct FileInput<P: AsRef<Path>>(P);

impl<P: AsRef<Path>> Node for FileInput<P> {
    type Input = ();
    type Output = BufReader<File>;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("input(file)")
    }
    fn process(&mut self, _: Self::Input) -> Result<Self::Output, Error> {
        Ok(BufReader::new(File::open(&self.0)?))
    }
}

pub struct Writer<W: Write>(W);

impl<W: Write> Node for Writer<W> {
    type Input = Vec<u8>;
    type Output = Self::Input;
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("output")
    }
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error> {
        self.0.write(&input)?;
        Ok(input)
    }
}

impl Writer<Stdout> {
    pub fn stdout() -> Self {
        Self(io::stdout())
    }
}
impl Writer<File> {
    pub fn file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        Ok(Self(File::create(path)?))
    }
}

pub struct PipelineBuilder<N: Node>(N);
impl<P: AsRef<Path>> PipelineBuilder<FileInput<P>> {
    pub fn from_file(path: P) -> Self {
        PipelineBuilder(FileInput(path))
    }
}
impl<T: Node> PipelineBuilder<T> {
    pub fn then<N>(self, next: N) -> PipelineBuilder<Chain<T, N>>
    where
        N: Node<Input = T::Output>,
    {
        PipelineBuilder(self.0.then(next))
    }

    pub fn process(&mut self, input: T::Input) -> Result<T::Output, Error> {
        self.0.process(input)
    }
}

pub struct LexerNode<Source: Read, L: DukaLexer<Source>>(PhantomData<(Source, L)>);
impl<Source: Read, L: DukaLexer<Source>> LexerNode<Source, L> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<Source: Read, L: DukaLexer<Source>> Node for LexerNode<Source, L> {
    type Input = Source;
    type Output = L;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tokenize")
    }
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error> {
        Ok(L::from_source(input))
    }
}

pub struct ParserNode<S: Read, L: DukaLexer<S>, P: DukaParser<S, L>>(PhantomData<(S, L, P)>);
impl<S: Read, L: DukaLexer<S>, P: DukaParser<S, L>> ParserNode<S, L, P> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<C, S: Read, L: DukaLexer<S>, P: DukaParser<S, L, ChunkType = C>> Node for ParserNode<S, L, P> {
    type Input = L;
    type Output = C;

    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("parse")
    }
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error> {
        Ok(P::from_lexer(input).parse()?)
    }
}

pub struct PostprocessNode<A: DukaAnalyzer, B: DukaAdapter>(A, B);

impl<A: DukaAnalyzer, B: DukaAdapter> PostprocessNode<A, B> {
    pub const fn new(a: A, b: B) -> Self {
        Self(a, b)
    }
}

#[inline(always)]
pub(crate) fn errors2one(errors: Vec<impl Send + Sync + Display + 'static>) -> Result<(), Error> {
    (!errors.is_empty()).then_error(|| {
        errors
            .into_iter()
            .fold(anyhow::anyhow!("Errors occurred"), |acc, e| acc.context(e))
    })
}

impl<C, A: DukaAnalyzer<InputType = C>, B: DukaAdapter<InputType = C>> Node
    for PostprocessNode<A, B>
{
    type Input = C;
    type Output = C;
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("analyze-adapt")
    }
    fn process(&mut self, mut input: Self::Input) -> Result<Self::Output, Error> {
        errors2one(self.0.analyze(&input).collect())?;
        self.1.adapt(&mut input);
        Ok(input)
    }
}

pub struct CodegenNode<G: DukaGenerator<O>, O>(PhantomData<(G, O)>);
impl<G: DukaGenerator<O>, O> CodegenNode<G, O> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<G: DukaGenerator<O>, O> Node for CodegenNode<G, O> {
    type Input = G::InputType;
    type Output = O;
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("codegen")
    }
    fn process(&mut self, input: Self::Input) -> Result<Self::Output, Error> {
        Ok(G::new().generate(input)?)
    }
}
