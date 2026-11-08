use colored::Colorize;
use std::{collections::HashMap, path::PathBuf};

use duka_lib::kao::{collect_sources, find_kao};

pub fn run_check_cmd(root: PathBuf, file: Option<PathBuf>) -> i32 {
    let kao = find_kao(&root).ok();
    let config = kao
        .as_ref()
        .and_then(|k| k.manifest())
        .map(duka_lib::kao::project_config)
        .unwrap_or_default();

    let mut errors: HashMap<PathBuf, Box<dyn std::error::Error>> = HashMap::new();
    let mut targets = 0usize;

    if let Some(file) = file {
        targets = 1;
        if let Err(e) = duka_lib::module::compile_file(&file, config) {
            errors.insert(file, e);
        }
    } else if let Some(kao) = kao {
        let files = match collect_sources(&kao, &root) {
            Ok(files) => files,
            Err(e) => {
                eprintln!("{}: {e}", "error".red().bold());
                return 1;
            }
        };
        targets = files.len();
        for file in files {
            if let Err(e) = duka_lib::module::compile_file(&file, config.clone()) {
                errors.insert(file, e);
            }
        }
    }

    let error_count = errors.len();
    for (file, err) in errors {
        println!("=== file {} ===", file.display());
        println!("{}: {err}", "check error".on_red());
    }

    if error_count > 0 {
        println!(
            "{}: checked {targets}, found {error_count} errors",
            "check failed".red().bold()
        );
        1
    } else {
        println!(
            "{}: checked {targets}, no error found",
            "check done".green().bold()
        );
        0
    }
}
