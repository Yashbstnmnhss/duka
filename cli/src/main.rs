//! Commandline Tool for Duka
//!
//!
use crate::pipeline::{
    AdapterNode, AnalyzerNode, ChunkToBytes, CodegenNode, FileNode, FileToChunk, FileToIR,
    FileToProto, FileToRaw, FileToTokens, IRToBytes, LexerNode, MacroLexerNode, ParserNode,
    ProtoToBytes, RunNode, TokensToBytes, ValueCountToBytes, WriterNode, to_diagnose,
};
use clap::{ArgAction, Parser as ClapParser, Subcommand, ValueEnum};
use duka_backend::codegen::targets::default::Generator;
use duka_frontend::{
    ir::IRGenerator,
    lexer::{Lexer, token::Token},
    prelude::*,
};
use duka_pipeline::{Pipeline, Recipe, RecipePart};
use duka_shared::types::{DukaResumable, TokenStream};
use miette::{IntoDiagnostic, MietteHandlerOpts, Result, miette};
use rustyline::{ColorMode, DefaultEditor, config::Configurer, error::ReadlineError};
use std::{fmt::Display, io::Cursor, path::PathBuf};
use syntect::{highlighting::ThemeSet, parsing::SyntaxSetBuilder};

mod pipeline;

const VERSION: &str = "0.2.0";

#[derive(Debug, Subcommand, Default)]
enum Commands {
    Pipeline {
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
    },
    #[default]
    REPL,
}

#[derive(ClapParser, Debug)]
#[command(
    version(VERSION),
    about("Interpreter commandline tool for duka language"),
    author("Aogangsolang")
)]
struct Args {
    #[command(subcommand)]
    cmd: Option<Commands>,
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
    let mut syntax_builder = SyntaxSetBuilder::new();
    syntax_builder
        .add_from_folder("./", true)
        .expect("Failed to load grammar file");
    let custom_set = syntax_builder.build();
    let theme_set = ThemeSet::load_defaults();
    use miette::highlighters::SyntectHighlighter;
    let highlighter = SyntectHighlighter::new(
        custom_set,
        theme_set.themes["base16-ocean.dark"].clone(),
        false,
    );
    miette::set_hook(Box::new(move |_| {
        Box::new(
            MietteHandlerOpts::new()
                .with_syntax_highlighting(highlighter.clone())
                .build(),
        )
    }))?;

    let cmd = if false {
        Commands::Pipeline {
            file: std::env::current_dir().unwrap().join("examples/test.duka"),
            output: None,
            to: Some(DataType::Run),
            from: Some(DataType::Raw),
            no_analyze: false,
            no_adapt: false,
            no_macro: false,
        }
    } else {
        Args::parse().cmd.unwrap_or_default()
    };

    do_cmd(cmd)
}

fn do_cmd(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Pipeline {
            file,
            output,
            to,
            from,
            no_analyze,
            no_adapt,
            no_macro,
        } => {
            let to = to.unwrap_or_default();
            let from = from.unwrap_or_default();

            let mut pipeline = Pipeline::new()
                .node(Box::new(FileNode))
                .node(Box::new(LexerNode))
                .node(Box::new(MacroLexerNode))
                .node(Box::new(ParserNode::<Parser<Token>>::new()))
                .node(Box::new(AnalyzerNode::new(Analyzer)))
                .node(Box::new(AdapterNode::new(Adapter)))
                .node(Box::new(CodegenNode::<IRGenerator, _, _>::new(
                    StepName::IRCompiler,
                )))
                .node(Box::new(CodegenNode::<Generator, _, _>::new(
                    StepName::Bytecode,
                )))
                .node(Box::new(RunNode))
                .node(Box::new(WriterNode::to(output)))
                .converter(Box::new(FileToRaw))
                .converter(Box::new(FileToTokens))
                .converter(Box::new(FileToChunk))
                .converter(Box::new(FileToProto))
                .converter(Box::new(FileToIR))
                .converter(Box::new(TokensToBytes))
                .converter(Box::new(ChunkToBytes))
                .converter(Box::new(ProtoToBytes))
                .converter(Box::new(IRToBytes))
                .converter(Box::new(ValueCountToBytes));

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
                .map_err(|e| miette!("Invalid parameter").context(e))?;
            pipeline.process(steps, Box::new(file))?;
        }
        _ => {
            fn deal(i: Result<String, ReadlineError>) -> miette::Result<String> {
                match i {
                    Ok(str) => Ok(str),
                    Err(ReadlineError::Eof | ReadlineError::Interrupted) => std::process::exit(0),
                    Err(e) => Err(e).into_diagnostic(),
                }
            }

            let mut rl = DefaultEditor::new().into_diagnostic()?;
            rl.set_color_mode(ColorMode::Enabled);

            println!("Duka REPL");
            println!("type :exit to exit");
            'main: loop {
                let line = deal(rl.readline(">>> "))?;
                if let Some(cmd) = line.strip_prefix(":") {
                    match cmd {
                        "exit" => break,
                        "clear" => rl.clear_screen().into_diagnostic()?,
                        _ => eprintln!("Unknown command: {cmd}"),
                    }
                    continue;
                }

                let mut lexer = Lexer::new(Cursor::new(line), Some("REPL".to_owned()));
                let mut tokens = vec![];
                let stream = loop {
                    let res = lexer.next_token_resumable();
                    match res {
                        Ok(DukaResumable::Complete((t, _))) if t.is_terminator() => {
                            break TokenStream::new(tokens.into(), lexer.source_info());
                        }
                        Ok(DukaResumable::Complete(tk)) => {
                            tokens.push(tk);
                        }
                        Ok(DukaResumable::Incomplete(..)) => {
                            let new_line = deal(rl.readline("... "))?;
                            lexer.resume(Cursor::new(format!("\n{new_line}")));
                        }
                        Err(e) => {
                            println!("{:?}", miette::Report::new(to_diagnose(e)));
                            continue 'main;
                        }
                    }
                };

                let ast = match Parser::new(stream).parse_expr_or_stmt() {
                    Ok(k) => k,
                    Err(e) => {
                        println!("{:?}", miette::Report::new(to_diagnose(e)));
                        continue 'main;
                    }
                };
                println!("{ast:?}")
            }
        }
    };

    Ok(())
}
