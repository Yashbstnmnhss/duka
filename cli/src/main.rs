//! Commandline Tool for Duka
//!
//!
use crate::docgen::gen_doc;
use crate::pipeline::{
    AdapterNode, AnalyzerNode, BangExpanderNode, ChunkToBytes, CodegenNode, DukaSpannedDiagnoses,
    FileNode, FileToChunk, FileToIR, FileToProto, FileToRaw, FileToTokens, IRToBytes, LexerNode,
    MacroLexerNode, ParserNode, ProtoToBytes, ResultsToBytes, RunNode, TokensToBytes, WriterNode,
    to_diagnose,
};
use clap::{ArgAction, Parser as ClapParser, Subcommand, ValueEnum};
use colored::Colorize;
use duka_lib::duka_frontend::{
    analyzer::{ScopeAnalyzer, TypeChecker},
    expander::BangExpanderRegistry,
    ir::IRGenerator,
    lexer::{Lexer, token::Token},
    parser::ast::{Block, DukaChunk, ExprOrStmt, Stmt, StmtKind, TypeDescriptor},
    prelude::*,
};
use duka_lib::duka_gc::Heap;
use duka_lib::duka_shared::{
    config::{DukaAnalyzerConfig, DukaIRConfig, DukaParserConfig},
    constants::COMPILED_SUFFIX,
    errors::{DukaErrorKind, DukaParserError, DukaSpannedError},
    types::{
        DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser, DukaResumable, TokenStream,
    },
};
use duka_lib::{
    DukaVM,
    codegen::DefaultGenerator,
    errors::{DukaRuntimeError, DukaStackTrace},
    vm::VM,
};
use duka_pipeline::{Pipeline, Recipe, RecipePart};
use miette::{Diagnostic, IntoDiagnostic, MietteHandlerOpts, Result, miette};
use rustyline::{
    ColorMode, Editor, Helper,
    completion::{Candidate, Completer},
    config::Configurer,
    error::ReadlineError,
    highlight::Highlighter,
    hint::{Hint, Hinter},
    history::FileHistory,
    validate::Validator,
};
use std::{
    borrow::Cow,
    error::Error,
    fmt::Display,
    io::Cursor,
    path::{Path, PathBuf},
};
use syntect::{highlighting::ThemeSet, parsing::SyntaxSetBuilder};
use thiserror::Error;

mod docgen;
mod pipeline;

const VERSION: &str = "0.2.5";

#[derive(Debug, clap::Args, Clone)]
#[group(required = false, multiple = true)]
struct Configs {
    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    enable_type_annotations: bool,
    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    var_default_local: bool,
    #[arg(long, action = ArgAction::Set, default_value_t = false)]
    default_nonnilable: bool,
}

#[derive(Debug, Subcommand, Default)]
enum Commands {
    /// Compilation pipeline
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

        #[command(flatten)]
        configs: Configs,
    },
    /// Generate markdown document files
    DocGen {
        #[arg(short, help = "Output path (if has)")]
        output: Option<PathBuf>,
    },
    /// Run in REPL mode
    #[default]
    Repl,
}

#[derive(ClapParser, Debug)]
#[command(
    version(VERSION),
    about("Interpreter commandline tool for duka language"),
    author("AogangSolang")
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
    BangExpander,
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
    AdaptedAST,
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
                DataType::AdaptedAST => "desugared syntax tree",
                DataType::Bytecode => "bytecode",
                DataType::Run => "result",
                DataType::IR => "IR code",
            }
        )
    }
}

/// Entrypoint of Commandline Tool for Duka
fn main() -> Result<()> {
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .name("duka-vm".into())
        .spawn(real_main)
        .into_diagnostic()?;
    match handle.join() {
        Ok(inner) => inner,
        Err(_) => Err(miette!("duka vm worker thread panicked")),
    }
}

fn real_main() -> Result<()> {
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

    let cmd = Args::parse().cmd.unwrap_or_default();
    do_cmd(cmd)
}

