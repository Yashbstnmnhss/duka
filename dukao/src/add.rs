use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::install;
use duka_lib::kao::{KAO_FILE, validate_dep_name};

pub fn run_add_cmd(
    project_root: PathBuf,
    spec: Option<String>,
    path: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    version: Option<String>,
    as_name: Option<String>,
) -> i32 {
    let (name, dep_line) = match &path {
        Some(dir) => {
            if tag.is_some() || branch.is_some() || version.is_some() {
                eprintln!(
                    "{}: --tag/--branch/--version only apply to git dependencies",
                    "error".red().bold()
                );
                return 2;
            }
            let name = match &as_name {
                Some(n) => n.clone(),
                None => spec.clone().unwrap_or_else(|| {
                    Path::new(dir)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "pkg".to_owned())
                }),
            };
            (name, format!("{{ path = \"{dir}\" }}"))
        }
        None => {
            let Some(url) = spec else {
                eprintln!(
                    "{}: expected a git url, or --path <dir> for a local package",
                    "error".red().bold()
                );
                return 2;
            };
            let name = match &as_name {
                Some(n) => n.clone(),
                None => extract_name_from_url(&url),
            };
            let url = url.replace('\\', "/");
            if let Err(e) = crate::git::validate_url(&url) {
                eprintln!("{}: {e}", "error".red().bold());
                return 2;
            }
            if version.is_some() && (tag.is_some() || branch.is_some()) {
                eprintln!(
                    "{}: --version cannot be combined with --tag/--branch",
                    "error".red().bold()
                );
                return 2;
            }
            if let Some(v) = &version
                && let Err(e) = duka_lib::semver::VersionReq::parse(v)
            {
                eprintln!("{}: invalid --version: {e}", "error".red().bold());
                return 2;
            }
            let dep_line = match (&tag, &branch, &version) {
                (Some(t), _, _) => format!("{{ git = \"{url}\", tag = \"{t}\" }}"),
                (_, Some(b), _) => format!("{{ git = \"{url}\", branch = \"{b}\" }}"),
                (_, _, Some(v)) => format!("{{ git = \"{url}\", version = \"{v}\" }}"),
                _ => format!("{{ git = \"{url}\" }}"),
            };
            (name, dep_line)
        }
    };
    if let Err(e) = validate_name(&name) {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }

    let kao_path = project_root.join(KAO_FILE);
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
    validate_dep_name(name)
}

fn append_dependency(kao_path: &Path, name: &str, dep_line: &str) -> Result<bool, String> {
    let content = std::fs::read_to_string(kao_path).map_err(|e| format!("read kao.toml: {e}"))?;
    let mut lines: Vec<String> = content.lines().map(|l| l.to_owned()).collect();

    if let Some(idx) = section_index(&lines, "dependencies") {
        let end = section_end(&lines, idx);
        for line in &lines[idx + 1..end] {
            if duka_lib::kao::dep_line_name(line) == Some(name) {
                if line.trim_end().ends_with(dep_line) {
                    return Ok(false);
                }
                return Err(format!(
                    "dependency '{name}' already exists with a different source"
                ));
            }
        }
        // insert right before the section ends, not at the end of the file
        lines.insert(end, format!("{name} = {dep_line}"));
    } else {
        if !lines.is_empty() && !lines.last().map(|l| l.trim().is_empty()).unwrap_or(true) {
            lines.push(String::new());
        }
        lines.push("[dependencies]".to_owned());
        lines.push(format!("{name} = {dep_line}"));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(kao_path, out).map_err(|e| format!("write kao.toml: {e}"))?;
    Ok(true)
}

/// Line index of a `[section]` header, tolerant of a trailing comment.
fn section_index(lines: &[String], section: &str) -> Option<usize> {
    lines.iter().position(|l| {
        let t = l.trim_start();
        let Some(rest) = t.strip_prefix('[') else {
            return false;
        };
        let Some(rest) = rest.strip_prefix(section) else {
            return false;
        };
        rest.starts_with(']') || rest.trim_start().starts_with('#')
    })
}

/// Index where a new line belongs inside the section at `idx`.
fn section_end(lines: &[String], idx: usize) -> usize {
    let mut i = idx + 1;
    while i < lines.len() && !lines[i].trim_start().starts_with('[') {
        i += 1;
    }
    i
}
