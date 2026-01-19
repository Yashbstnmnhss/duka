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
    FileToRaw, FileToTokens, LexerNode, MacroLexerNode, ParserNode, ProtoToBytes, Raw, Tokens,
    TokensToBytes, WriterNode,
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
    #[arg(long, help="Disable macro expander", action = ArgAction::SetTrue)]
    no_macro: bool,
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
            file: std::env::current_dir().unwrap().join("test.duka"),
            output: Some(r"D:\a.tokens".into()),
            to: Some(ArcType::Tokens),
            from: Some(ArcType::Raw),
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
        .node(Box::new(CodegenNode::<Generator, _>::new()))
        .node(Box::new(WriterNode::to(output)))
        .converter(Box::new(FileToRaw))
        .converter(Box::new(FileToTokens))
        .converter(Box::new(FileToChunk))
        .converter(Box::new(FileToProto))
        .converter(Box::new(TokensToBytes))
        .converter(Box::new(ChunkToBytes))
        .converter(Box::new(ProtoToBytes));

    let recipe = Recipe::<_, &'static str>::new()
        .pre("file")
        .step(
            RecipePart::named(no_macro.then_some("lexer").unwrap_or("macro-lexer"))
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