macro_rules! repl_help {
    ($prefix: literal, @help ($cmd: ident, $head:literal) $(,($rest_cmd: ident, $rest:literal))*) => {
        concat!("- ", $prefix, stringify!($cmd), ": ", $head, "\n", repl_help!($prefix, @help $(($rest_cmd, $rest)),*))
    };
    ($prefix: literal, @help) => {
        concat!(
            "- ", $prefix, "help: get command list", "\n",
            "- ", $prefix, "type <expr>: infer the type of an expression", "\n"
        )
    }
}
macro_rules! repl_cmd {
    (fn $fn_name: ident($($pname:ident : $ptype:ty),*); prefix = $prefix: literal; $($name:ident($desc:literal) => $do:block),+) => {
        #[allow(dead_code)]
        const REPL_CMDS: &[&str] = &[
            "help",
            $(stringify!($name)),+
        ];
        fn $fn_name(input: &str, $($pname : $ptype),*) -> Result<()> {
            match input {
                $(stringify!($name) => $do),+,
                "help" => {
                    println!("{}", "Duka REPL Help".bright_blue());
                    println!(
                        repl_help!($prefix, @help $(($name, $desc)),+)
                    );
                },
                i if i.is_empty() => eprintln!("{}", "Empty command".red()),
                _ => eprintln!("{}", format!("Unknown command: {}", input).red())
            };
            Ok(())
        }
    };
}

repl_cmd! {
    fn parse_cmd(rl: &mut Editor<DukaHelper, FileHistory>);
    prefix = "?";
    exit("exit REPL") => { std::process::exit(0) },
    clear("clear screen") => { rl.clear_screen().into_diagnostic()? }
}

fn deal(i: Result<String, ReadlineError>) -> Result<String> {
    match i {
        Ok(str) => Ok(str),
        Err(ReadlineError::Eof | ReadlineError::Interrupted) => std::process::exit(0),
        Err(e) => Err(e).into_diagnostic(),
    }
}

fn read_repl_input(
    rl: &mut Editor<DukaHelper, FileHistory>,
    first: String,
) -> Result<Option<(ExprOrStmt, duka_lib::duka_shared::types::SourceInfo)>> {
    let mut lexer = Lexer::new(
        Cursor::new(first),
        Some("REPL".to_owned()),
        Default::default(),
    );
    let mut tokens = vec![];
    loop {
        let res = lexer.next_token_resumable();
        match res {
            Ok(DukaResumable::Complete((t, _))) if t.is_terminator() => {
                let stream = TokenStream::new(tokens.clone().into(), lexer.source_info());
                match Parser::new(
                    stream,
                    DukaParserConfig {
                        var_default_local: false,
                        ..DukaParserConfig::default()
                    },
                )
                .parse_expr_or_stmt()
                {
                    Ok(k) => return Ok(Some((k, lexer.source_info()))),
                    Err(DukaSpannedError {
                        kind: DukaErrorKind::Parser(DukaParserError::UnexpectedEnd(_)),
                        ..
                    }) => (),
                    Err(e) => {
                        println!("{:?}", miette::Report::new(to_diagnose(e)));
                        return Ok(None);
                    }
                }
            }
            Ok(DukaResumable::Complete(tk)) => {
                tokens.push(tk);
                continue;
            }
            Ok(DukaResumable::Incomplete(..)) => (),
            Err(e) => {
                println!("{:?}", miette::Report::new(to_diagnose(e)));
                return Ok(None);
            }
        }
        let new_line = deal(rl.readline("... "))?;
        lexer.resume(Cursor::new(format!("\n{new_line}")));
    }
}

fn handle_type_query(rl: &mut Editor<DukaHelper, FileHistory>, src: String) -> Result<()> {
    let Some((ast, info)) = read_repl_input(rl, src.clone())? else {
        return Ok(());
    };
    let probe_expr = match ast {
        ExprOrStmt::Expr(e) => e,
        ExprOrStmt::Stmt(_) => {
            eprintln!("{}", "?type expects an expression".red());
            return Ok(());
        }
    };
    let probe_span = probe_expr.1;
    let alias = Stmt(
        StmtKind::TypeAlias(
            ("__ReplProbe__".to_owned(), probe_span),
            Box::new(TypeDescriptor::TypeOf {
                expr: Box::new(probe_expr),
                span: probe_span,
            }),
        ),
        probe_span,
    );
    let chunk = DukaChunk {
        span: probe_span,
        block: Block([alias].into(), None),
        logic: Default::default(),
        source_info: info,
    };

    let Some(sa) = analyze_with_prelude(&chunk) else {
        eprintln!("{}", "could not infer a type".red());
        return Ok(());
    };
    let ty = sa
        .symbols
        .lookup("__ReplProbe__")
        .and_then(|s| s.ty.clone());
    let ty = match ty {
        Some(t) if t.as_ref() != "any" => Some(t),
        _ => infer_type_syntax(&src).or(ty),
    };
    match ty {
        Some(ty) => println!("{} : {}", src.trim(), ty),
        None => eprintln!("{}", "could not infer a type".red()),
    }
    Ok(())
}

