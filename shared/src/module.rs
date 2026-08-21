use std::path::{Component, Path, PathBuf};

use crate::constants::{COMPILED_SUFFIX, SOURCE_SUFFIX};

pub fn is_relative_name(name: &str) -> bool {
    name.starts_with("./") || name.starts_with("../") || name == "." || name == ".."
}

pub fn normalize_name(name: &str) -> String {
    name.replace('.', "/")
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn module_candidates(base: &Path) -> Vec<String> {
    let b = base.to_string_lossy().replace('\\', "/");
    let mut out = Vec::new();
    for ext in [SOURCE_SUFFIX, COMPILED_SUFFIX] {
        out.push(format!("{b}.{ext}"));
    }
    for ext in [SOURCE_SUFFIX, COMPILED_SUFFIX] {
        out.push(format!("{b}/init.{ext}"));
    }
    out.push(b);
    out
}

pub fn package_candidates(templates: &[String], name: &str) -> Vec<String> {
    let n = normalize_name(name);
    templates.iter().map(|t| t.replace('?', &n)).collect()
}

pub fn relative_candidates(name: &str, caller_dir: &Path) -> Vec<String> {
    let joined = caller_dir.join(Path::new(name));
    let mut out = module_candidates(&joined);
    out.push(joined.to_string_lossy().replace('\\', "/"));
    out
}