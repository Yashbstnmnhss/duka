use colored::Colorize;
use std::path::PathBuf;

pub fn run_remove_cmd(project_root: PathBuf, name: String) -> i32 {
    let kao_path = project_root.join("kao.toml");

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

    let modules_dir = project_root.join("modules").join(&name);
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
        if in_deps && trimmed.starts_with(name) && trimmed.contains('=') {
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