fn analyze_with_prelude(
    chunk: &DukaChunk,
) -> Option<duka_lib::duka_frontend::analyzer::ScopeAnalysis> {
    let cfg = DukaAnalyzerConfig {
        type_annotations: true,
        var_default_local: false,
        ..Default::default()
    };
    let (d0, e0) = ScopeAnalyzer.analyze(chunk, cfg.clone());
    if e0.count() > 0 {
        return None;
    }
    let (cfg2, mut sa) = d0;
    duka_lib::duka_frontend::analyzer::prelude::inject_type_prelude(&mut sa);
    let pipeline = BasicAnalyzer.chain(TypeChecker);
    let (d1, errs) = pipeline.analyze(chunk, (cfg2, sa));
    if errs.count() > 0 {
        return None;
    }
    Some(d1.1)
}

fn infer_type_syntax(src: &str) -> Option<Box<str>> {
    const PROBE: &str = "__ReplProbe__";
    let text = format!("type {PROBE} = {src}");
    let lexer = Lexer::new(
        Cursor::new(&text),
        Some("REPL".to_owned()),
        Default::default(),
    );
    let stream = lexer.tokenize().ok()?;
    let chunk = Parser::parse(
        stream,
        DukaParserConfig {
            type_annotations: true,
            ..DukaParserConfig::default()
        },
    )
    .ok()?;
    analyze_with_prelude(&chunk)?
        .symbols
        .lookup(PROBE)
        .and_then(|s| s.ty.clone())
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
            configs,
        } => {
            let to = to.unwrap_or_default();
            // Infer the input type from the file suffix: `{COMPILED_SUFFIX}`
            // files are pre-compiled bytecode and skip the whole compile chain.
            let from = from.unwrap_or_else(|| {
                if matches!(file.extension(), Some(t) if t == COMPILED_SUFFIX) {
                    DataType::Bytecode
                } else {
                    DataType::Raw
                }
            });

            // Wire up the module loader: `require("foo.bar")` resolves against the
            // DUKA_PATH templates (default: `<dir-of-script>/modules`).
            let parent = file.parent().unwrap_or_else(|| Path::new("."));
            let paths = duka_lib::module::search_paths(parent, "modules");
            duka_lib::builtin::require::set_loader(duka_lib::module::file_loader(
                paths,
                Default::default(),
            ));

            let mut pipeline = Pipeline::new()
                .node(Box::new(FileNode))
                .node(Box::new(LexerNode))
                .node(Box::new(MacroLexerNode))
                .node(Box::new(ParserNode::<Parser<Token>>::new(
                    DukaParserConfig {
                        type_annotations: configs.enable_type_annotations,
                        var_default_local: configs.var_default_local,
                        default_nonnilable: configs.default_nonnilable,
                        ..Default::default()
                    },
                )))
                .node(Box::new(BangExpanderNode(BangExpanderRegistry::new())))
                .node(Box::new(AnalyzerNode::new(
                    ScopeAnalyzer.chain(BasicAnalyzer).chain(TypeChecker),
                    DukaAnalyzerConfig {
                        var_default_local: configs.var_default_local,
                        type_annotations: configs.enable_type_annotations,
                        default_nonnilable: configs.default_nonnilable,
                    },
                )))
                .node(Box::new(AdapterNode::new(Adapter)))
                .node(Box::new(CodegenNode::<IRGenerator, _, _>::new(
                    StepName::IRCompiler,
                    DukaIRConfig {
                        var_default_local: configs.var_default_local,
                    },
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
                    RecipePart::named(StepName::BangExpander)
                        .input(DataType::AST)
                        .output(DataType::AST),
                )
                .step(
                    RecipePart::named(StepName::Analyzer)
                        .input(DataType::AST)
                        .when(!no_analyze),
                )
                .step(
                    RecipePart::named(StepName::Adapter)
                        .output(DataType::AdaptedAST)
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
        Commands::DocGen { output } => gen_doc(output)?,
        _ => {
            let mut rl = Editor::<DukaHelper, FileHistory>::new().into_diagnostic()?;
            rl.set_color_mode(ColorMode::Enabled);
            rl.set_indent_size(4);
            rl.set_helper(Some(DukaHelper));

            println!("type ?exit to exit");

            let mut vm = VM::new(Heap::new());
            'main: loop {
                let line = deal(rl.readline(">>> "))?;
                rl.add_history_entry(&line).into_diagnostic()?;
                if let Some(rest) = line
                    .strip_prefix("?type ")
                    .or_else(|| line.strip_prefix("?t "))
                {
                    handle_type_query(&mut rl, rest.to_owned())?;
                    continue;
                }
                if let Some(cmd) = line.strip_prefix("?") {
                    parse_cmd(cmd, &mut rl)?;
                    continue;
                }

                let Some((ast, info)) = read_repl_input(&mut rl, line)? else {
                    continue 'main;
                };
                let mut chunk = DukaChunk {
                    span: ast.get_span(),
                    block: ast.into_block(),
                    logic: Default::default(),
                    source_info: info,
                };

                let errors: Vec<_> = ScopeAnalyzer
                    .chain(BasicAnalyzer)
                    .chain(TypeChecker)
                    .analyze(
                        &chunk,
                        DukaAnalyzerConfig {
                            var_default_local: false,
                            ..Default::default()
                        },
                    )
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
                        eprintln!("{:?}", miette!("IRGenerator error").context(e));
                        continue 'main;
                    }
                };
                let proto = match DefaultGenerator::generate(ir, ()) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("{:?}", miette!("DefaultGenerator error").context(e));
                        continue 'main;
                    }
                };

                let vc = match vm.execute(&proto) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "{:?}",
                            miette::Report::new(DukaRuntimeDiagnose {
                                source: e.kind,
                                stack_trace: DukaStackTraceDiagnose { inner: e.trace }
                            })
                        );
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

