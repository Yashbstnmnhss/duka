//! Commandline Tool for Duka
//!
//!

use anyhow::{Context, Result, anyhow};
use clap::{Parser as ClapParser, ValueEnum};
use duka_frontend::prelude::*;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaParser};
use std::{fs::File, io::BufReader, path::PathBuf};

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

    /// Type of output
    #[arg(long, short, help = "Output mode")]
    mode: Option<Mode>,
}
#[derive(ValueEnum, Clone, Debug, Default)]
enum Mode {
    /// Tokenized
    Tokens,
    /// AST
    Tree,
    /// Optimized AST
    #[default]
    OptimizedTree,
    /// Run code
    Run,
    /// Compile to bytecode
    Compile,
}

/// Entrypoint of Commandline Tool for Duka
fn main() -> Result<()> {
    let args = Args::parse();
    let mode = args.mode.unwrap_or_default();

    let script_path = &args.file;
    let input = File::open(script_path)
        .with_context(|| format!("Cannot open file {}", script_path.display()))?;
    let lex = LexerWithMacro::new(BufReader::new(input));

    if let Mode::Tokens = mode {
        let res: Result<Vec<_>, _> = lex.collect();
        let tsk = res?;

        serde_json::to_writer_pretty(std::io::stdout(), &tsk)?;
        return Ok(());
    }

    let mut chunk = Parser::new(lex).parse()?;

    if let Mode::Tree = mode {
        serde_json::to_writer_pretty(std::io::stdout(), &chunk)?;
        return Ok(());
    }

    let errs = Analyzer.analyze(&chunk);
    if !errs.is_empty() {
        return Err(errs
            .into_iter()
            .fold(anyhow!("Errors occurred during analyzing"), |acc, e| {
                acc.context(e)
            }));
    }
    Adapter.adapt(&mut chunk);

    if let Mode::OptimizedTree = mode {
        serde_json::to_writer_pretty(std::io::stdout(), &chunk)?;
        return Ok(());
    }

    // let res = Generator::new().generate(chunk);
    // ExeState::new().execute(&res);
    Ok(())
}
