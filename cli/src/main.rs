//! Commandline Tool for Duka
//!
//!

use anyhow::{Result, anyhow};
use clap::{ArgAction, Parser as ClapParser, ValueEnum};
use duka_backend::codegen::targets::default::Generator;
use duka_frontend::{ir::IRGenerator, prelude::*};
use std::{fmt::Display, path::PathBuf};

use crate::pipeline::{
    AdapterNode, AnalyzerNode, ChunkToBytes, CodegenNode, FileNode, FileToChunk, FileToIR,
    FileToProto, FileToRaw, FileToTokens, IRToBytes, LexerNode, MacroLexerNode, ParserNode,
    ProtoToBytes, Tokens, TokensToBytes, WriterNode,
};

use duka_pipeline::{Pipeline, Recipe, RecipePart};

mod pipeline;

const VERSION: &str = "0.2.0";

#[derive(ClapParser, Debug)]
#[command(
    version(VERSION),
    about("Interpreter commandline tool for duka language"),
    author("Aogangsolang")
)]
struct Args {
    /// Input path
    file: PathBuf,

    #[arg(short, help = "Output path (if has)")]
    output: Option<PathBuf>,

    /// Type of output
    #[arg(long, short, help = "Output type")]
    to: Option<DataType>,
    /// Type of input
    #[arg(long, short, help = "Input type")]
    from: Option<DataType>,

    #[arg(long, help = "Disable analyzer", action = ArgAction::SetTrue)]
    no_analyze: bool,
    #[arg(long, help = "Disable adapter", action = ArgAction::SetTrue)]
    no_adapt: bool,
    #[arg(long, help="Disable macro expander", action = ArgAction::SetTrue)]
    no_macro: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StepName {
    File,
    Output,
    Lexer,
    MacroLexer,
    Parser,
    Analyzer,
    Adapter,
    IRCompiler,
    Bytecode,
    Executor,
}
impl Display for StepName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
pub(crate) enum DataType {
    /// Raw code file .duka
    #[default]
    Raw,
    /// Tokens array in .json
    Tokens,
    /// AST object in .json
    AST,
    IR,
    /// Compiled bytecode in .dukac
    Bytecode,
    Run,
}
impl Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Type({})",
            match self {
                DataType::Raw => "source code",
                DataType::Tokens => "tokens",
                DataType::AST => "syntax tree",
                DataType::Bytecode => "bytecode",
                DataType::Run => "result",
                DataType::IR => "IR code",
            }
        )
    }
}

/// Entrypoint of Commandline Tool for Duka
fn main() -> Result<()> {
    let Args {
        file,
        output,
        to,
        from,
        no_adapt,
        no_analyze,
        no_macro,
    } = if cfg!(debug_assertions) {
        Args {
            file: std::env::current_dir().unwrap().join("examples/test.duka"),
            output: None,
            to: Some(DataType::Bytecode),
            from: Some(DataType::Raw),
            no_analyze: false,
            no_adapt: false,
            no_macro: false,
        }
    } else {
        Args::parse()
    };
    let to = to.unwrap_or_default();
    let from = from.unwrap_or_default();

    let mut pipeline = Pipeline::new()
        .node(Box::new(FileNode))
        .node(Box::new(LexerNode))
        .node(Box::new(MacroLexerNode))
        .node(Box::new(ParserNode::<Parser<Tokens>>::new()))
        .node(Box::new(AnalyzerNode::new(Analyzer)))
        .node(Box::new(AdapterNode::new(Adapter)))
        .node(Box::new(CodegenNode::<IRGenerator, _, _>::new(
            StepName::IRCompiler,
        )))
        .node(Box::new(CodegenNode::<Generator, _, _>::new(
            StepName::Bytecode,
        )))
        .node(Box::new(WriterNode::to(output)))
        .converter(Box::new(FileToRaw))
        .converter(Box::new(FileToTokens))
        .converter(Box::new(FileToChunk))
        .converter(Box::new(FileToProto))
        .converter(Box::new(FileToIR))
        .converter(Box::new(TokensToBytes))
        .converter(Box::new(ChunkToBytes))
        .converter(Box::new(ProtoToBytes))
        .converter(Box::new(IRToBytes));

    let recipe = Recipe::new()
        .pre(StepName::File)
        .step(
            RecipePart::named(if no_macro {
                StepName::Lexer
            } else {
                StepName::MacroLexer
            })
            .input(DataType::Raw)
            .output(DataType::Tokens),
        )
        .step(
            RecipePart::named(StepName::Parser).input(DataType::Tokens), //.output(ArcType::AST),
        )
        .step(
            RecipePart::named(StepName::Analyzer)
                .input(DataType::AST)
                .when(!no_analyze),
        )
        .step(
            RecipePart::named(StepName::Adapter)
                .output(DataType::AST)
                .when(!no_adapt),
        )
        .step(RecipePart::named(StepName::IRCompiler).output(DataType::IR))
        .step(
            RecipePart::named(StepName::Bytecode)
                .input(DataType::IR)
                .output(DataType::Bytecode),
        )
        .step(
            RecipePart::named(StepName::Executor)
                .input(DataType::Bytecode)
                .output(DataType::Run),
        )
        .post(StepName::Output);

    let steps = recipe
        .find(from, to)
        .map_err(|e| anyhow!("Invalid parameter").context(e))?;
    pipeline.process(steps, Box::new(file))?;

    Ok(())
}