#[derive(Debug, Diagnostic)]
struct DukaStackTraceDiagnose {
    inner: DukaStackTrace,
}
impl Error for DukaStackTraceDiagnose {}
impl Display for DukaStackTraceDiagnose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

#[derive(Debug, Diagnostic, Error)]
#[error("Duka error")]
#[diagnostic()]
pub struct DukaRuntimeDiagnose {
    #[source]
    source: DukaRuntimeError,
    #[help]
    stack_trace: DukaStackTraceDiagnose,
}

struct DukaHint {
    pub display: String,
    pub completion: Option<String>,
}
impl Hint for DukaHint {
    fn display(&self) -> &str {
        &self.display
    }
    fn completion(&self) -> Option<&str> {
        self.completion.as_ref().map(|s| s.as_str())
    }
}

struct DukaCandidate {
    pub display: String,
    pub replacement: String,
}
impl Candidate for DukaCandidate {
    fn display(&self) -> &str {
        &self.display
    }
    fn replacement(&self) -> &str {
        &self.replacement
    }
}

struct DukaHelper;

impl Helper for DukaHelper {}
impl Hinter for DukaHelper {
    type Hint = DukaHint;
    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<Self::Hint> {
        if pos == line.len() && line.starts_with('?') {
            let prefix = &line[1..];
            for cmd in REPL_CMDS {
                if cmd.starts_with(prefix) && cmd.len() > prefix.len() {
                    let suffix = cmd[prefix.len()..].to_string();
                    return Some(DukaHint {
                        display: suffix,
                        completion: Some(format!("?{}", cmd)),
                    });
                }
            }
        }
        None
    }
}
impl Completer for DukaHelper {
    type Candidate = DukaCandidate;
    fn complete(
        &self, // FIXME should be `&mut self`
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        if line.starts_with('?') && pos >= 1 {
            let prefix = &line[1..pos];
            let matches: Vec<_> = REPL_CMDS
                .iter()
                .filter(|cmd| cmd.starts_with(prefix))
                .map(|cmd| DukaCandidate {
                    display: format!("?{}", cmd),
                    replacement: format!("?{}", cmd),
                })
                .collect();
            return Ok((0, matches));
        }
        Ok((pos, vec![]))
    }
}
impl Validator for DukaHelper {}
impl Highlighter for DukaHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(hint.blue().to_string())
    }
}
