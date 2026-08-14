//! Commandline tool for duka
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::build::run_build_cmd;
use crate::init::run_init_cmd;
use crate::test::run_test_cmd;

const VERSION: &str = "0.1.0";

mod build;
mod init;
mod test;

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
}

fn main() {
    let args = Args::parse();
    let exit = match args.cmd {
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
        Commands::Build { path, list } => {
            run_build_cmd(path.unwrap_or_else(|| PathBuf::from(".")), list)
        }
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
                path.unwrap_or_else(|| PathBuf::from("./tests")),
                list,
                filter.as_deref(),
            )
        }
    };
    std::process::exit(exit);
}
