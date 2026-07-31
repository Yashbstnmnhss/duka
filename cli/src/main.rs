//! Commandline Tool for Duka
//!
//!
use crate::pipeline::{
    AdapterNode, AnalyzerNode, ChunkToBytes, CodegenNode, DukaSpannedDiagnoses, FileNode,
    FileToChunk, FileToIR, FileToProto, FileToRaw, FileToTokens, IRToBytes, LexerNode,
    MacroLexerNode, ParserNode, ProtoToBytes, ResultsToBytes, RunNode, TokensToBytes, WriterNode,
    to_diagnose,
};
use clap::{ArgAction, Parser as ClapParser, Subcommand, ValueEnum};
use colored::Colorize;
use duka_backend::{DukaVM, codegen::DefaultGenerator, vm::VM};
use duka_frontend::{
    analyzer::ScopeAnalyzer,
    ir::IRGenerator,
    lexer::{Lexer, token::Token},
    parser::ast::DukaChunk,
    prelude::*,
};
use duka_gc::Heap;
use duka_pipeline::{Pipeline, Recipe, RecipePart};
use duka_shared::{
    config::DukaIRConfig,
    errors::{DukaErrorKind, DukaParserError, DukaSpannedError},
    types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaResumable, TokenStream},
};
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
    Repl,
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
    Ast,
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
                DataType::Ast => "syntax tree",
                DataType::Bytecode => "bytecode",
                DataType::Run => "result",
                DataType::IR => "IR code",
            }
        )
    }
}

/// Entrypoint of Commandline Tool for Duka
fn main() -> Result<()> {
    miette::set_hook(Box::new(move |_| {
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
        Box::new(
            MietteHandlerOpts::new()
                .with_syntax_highlighting(highlighter)
                .build(),
        )
    }))?;

    let cmd = if cfg!(debug_assertions) {
        Commands::Pipeline {
            file: std::env::current_dir()
                .unwrap()
                .join("examples/test.duka.lua"),
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

macro_rules! repl_help {
    ($prefix: literal, @help ($cmd: ident, $head:literal) $(,($rest_cmd: ident, $rest:literal))*) => {
        concat!("- ", $prefix, stringify!($cmd), ": ", $head, "\n", repl_help!($prefix, @help $(($rest_cmd, $rest)),*))
    };
    ($prefix: literal, @help) => {
        concat!("- ", $prefix, "help: get command list", "\n")
    }
}
macro_rules! repl_cmd {
    (match $input: ident; prefix = $prefix: literal; $($name:ident($desc:literal) => $do:block),+) => {
        match $input {
            $(stringify!($name) => $do),+,
            "help" => {
                println!("{}", "Duka REPL Help".bright_blue());
                println!(
                    repl_help!($prefix, @help $(($name, $desc)),+)
                );
            },
            i if i.is_empty() => eprintln!("{}", "Empty command".red()),
            _ => eprintln!("{}", format!("Unknown command: {}", $input).red())
        }
    };
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
                .node(Box::new(AnalyzerNode::new(
                    ScopeAnalyzer.chain(BasicAnalyzer),
                    Default::default(),
                )))
                .node(Box::new(AdapterNode::new(Adapter)))
                .node(Box::new(CodegenNode::<IRGenerator, _, _>::new(
                    StepName::IRCompiler,
                    Default::default(),
                )))
                .node(Box::new(CodegenNode::<DefaultGenerator, _, _>::new(
                    StepName::Bytecode,
                    (),
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
                .converter(Box::new(ResultsToBytes));
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
                        .input(DataType::Ast)
                        .when(!no_analyze),
                )
                .step(
                    RecipePart::named(StepName::Adapter)
                        .output(DataType::Ast)
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
            fn deal(i: Result<String, ReadlineError>) -> Result<String> {
                match i {
                    Ok(str) => Ok(str),
                    Err(ReadlineError::Eof | ReadlineError::Interrupted) => std::process::exit(0),
                    Err(e) => Err(e).into_diagnostic(),
                }
            }

            let mut rl = DefaultEditor::new().into_diagnostic()?;
            rl.set_color_mode(ColorMode::Enabled);

            println!("Duka REPL");
            println!("type ?exit to exit");

            let mut vm = VM::new(Heap::new());

            'main: loop {
                let line = deal(rl.readline(">>> "))?;
                if let Some(cmd) = line.strip_prefix("?") {
                    repl_cmd! {
                        match cmd;
                        prefix = "?";
                        exit("exit REPL") => { break },
                        clear("clear screen") => { rl.clear_screen().into_diagnostic()? }
                    }
                    continue;
                }

                let mut lexer = Lexer::new(Cursor::new(line), Some("REPL".to_owned()));
                let mut tokens = vec![];
                let ast = loop {
                    let res = lexer.next_token_resumable();
                    match res {
                        Ok(DukaResumable::Complete((t, _))) if t.is_terminator() => {
                            let stream =
                                TokenStream::new(tokens.clone().into(), lexer.source_info());
                            match Parser::new(stream, Default::default()).parse_expr_or_stmt() {
                                Ok(k) => break k,
                                Err(DukaSpannedError {
                                    kind: DukaErrorKind::Parser(DukaParserError::UnexpectedEnd(_)),
                                    ..
                                }) => (), // read_new_line
                                Err(e) => {
                                    println!("{:?}", miette::Report::new(to_diagnose(e)));
                                    continue 'main;
                                }
                            }
                        }
                        Ok(DukaResumable::Complete(tk)) => {
                            tokens.push(tk);
                            continue;
                        }
                        Ok(DukaResumable::Incomplete(..)) => (), // read new_line
                        Err(e) => {
                            println!("{:?}", miette::Report::new(to_diagnose(e)));
                            continue 'main;
                        }
                    }
                    let new_line = deal(rl.readline("... "))?;
                    lexer.resume(Cursor::new(format!("\n{new_line}")));
                };
                let mut chunk = DukaChunk {
                    span: ast.get_span(),
                    block: ast.into_block(),
                    logic: Default::default(),
                    source_info: lexer.source_info(),
                };

                let errors: Vec<_> = ScopeAnalyzer
                    .chain(BasicAnalyzer)
                    .analyze(&chunk, Default::default())
                    .1
                    .map(to_diagnose)
                    .collect();
                if !errors.is_empty() {
                    println!(
                        "{:?}",
                        miette::Report::new(DukaSpannedDiagnoses { relates: errors })
                    );
                    continue 'main;
                }

                Adapter.adapt(&mut chunk);

                let ir = match IRGenerator::generate(
                    chunk,
                    DukaIRConfig {
                        var_default_local: false,
                        ..DukaIRConfig::default()
                    },
                ) {
                    Ok(ir) => ir,
                    Err(e) => {
                        eprintln!("{}", format!("{:?}", e).red());
                        continue 'main;
                    }
                };
                let proto = match DefaultGenerator::generate(ir, ()) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("{}", format!("{:?}", e).red());
                        continue 'main;
                    }
                };

                let vc = match vm.execute(&proto) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("{}", format!("{:?}", e).red());
                        continue 'main;
                    }
                };
                println!(
                    "{}",
                    vm.scheduler
                        .main()
                        .inner
                        .get_stack_many(0, vc)
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    };

    Ok(())
}
