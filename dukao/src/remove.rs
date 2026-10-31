use colored::Colorize;
use std::path::PathBuf;

use crate::install::{read_lock, write_lock};
use duka_lib::kao::{DEFAULT_MODULES_DIR, KAO_FILE, KaoManifest, validate_dep_name};

pub fn run_remove_cmd(project_root: PathBuf, name: String) -> i32 {
    if let Err(e) = validate_dep_name(&name) {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }
    let kao_path = project_root.join(KAO_FILE);

    match remove_from_toml(&kao_path, &name) {
        Ok(()) => {
            println!(
                "{} removed {} from [dependencies]",
                "✔".green(),
                name.green()
            );
        }
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            return 2;
        }
    }

    let modules_dir_name = std::fs::read_to_string(&kao_path)
        .ok()
        .and_then(|c| toml::from_str::<KaoManifest>(&c).ok())
        .and_then(|m| m.build.modules_dir)
        .unwrap_or_else(|| DEFAULT_MODULES_DIR.to_owned());
    let modules_dir = project_root.join(modules_dir_name).join(&name);
    if modules_dir.exists() {
        match std::fs::remove_dir_all(&modules_dir) {
            Ok(()) => println!("{} removed modules/{name}/", "✔".green()),
            Err(e) => {
                eprintln!(
                    "{}: failed to remove modules/{name}: {e}",
                    "error".red().bold()
                );
                return 2;
            }
        }
    }

    // the lock has to follow, or a frozen install would bring it straight back
    if let Some(mut lock) = read_lock(&project_root) {
        let before = lock.packages.len();
        lock.packages.retain(|p| p.name != name);
        if lock.packages.len() != before
            && let Err(e) = write_lock(&project_root, &lock)
        {
            eprintln!("{}: {e}", "error".red().bold());
            return 2;
        }
    }
    0
}

fn remove_from_toml(kao_path: &std::path::Path, name: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(kao_path).map_err(|e| format!("read kao.toml: {e}"))?;
    let mut in_deps = false;
    let mut out_lines = vec![];
    let mut removed = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]";
        }
        if in_deps && duka_lib::kao::dep_line_name(line) == Some(name) {
            removed = true;
            continue;
        }
        out_lines.push(line);
    }

    if !removed {
        return Err(format!("dependency '{name}' not found in kao.toml"));
    }
    std::fs::write(kao_path, out_lines.join("\n") + "\n")
        .map_err(|e| format!("write kao.toml: {e}"))
}
