//! Commandline tool for duka
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::build::{BuildTarget, run_build_cmd};
use crate::init::run_init_cmd;
use crate::run::run_run_cmd;
use crate::test::run_test_cmd;

const VERSION: &str = "0.1.0";

mod add;
mod build;
mod diag;
mod git;
mod init;
mod install;
mod remove;
mod run;
mod serve;
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
    /// Build the current kao project (entry + modules) to targets
    Build {
        #[command(subcommand)]
        /// Build target, default for compiled duka
        target: Option<BuildTarget>,
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
    /// Install dependencies from kao.toml
    Install {
        /// Project root (defaults to nearest kao.toml)
        path: Option<PathBuf>,
        /// Strictly follow kao.lock.toml without updating
        #[arg(long, short)]
        frozen: bool,
    },
    /// Add a git dependency
    Add {
        /// Git repository URL
        url: String,
        /// Git tag to pin
        #[arg(long)]
        tag: Option<String>,
        /// Git branch to track
        #[arg(long)]
        branch: Option<String>,
        /// Local package name (default: extracted from URL)
        #[arg(long = "as")]
        as_name: Option<String>,
    },
    /// Remove a dependency
    Remove { name: String },
    /// Serve build output over HTTP
    Serve {
        /// Directory to serve (default: `build`)
        dir: Option<PathBuf>,
        /// Port number
        #[arg(long, short, default_value_t = 3000)]
        port: u16,
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
        Commands::Build { path, list, target } => run_build_cmd(
            path.unwrap_or_else(|| PathBuf::from(".")),
            list,
            target.unwrap_or_default(),
        ),
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
        Commands::Install { path, frozen } => install::run_install_cmd(find_kao_root(path), frozen),
        Commands::Add {
            url,
            tag,
            branch,
            as_name,
        } => add::run_add_cmd(find_kao_root(None), url, tag, branch, as_name),
        Commands::Remove { name } => remove::run_remove_cmd(find_kao_root(None), name),
        Commands::Serve { dir, port } => {
            serve::run_serve_cmd(dir.unwrap_or_else(|| PathBuf::from("build")), port)
        }
    }
}

fn find_kao_root(path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = path {
        return p;
    }
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if cur.join(duka_lib::kao::KAO_FILE).exists() {
            return cur;
        }
        if !cur.pop() {
            return PathBuf::from(".");
        }
    }
}
