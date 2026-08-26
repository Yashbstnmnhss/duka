use colored::Colorize;
use std::path::PathBuf;

use crate::install;

pub fn run_add_cmd(
    project_root: PathBuf,
    url: String,
    tag: Option<String>,
    branch: Option<String>,
    as_name: Option<String>,
) -> i32 {
    let name = match &as_name {
        Some(n) => n.clone(),
        None => extract_name_from_url(&url),
    };
    if let Err(e) = validate_name(&name) {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }

    let url = url.replace('\\', "/");
    if let Err(e) = crate::git::validate_url(&url) {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }
    let dep_line = match (&tag, &branch) {
        (Some(t), _) => format!("{{ git = \"{url}\", tag = \"{t}\" }}"),
        (_, Some(b)) => format!("{{ git = \"{url}\", branch = \"{b}\" }}"),
        _ => format!("{{ git = \"{url}\" }}"),
    };

    let kao_path = project_root.join("kao.toml");
    match append_dependency(&kao_path, &name, &dep_line) {
        Ok(true) => println!("{} added {} to [dependencies]", "✔".green(), name.green()),
        Ok(false) => println!("{} {} already configured, installing", "•".yellow(), name),
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            return 2;
        }
    }

    install::run_install_cmd(project_root, false)
}

fn extract_name_from_url(url: &str) -> String {
    url.trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .to_owned()
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("package name cannot be empty".into());
    }
    for c in name.chars() {
        if !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
            return Err(format!("invalid character '{c}' in package name"));
        }
    }
    Ok(())
}

fn append_dependency(
    kao_path: &std::path::Path,
    name: &str,
    dep_line: &str,
) -> Result<bool, String> {
    let content = std::fs::read_to_string(kao_path).map_err(|e| format!("read kao.toml: {e}"))?;
    if content.contains("[dependencies]") {
        for line in content.lines() {
            if line.trim_start().starts_with(name) && line.contains('=') {
                if line
                    .trim_start()
                    .trim_start_matches(name)
                    .trim_start_matches(|c| c == ' ' || c == '=')
                    .trim()
                    == dep_line
                {
                    return Ok(false);
                }
                return Err(format!(
                    "dependency '{name}' already exists with a different source"
                ));
            }
        }
        std::fs::write(kao_path, format!("{content}{name} = {dep_line}\n"))
            .map_err(|e| format!("write kao.toml: {e}"))?;
    } else {
        std::fs::write(
            kao_path,
            format!("{content}\n[dependencies]\n{name} = {dep_line}\n"),
        )
        .map_err(|e| format!("write kao.toml: {e}"))?;
    }
    Ok(true)
}
