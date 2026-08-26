use std::path::{Path, PathBuf};

use colored::Colorize;

use duka_lib::kao::{
    DEFAULT_MODULES_DIR, Dependency, DependencySource, KaoManifest, LockEntry, LockFile,
};

pub fn cache_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".dukao").join("cache")
}

fn is_excluded(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".gitignore" | "kao.toml" | "kao.lock.toml" | "tests" | "build"
    )
}

fn is_unsafe_rel(rel: &Path) -> bool {
    rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    })
}

fn copy_pkg_files(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir failed: {e}"))?;
    let mut stack = vec![src.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let entries = std::fs::read_dir(&cur).map_err(|e| format!("read_dir failed: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
            let path = entry.path();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if is_excluded(&file_name) {
                continue;
            }
            let rel = path.strip_prefix(src).unwrap_or(&path);
            if is_unsafe_rel(rel) {
                return Err(format!(
                    "unsafe path in package: '{}' escapes destination",
                    rel.display()
                ));
            }
            let target = dest.join(rel);
            if path.is_dir() {
                std::fs::create_dir_all(&target).map_err(|e| format!("mkdir failed: {e}"))?;
                stack.push(path);
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
                }
                std::fs::copy(&path, &target).map_err(|e| format!("copy failed: {e}"))?;
            }
        }
    }
    Ok(())
}

pub fn install_deps(
    project_root: &Path,
    deps: &std::collections::HashMap<String, Dependency>,
) -> Result<Vec<LockEntry>, String> {
    let modules_dir = project_root.join(DEFAULT_MODULES_DIR);
    std::fs::create_dir_all(&modules_dir).map_err(|e| format!("mkdir failed: {e}"))?;

    let mut entries = vec![];

    for (name, dep) in deps {
        let dest = modules_dir.join(name);
        match &dep.source {
            DependencySource::Path(path) => {
                let src_path = project_root.join(path);
                if !src_path.exists() {
                    return Err(format!("path dependency '{name}' not found at '{path}'"));
                }
                copy_pkg_files(&src_path, &dest)?;
                entries.push(LockEntry {
                    name: name.clone(),
                    source: "path".into(),
                    url: None,
                    tag: None,
                    rev: None,
                    path: Some(path.clone()),
                });
            }
            DependencySource::Git { url, tag, branch } => {
                crate::git::validate_url(url)?;
                let safe_name = name.replace('/', "__");
                let cache = cache_dir().join(&safe_name);
                let worktree = cache.with_extension("worktree");
                crate::git::clone_bare(url, &cache)?;
                crate::git::checkout_to(&cache, tag.as_deref(), branch.as_deref(), &worktree)?;
                copy_pkg_files(&worktree, &dest)?;
                let _ = std::fs::remove_dir_all(&worktree);
                let rev = crate::git::resolve_rev(
                    &cache,
                    tag.as_deref().or(branch.as_deref()).unwrap_or("HEAD"),
                )?;

                entries.push(LockEntry {
                    name: name.clone(),
                    source: "git".into(),
                    url: Some(url.clone()),
                    tag: tag.clone(),
                    rev: Some(rev),
                    path: None,
                });
            }
        }
    }

    write_gitignore(&modules_dir)?;
    Ok(entries)
}

fn write_gitignore(modules_dir: &Path) -> Result<(), String> {
    let ignore = modules_dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n!.gitignore\n")
            .map_err(|e| format!("write .gitignore: {e}"))?;
    }
    Ok(())
}

pub fn read_lock(project_root: &Path) -> Option<LockFile> {
    let lock_path = project_root.join(crate::KAO_LOCK_FILE);
    let content = std::fs::read_to_string(lock_path).ok()?;
    toml::from_str(&content).ok()
}

pub fn write_lock(project_root: &Path, lock: &LockFile) -> Result<(), String> {
    let lock_path = project_root.join(crate::KAO_LOCK_FILE);
    let content = toml::to_string_pretty(lock).map_err(|e| format!("serialize lock: {e}"))?;
    std::fs::write(&lock_path, content).map_err(|e| format!("write lock: {e}"))
}

pub fn read_manifest_deps(
    project_root: &Path,
) -> Result<std::collections::HashMap<String, Dependency>, String> {
    let manifest_path = project_root.join(duka_lib::kao::KAO_FILE);
    let content =
        std::fs::read_to_string(manifest_path).map_err(|e| format!("read kao.toml: {e}"))?;
    let manifest: KaoManifest =
        toml::from_str(&content).map_err(|e| format!("parse kao.toml: {e}"))?;
    Ok(manifest.dependencies)
}

pub fn run_install_cmd(project_root: PathBuf, frozen: bool) -> i32 {
    if frozen {
        let Some(lock) = read_lock(&project_root) else {
            eprintln!("{}: kao.lock.toml not found", "error".red().bold());
            return 2;
        };
        let mut deps = std::collections::HashMap::new();
        for pkg in &lock.packages {
            let source = match (&pkg.url, &pkg.path) {
                (Some(url), _) => DependencySource::Git {
                    url: url.clone(),
                    tag: pkg.tag.clone(),
                    branch: None,
                },
                (None, Some(path)) => DependencySource::Path(path.clone()),
                _ => continue,
            };
            deps.insert(pkg.name.clone(), Dependency { source });
        }
        return finish_install(project_root, deps, "from lock");
    }

    match read_manifest_deps(&project_root) {
        Ok(deps) => {
            if deps.is_empty() {
                println!("no dependencies");
                return 0;
            }
            finish_install(project_root, deps, "")
        }
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            2
        }
    }
}

fn finish_install(
    project_root: PathBuf,
    deps: std::collections::HashMap<String, Dependency>,
    note: &str,
) -> i32 {
    match install_deps(&project_root, &deps) {
        Ok(entries) => {
            let lock = LockFile { packages: entries };
            if let Err(e) = write_lock(&project_root, &lock) {
                eprintln!("{}: {e}", "error".red().bold());
                return 2;
            }
            println!(
                "{} {} dependencies installed {}",
                "✔".green(),
                lock.packages.len(),
                note
            );
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            2
        }
    }
}
