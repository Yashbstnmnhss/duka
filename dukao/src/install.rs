use std::collections::HashMap;
use std::path::{Path, PathBuf};

use colored::Colorize;

use duka_lib::kao::{
    DEFAULT_MODULES_DIR, Dependency, DependencySource, KAO_FILE, KaoManifest, LockEntry, LockFile,
    validate_dep_name,
};

pub fn cache_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".dukao").join("cache")
}

/// Two repositories behind the same package name must never share a cache
/// entry, so the key carries the url, not just the name.
pub fn cache_key(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let hash = h.finish();
    let safe: String = url
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    format!("{safe}-{hash:016x}")
}

fn is_excluded(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".gitignore" | "kao.lock.toml" | "tests" | "build"
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

/// How a lock file spells a path source: relative to the project when the
/// source lives inside it, absolute otherwise, so a frozen install resolves
/// the same directory no matter which package declared it.
fn lock_path_str(project_root: &Path, resolved: &Path) -> String {
    let root = duka_lib::duka_shared::module::normalize(project_root);
    let abs = duka_lib::duka_shared::module::normalize(resolved);
    match abs.strip_prefix(&root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => abs.to_string_lossy().into_owned(),
    }
}

fn read_optional_manifest(project_root: &Path) -> Option<KaoManifest> {
    let content = std::fs::read_to_string(project_root.join(KAO_FILE)).ok()?;
    toml::from_str(&content).ok()
}

/// Install `deps` into the project's modules dir, then keep going: every
/// installed package's own manifest is read and its dependencies installed
/// alongside it (flat, npm-style), deduped by package name.
pub fn install_deps(
    project_root: &Path,
    deps: &HashMap<String, Dependency>,
) -> Result<Vec<LockEntry>, String> {
    let modules_dir = project_root.join(
        read_optional_manifest(project_root)
            .and_then(|m| m.build.modules_dir)
            .unwrap_or_else(|| DEFAULT_MODULES_DIR.to_owned()),
    );
    std::fs::create_dir_all(&modules_dir).map_err(|e| format!("mkdir failed: {e}"))?;
    let modules_norm = duka_lib::duka_shared::module::normalize(&modules_dir);

    let mut queue: Vec<(String, Dependency, PathBuf)> = deps
        .iter()
        .map(|(name, dep)| (name.clone(), dep.clone(), project_root.to_path_buf()))
        .collect();
    queue.sort_by(|a, b| a.0.cmp(&b.0));

    let mut installed: HashMap<String, String> = HashMap::new();
    let mut entries: Vec<LockEntry> = vec![];
    let mut cursor = 0;

    while cursor < queue.len() {
        let (name, dep, base) = queue[cursor].clone();
        cursor += 1;

        validate_dep_name(&name)?;
        // a path pointing at an installed package dir is a reference, not a dep
        if let DependencySource::Path(p) = &dep.source {
            let resolved = duka_lib::duka_shared::module::normalize(&base.join(p));
            if let Ok(rel) = resolved.strip_prefix(&modules_norm) {
                let mut comps = rel.components();
                let is_installed = match (comps.next(), comps.next()) {
                    (None, _) => true,
                    (Some(c), None) => {
                        installed.contains_key(c.as_os_str().to_string_lossy().as_ref())
                    }
                    _ => false,
                };
                if is_installed {
                    continue;
                }
            }
        }
        let fingerprint = dep.fingerprint(&base);
        if let Some(prev) = installed.get(&name) {
            if *prev == fingerprint {
                continue;
            }
            return Err(format!(
                "dependency '{name}' conflicts: '{prev}' vs '{fingerprint}'"
            ));
        }

        let dest = modules_dir.join(&name);
        let entry = install_one(project_root, &name, &dep, &base, &dest)?;
        // path packages resolve their own deps from their source dir, git ones
        // from the installed copy
        let nested_base = match &dep.source {
            DependencySource::Path(p) => duka_lib::duka_shared::module::normalize(&base.join(p)),
            DependencySource::Git { .. } => dest.clone(),
        };
        installed.insert(name.clone(), fingerprint);
        entries.push(entry);

        // npm-style: a package's own manifest brings its dependencies along
        if let Ok(content) = std::fs::read_to_string(dest.join(KAO_FILE))
            && let Ok(manifest) = toml::from_str::<KaoManifest>(&content)
        {
            let mut nested: Vec<(String, Dependency)> = manifest.dependencies.into_iter().collect();
            nested.sort_by(|a, b| a.0.cmp(&b.0));
            for (nested_name, nested_dep) in nested {
                queue.push((nested_name, nested_dep, nested_base.clone()));
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    write_gitignore(&modules_dir)?;
    Ok(entries)
}

fn install_one(
    project_root: &Path,
    name: &str,
    dep: &Dependency,
    base: &Path,
    dest: &Path,
) -> Result<LockEntry, String> {
    match &dep.source {
        DependencySource::Path(path) => {
            let src_path = base.join(path);
            if !src_path.exists() {
                return Err(format!("path dependency '{name}' not found at '{path}'"));
            }
            copy_pkg_files(&src_path, dest)?;
            Ok(LockEntry {
                name: name.to_owned(),
                source: "path".into(),
                url: None,
                tag: None,
                branch: None,
                rev: None,
                version: None,
                path: Some(lock_path_str(project_root, &src_path)),
            })
        }
        DependencySource::Git {
            url,
            tag,
            branch,
            rev,
            version,
        } => {
            crate::git::validate_url(url)?;
            if version.is_some() && (tag.is_some() || branch.is_some()) {
                return Err(format!(
                    "dependency '{name}': `version` cannot be combined with `tag` or `branch`"
                ));
            }
            let cache = cache_dir().join(cache_key(url));
            let worktree = cache.with_extension("worktree");
            crate::git::clone_bare(url, &cache)?;
            // the recorded rev wins: a frozen install must reproduce exactly
            let (ref_name, resolved_tag) = if let Some(r) = rev.as_deref() {
                (r.to_owned(), tag.clone())
            } else if let Some(v) = version.as_deref() {
                let req = duka_lib::semver::VersionReq::parse(v)
                    .map_err(|e| format!("dependency '{name}': {e}"))?;
                let tags = crate::git::list_tags(&cache)?;
                let (best, _) = duka_lib::semver::best_tag(tags.iter().map(String::as_str), &req)
                    .ok_or_else(|| {
                    format!("dependency '{name}': no tag matches version '{v}'")
                })?;
                (best.clone(), Some(best))
            } else {
                (
                    tag.as_deref()
                        .or(branch.as_deref())
                        .unwrap_or("HEAD")
                        .to_owned(),
                    tag.clone(),
                )
            };
            let pinned = crate::git::resolve_rev(&cache, &ref_name)?;
            crate::git::checkout_to(&cache, &pinned, &worktree)?;
            let copy = copy_pkg_files(&worktree, dest);
            let _ = std::fs::remove_dir_all(&worktree);
            copy?;
            Ok(LockEntry {
                name: name.to_owned(),
                source: "git".into(),
                url: Some(url.clone()),
                tag: resolved_tag,
                branch: branch.clone(),
                rev: Some(pinned),
                version: version.clone(),
                path: None,
            })
        }
    }
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

pub fn read_manifest_deps(project_root: &Path) -> Result<HashMap<String, Dependency>, String> {
    let manifest_path = project_root.join(KAO_FILE);
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
        let mut deps = HashMap::new();
        for pkg in &lock.packages {
            let source = match (&pkg.url, &pkg.path) {
                (Some(url), _) => DependencySource::Git {
                    url: url.clone(),
                    tag: pkg.tag.clone(),
                    branch: pkg.branch.clone(),
                    rev: pkg.rev.clone(),
                    version: pkg.version.clone(),
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

fn finish_install(project_root: PathBuf, deps: HashMap<String, Dependency>, note: &str) -> i32 {
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
