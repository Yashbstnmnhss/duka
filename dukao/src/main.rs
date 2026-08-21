//! Commandline tool for duka
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::build::run_build_cmd;
use crate::init::run_init_cmd;
use crate::run::run_run_cmd;
use crate::test::run_test_cmd;

const VERSION: &str = "0.1.0";

mod build;
mod diag;
mod init;
mod run;
mod test;

pub const KAO_TESTS: &str = "tests";
pub const KAO_LOCK_FILE: &str = "kao.lock.toml";

#[derive(Parser, Debug)]
#[command(
    version(VERSION),
    about("Test & package tools for duka language"),
    author("AogangSolang")
)]
struct Args {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Create a new kao project
    Init {
        /// Where to create the project (default: `./`)
        path: Option<PathBuf>,
        /// Project name, defaults to the directory name
        #[arg(long)]
        name: Option<String>,
        /// Project version (default: 0.1.0)
        #[arg(long)]
        version: Option<String>,
        /// Overwrite existing files
        #[arg(long, short)]
        force: bool,
    },
    /// Build the current kao project (entry + modules) to bytecode
    Build {
        /// Project root (defaults to the nearest `kao.toml`, else `./`)
        path: Option<PathBuf>,
        /// Only list files that would be built, do not compile
        #[arg(long, short)]
        list: bool,
        /// Bundle the project into a single native executable
        #[arg(long)]
        exe: Option<Option<PathBuf>>,
        /// Bundle the project into a self-contained JS module (wasm)
        #[arg(long)]
        wasm: Option<Option<PathBuf>>,
    },
    /// Run duka scripts under a directory as unit tests
    Test {
        /// Directory to scan for `.duka` test scripts (default: `./tests`)
        path: Option<PathBuf>,
        /// Only list tests, do not run them
        #[arg(long, short)]
        list: bool,
        /// Only run tests whose path contains this substring
        #[arg(long)]
        filter: Option<String>,
        /// Disable colored output
        #[arg(long)]
        no_color: bool,
    },
    /// Run the current kao project's entry script
    Run {
        /// Project root (defaults to the nearest `kao.toml`, else `./`)
        path: Option<PathBuf>,
        /// Override the entry script (relative to the project root)
        #[arg(long)]
        entry: Option<String>,
        /// Disable colored output
        #[arg(long)]
        no_color: bool,
        /// Arguments passed to the script as its top-level `...`
        #[arg(last = true)]
        script_args: Vec<String>,
    },
}

fn main() {
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .name("dukao-worker".into())
        .spawn(real_main)
        .expect("failed to spawn dukao worker thread");
    let exit = match handle.join() {
        Ok(exit) => exit,
        Err(_) => {
            eprintln!("dukao worker thread panicked");
            1
        }
    };
    std::process::exit(exit);
}

fn real_main() -> i32 {
    let args = Args::parse();
    match args.cmd {
        Commands::Init {
            path,
            name,
            version,
            force,
        } => run_init_cmd(
            path.unwrap_or_else(|| PathBuf::from(".")),
            name,
            version,
            force,
        ),
        Commands::Build {
            path,
            list,
            exe,
            wasm,
        } => run_build_cmd(path.unwrap_or_else(|| PathBuf::from(".")), list, exe, wasm),
        Commands::Test {
            path,
            list,
            filter,
            no_color,
        } => {
            if no_color {
                colored::control::set_override(false);
            }
            run_test_cmd(
                path.unwrap_or_else(|| PathBuf::from(KAO_TESTS)),
                list,
                filter.as_deref(),
            )
        }
        Commands::Run {
            path,
            entry,
            no_color,
            script_args,
        } => {
            if no_color {
                colored::control::set_override(false);
            }
            run_run_cmd(
                path.unwrap_or_else(|| PathBuf::from(".")),
                entry,
                script_args,
            )
        }
    }
}
