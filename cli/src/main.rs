//! Commandline Tool for Duka
//!
//!

use anyhow::{Context, Result, anyhow};
use clap::{ArgAction, Parser as ClapParser, ValueEnum};
use duka_backend::{codegen::Generator, value::DukaProto};
use duka_frontend::prelude::*;
use duka_shared::{token::Token, types::DukaChunk};

use std::{fs::File, io, path::PathBuf};

use crate::pipeline::{
    AdapterNode, AnalyzerNode, ChunkToBytes, CodegenNode, FileNode, FileToChunk, FileToProto,
    LexerNode, OutNode, ParserNode, ProtoToBytes,
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
    to: Option<ArcType>,
    /// Type of input
    #[arg(long, short, help = "Input type")]
    from: Option<ArcType>,

    #[arg(long, help = "Disable analyzer", action = ArgAction::SetTrue)]
    no_analyze: bool,
    #[arg(long, help = "Disable adapter", action = ArgAction::SetTrue)]
    no_adapt: bool,
}
#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
enum ArcType {
    /// Raw code file .duka
    #[default]
    Raw,
    /// Tokens array in .json
    Tokens,
    /// AST object in .json
    AST,
    /// Compiled bytecode in .dukac
    Bytecode,
    Run,
}

type LexerN = LexerNode<File, LexerWithMacro<File>>;
type ParserN = ParserNode<File, LexerWithMacro<File>, Parser<File, Token, LexerWithMacro<File>>>;
type AnalyzerN = AnalyzerNode<Analyzer>;
type AdapterN = AdapterNode<Adapter>;
type CompilerN = CodegenNode<Generator, DukaProto>;

/// Entrypoint of Commandline Tool for Duka
fn main() -> Result<()> {
    #[cfg(debug_assertions)]
    #[inline]
    fn get_args() -> Args {
        Args {
            file: std::env::current_dir().unwrap().join("test.duka"),
            output: None,
            to: Some(ArcType::AST),
            from: Some(ArcType::Raw),
            no_analyze: false,
            no_adapt: false,
        }
    }
    #[cfg(not(debug_assertions))]
    #[inline]
    fn get_args() -> Args {
        Args::parse()
    }

    let Args {
        file,
        output,
        to,
        from,
        no_adapt,
        no_analyze,
    } = get_args();
    let to = to.unwrap_or_default();
    let from = from.unwrap_or_default();

    let mut pipeline = Pipeline::new()
        .node(Box::new(FileNode))
        .node(Box::new(LexerN::new()))
        .node(Box::new(ParserN::new()))
        .node(Box::new(AnalyzerN::new(Analyzer)))
        .node(Box::new(AdapterN::new(Adapter)))
        .node(Box::new(CompilerN::new()))
        .node(Box::new(OutNode::from(output)))
        .converter(Box::new(FileToChunk))
        .converter(Box::new(FileToProto))
        .converter(Box::new(ChunkToBytes))
        .converter(Box::new(ProtoToBytes));

    let recipe = Recipe::<_, &'static str>::new()
        .pre("file")
        .step(
            RecipePart::named("lexer")
                .input(ArcType::Raw)
                .output(ArcType::Tokens),
        )
        .step(
            RecipePart::named("parser").input(ArcType::Tokens), //.output(ArcType::AST),
        )
        .step(
            RecipePart::named("analyzer")
                .input(ArcType::AST)
                .when(!no_analyze),
        )
        .step(
            RecipePart::named("adapter")
                .output(ArcType::AST)
                .when(!no_adapt),
        )
        .step(RecipePart::named("compiler").output(ArcType::Bytecode))
        .step(
            RecipePart::named("executor")
                .input(ArcType::Bytecode)
                .output(ArcType::Run),
        )
        .post("output");

    let steps = recipe
        .find(from, to)
        .map_err(|e| anyhow!("Invalid parameter").context(e))?;
    pipeline.process(steps, Box::new(file))?;

    Ok(())
}
