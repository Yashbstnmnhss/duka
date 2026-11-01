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
        ".git" | ".gitignore" | "kao.lock.toml" | "tests" | "build" | "modules"
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

/// How a lock file spells a path source: relative to the project, so the same
/// checkout installs on another machine.
fn lock_path_str(project_root: &Path, resolved: &Path) -> String {
    let root = duka_lib::duka_shared::module::normalize(project_root);
    let abs = duka_lib::duka_shared::module::normalize(resolved);
    match abs.strip_prefix(&root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => relative_path(&root, &abs),
    }
}

fn relative_path(root: &Path, target: &Path) -> String {
    let rc: Vec<_> = root.components().collect();
    let tc: Vec<_> = target.components().collect();
    if rc.first() != tc.first() || rc.first().is_none() {
        return target.to_string_lossy().replace('\\', "/");
    }
    let common = rc.iter().zip(tc.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_owned(); rc.len() - common];
    parts.extend(
        tc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn read_optional_manifest(project_root: &Path) -> Option<KaoManifest> {
    let content = std::fs::read_to_string(project_root.join(KAO_FILE)).ok()?;
    toml::from_str(&content).ok()
}

/// Install `deps` into the project's modules dir, then keep going: every
/// installed package's own manifest is read and its dependencies installed
/// alongside it (flat, npm-style), deduped by package name.
fn modules_dir_of(project_root: &Path) -> PathBuf {
    project_root.join(
        read_optional_manifest(project_root)
            .and_then(|m| m.build.modules_dir)
            .unwrap_or_else(|| DEFAULT_MODULES_DIR.to_owned()),
    )
}

fn clear_deps(project_root: &Path) -> i32 {
    let modules_dir = modules_dir_of(project_root);
    if modules_dir.is_dir()
        && let Err(e) = prune_modules(&modules_dir, std::iter::empty())
    {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }
    let lock_path = project_root.join(crate::KAO_LOCK_FILE);
    if lock_path.exists()
        && let Err(e) = std::fs::remove_file(&lock_path)
    {
        eprintln!("{}: {e}", "error".red().bold());
        return 2;
    }
    println!("no dependencies");
    0
}

pub fn install_deps(
    project_root: &Path,
    deps: &HashMap<String, Dependency>,
    lock: Option<&LockFile>,
    require_lock: bool,
    offline: bool,
) -> Result<Vec<LockEntry>, String> {
    let modules_dir = modules_dir_of(project_root);
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
        let locked = locked_entry(lock, &dep);
        if require_lock && matches!(dep.source, DependencySource::Git { .. }) && locked.is_none() {
            return Err(format!(
                "kao.lock.toml is out of date for '{name}'; run `dukao update`"
            ));
        }
        let entry = install_one(project_root, &name, &dep, &base, &dest, locked, offline)?;
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
    prune_modules(&modules_dir, installed.keys())?;
    Ok(entries)
}

fn locked_entry<'a>(lock: Option<&'a LockFile>, dep: &Dependency) -> Option<&'a LockEntry> {
    let DependencySource::Git {
        url,
        tag,
        branch,
        version,
        ..
    } = &dep.source
    else {
        return None;
    };
    lock?.packages.iter().find(|p| {
        p.url.as_deref() == Some(url.as_str())
            && p.version == *version
            && (tag.is_none() || p.tag == *tag)
            && (branch.is_none() || p.branch == *branch)
    })
}

fn prune_modules<'a>(
    modules_dir: &Path,
    keep: impl Iterator<Item = &'a String>,
) -> Result<(), String> {
    let keep: std::collections::HashSet<&str> = keep.map(|k| k.as_str()).collect();
    let entries = std::fs::read_dir(modules_dir).map_err(|e| format!("read_dir failed: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read_dir entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".gitignore" || keep.contains(name.as_str()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| format!("prune failed: {e}"))?;
        } else {
            std::fs::remove_file(&path).map_err(|e| format!("prune failed: {e}"))?;
        }
    }
    Ok(())
}

fn install_one(
    project_root: &Path,
    name: &str,
    dep: &Dependency,
    base: &Path,
    dest: &Path,
    locked: Option<&LockEntry>,
    offline: bool,
) -> Result<LockEntry, String> {
    match &dep.source {
        DependencySource::Path(path) => {
            let src_path = base.join(path);
            if !src_path.exists() {
                return Err(format!("path dependency '{name}' not found at '{path}'"));
            }
            if duka_lib::duka_shared::module::normalize(&src_path)
                == duka_lib::duka_shared::module::normalize(dest)
            {
                return Err(format!(
                    "dependency '{name}' points at its own install location"
                ));
            }
            clean_dest(dest)?;
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
            if offline {
                if !cache.exists() {
                    return Err(format!(
                        "offline: no cached clone for '{url}', run `dukao install` first"
                    ));
                }
            } else {
                crate::git::clone_bare(url, &cache)?;
            }
            // the recorded rev wins: a frozen install must reproduce exactly
            let (ref_name, resolved_tag) = if let Some(r) = rev.as_deref() {
                (r.to_owned(), tag.clone())
            } else if let Some(entry) = locked {
                (
                    entry
                        .rev
                        .clone()
                        .or_else(|| entry.tag.clone())
                        .unwrap_or_else(|| "HEAD".to_owned()),
                    entry.tag.clone().or_else(|| tag.clone()),
                )
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
            clean_dest(dest)?;
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

fn clean_dest(dest: &Path) -> Result<(), String> {
    if dest.is_dir() {
        std::fs::remove_dir_all(dest).map_err(|e| format!("cleanup failed: {e}"))?;
    }
    Ok(())
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

pub fn run_install_cmd(project_root: PathBuf, frozen: bool, offline: bool) -> i32 {
    let lock = read_lock(&project_root);
    if frozen && lock.is_none() {
        eprintln!("{}: kao.lock.toml not found", "error".red().bold());
        return 2;
    }
    let deps = match read_manifest_deps(&project_root) {
        Ok(deps) => deps,
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            return 2;
        }
    };
    if deps.is_empty() {
        return clear_deps(&project_root);
    }
    finish_install(
        project_root,
        deps,
        lock.as_ref(),
        frozen,
        offline,
        if frozen { "from lock" } else { "installed" },
    )
}

pub fn run_update_cmd(project_root: PathBuf, offline: bool) -> i32 {
    let deps = match read_manifest_deps(&project_root) {
        Ok(deps) => deps,
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            return 2;
        }
    };
    if deps.is_empty() {
        return clear_deps(&project_root);
    }
    finish_install(project_root, deps, None, false, offline, "updated")
}

fn finish_install(
    project_root: PathBuf,
    deps: HashMap<String, Dependency>,
    lock: Option<&LockFile>,
    require_lock: bool,
    offline: bool,
    note: &str,
) -> i32 {
    match install_deps(&project_root, &deps, lock, require_lock, offline) {
        Ok(entries) => {
            let lock = LockFile { packages: entries };
            if let Err(e) = write_lock(&project_root, &lock) {
                eprintln!("{}: {e}", "error".red().bold());
                return 2;
            }
            println!(
                "{} {} {}",
                "✔".green(),
                lock.packages.len(),
                match note {
                    "installed" => "dependencies installed".to_owned(),
                    "updated" => "dependencies updated".to_owned(),
                    other => format!("dependencies installed {other}"),
                }
            );
            0
        }
        Err(e) => {
            eprintln!("{}: {e}", "error".red().bold());
            2
        }
    }
}
