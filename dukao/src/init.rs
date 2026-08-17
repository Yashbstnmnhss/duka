use std::fs;
use std::path::{Path, PathBuf};

use colored::Colorize;
use duka_lib::kao::{DEFAULT_ENTRY, DEFAULT_MODULES_DIR, DEFAULT_SRC, KAO_FILE};

pub fn run_init_cmd(
    path: PathBuf,
    name: Option<String>,
    version: Option<String>,
    force: bool,
) -> i32 {
    let root = if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path
    };

    if let Err(e) = fs::create_dir_all(&root) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }

    let manifest_path = root.join(KAO_FILE);
    if manifest_path.exists() && !force {
        eprintln!(
            "{}: {} already exists, use --force to overwrite",
            "error".red().bold(),
            manifest_path.display()
        );
        return 2;
    }

    let dir_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("duka")
        .to_string();
    let name = name.unwrap_or(dir_name);
    let version = version.unwrap_or_else(|| "0.1.0".to_string());

    let manifest = format!(
        r#"[kao]
name = "{name}"
version = "{version}"
"#
    );
    if let Err(e) = fs::write(&manifest_path, manifest) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }
    println!("{} {}", "created".green().bold(), manifest_path.display());

    let src_dir = root.join(DEFAULT_SRC);
    if let Err(e) = fs::create_dir_all(&src_dir) {
        eprintln!("{}: {}", "error".red().bold(), e);
        return 2;
    }
    let entry_path = src_dir.join(DEFAULT_ENTRY);
    if !entry_path.exists() || force {
        if let Err(e) = fs::write(&entry_path, "print(\"Hello, World!\")\n") {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
        println!("{} {}", "created".green().bold(), entry_path.display());
    }

    for dir in [root.join(DEFAULT_MODULES_DIR), root.join("tests")] {
        if !dir.exists() {
            if let Err(e) = fs::create_dir_all(&dir) {
                eprintln!("{}: {}", "error".red().bold(), e);
                return 2;
            }
            println!("{} {}", "created".green().bold(), dir.display());
        }
    }

    let gitignore_path = root.join(".gitignore");
    if !gitignore_path.exists() || force {
        if let Err(e) = fs::write(
            &gitignore_path,
            r#"modules/
build/
"#,
        ) {
            eprintln!("{}: {}", "error".red().bold(), e);
            return 2;
        }
        println!("{} {}", "created".green().bold(), gitignore_path.display());
    }

    0
}
