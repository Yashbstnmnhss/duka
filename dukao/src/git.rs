use std::path::Path;
use std::process::Command;

pub const ALLOWED_PROTOCOLS: &[&str] = &["https://", "git://"];

pub fn validate_url(url: &str) -> Result<(), String> {
    let has_scheme = url.contains("://") || url.starts_with("git@");
    if !has_scheme || ALLOWED_PROTOCOLS.iter().any(|p| url.starts_with(p)) {
        Ok(())
    } else {
        Err(format!(
            "unsupported protocol in '{url}', allowed: {} or local path",
            ALLOWED_PROTOCOLS.join(", ")
        ))
    }
}

fn run_git(cwd: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn clone_bare(url: &str, cache_path: &Path) -> Result<(), String> {
    validate_url(url)?;
    if cache_path.exists() {
        return Ok(());
    }
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    run_git(
        None,
        &["clone", "--bare", url, &cache_path.to_string_lossy()],
    )
    .map(|_| ())
}

pub fn checkout_to(
    cache_path: &Path,
    tag: Option<&str>,
    branch: Option<&str>,
    target: &Path,
) -> Result<(), String> {
    let ref_name = tag.or(branch).unwrap_or("HEAD");
    let _ = run_git(
        None,
        &[
            "--git-dir",
            &cache_path.to_string_lossy(),
            "worktree",
            "prune",
        ],
    );
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|e| format!("cleanup failed: {e}"))?;
    }
    std::fs::create_dir_all(target).map_err(|e| format!("mkdir failed: {e}"))?;

    run_git(
        None,
        &[
            "--git-dir",
            &cache_path.to_string_lossy(),
            "worktree",
            "add",
            "--detach",
            &target.to_string_lossy(),
            ref_name,
        ],
    )
    .map(|_| ())
}

pub fn resolve_rev(cache_path: &Path, ref_name: &str) -> Result<String, String> {
    run_git(
        None,
        &[
            "--git-dir",
            &cache_path.to_string_lossy(),
            "rev-parse",
            ref_name,
        ],
    )
}
