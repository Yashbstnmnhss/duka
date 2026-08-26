//! Kao (project) discovery and manifest handling for duka
//!
//! In default, `kao.toml` is the manifest file for a kao (project), See examples/

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use duka_shared::{config::DukaConfig, constants::SOURCE_SUFFIX};
use serde::{Deserialize, Serialize};

pub const KAO_FILE: &str = "kao.toml";
pub const KAO_LOCK_FILE: &str = "kao.lock.toml";
pub const DEFAULT_SRC: &str = "src";
pub const DEFAULT_ENTRY: &str = "main.duka";
pub const DEFAULT_OUT_DIR: &str = "build";
pub const DEFAULT_MODULES_DIR: &str = "modules";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KaoManifest {
    #[serde(rename = "kao", default)]
    pub meta: KaoMeta,
    #[serde(rename = "build", default)]
    pub build: BuildConfig,
    #[serde(rename = "dependencies", default)]
    pub dependencies: HashMap<String, Dependency>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KaoMeta {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    pub entry: Option<String>,
    pub out_dir: Option<String>,
    pub src_dir: Option<String>,
    pub modules_dir: Option<String>,
    pub config: Option<DukaConfig>,
}

#[derive(Debug, Clone)]
pub enum DependencySource {
    Git {
        url: String,
        tag: Option<String>,
        branch: Option<String>,
    },
    Path(String),
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub source: DependencySource,
}

impl<'de> Deserialize<'de> for Dependency {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            git: Option<String>,
            tag: Option<String>,
            branch: Option<String>,
            path: Option<String>,
        }
        let raw = Raw::deserialize(d)?;
        if let Some(url) = raw.git {
            Ok(Dependency {
                source: DependencySource::Git {
                    url,
                    tag: raw.tag,
                    branch: raw.branch,
                },
            })
        } else if let Some(path) = raw.path {
            Ok(Dependency {
                source: DependencySource::Path(path),
            })
        } else {
            Err(serde::de::Error::custom(
                "dependency must have either `git` or `path`",
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub name: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockFile {
    #[serde(rename = "package", default)]
    pub packages: Vec<LockEntry>,
}

/// A resolved duka project (kao)
#[derive(Debug, Clone)]
pub struct Kao {
    root: PathBuf,
    manifest: Option<KaoManifest>,
}

impl Kao {
    /// Project root directory
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> Option<&KaoManifest> {
        self.manifest.as_ref()
    }

    pub fn name(&self) -> Option<&str> {
        self.manifest.as_ref().and_then(|m| m.meta.name.as_deref())
    }

    /// Entry script, default `main.duka`, can be modified in kao.toml
    pub fn entry(&self) -> PathBuf {
        self.manifest
            .as_ref()
            .and_then(|m| m.build.entry.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SRC).join(DEFAULT_ENTRY))
    }
    /// Output directory for build artifacts, default `build`
    pub fn out_dir(&self) -> PathBuf {
        self.manifest
            .as_ref()
            .and_then(|m| m.build.out_dir.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_OUT_DIR))
    }

    /// Source directory for the kao, default `src`
    pub fn src_dir(&self) -> PathBuf {
        self.manifest
            .as_ref()
            .and_then(|m| m.build.src_dir.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_SRC))
    }

    /// Directory holding `require()`-able modules, default `modules`
    pub fn modules_dir(&self) -> PathBuf {
        self.manifest
            .as_ref()
            .and_then(|m| m.build.modules_dir.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_MODULES_DIR))
    }
}

/// Looking for kao from start path
pub fn find_kao(start: &Path) -> Result<Kao, String> {
    let start = if start.is_file() {
        start.parent().unwrap_or(Path::new("."))
    } else {
        start
    };
    let mut dir = Some(start);
    while let Some(cur) = dir {
        let manifest_path = cur.join(KAO_FILE);
        if manifest_path.is_file() {
            let content = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
            let manifest: KaoManifest = toml::from_str(&content).map_err(|e| e.to_string())?;
            return Ok(Kao {
                root: cur.to_path_buf(),
                manifest: Some(manifest),
            });
        }
        dir = cur.parent();
    }
    Ok(Kao {
        root: start.to_path_buf(),
        manifest: None,
    })
}

/// Recursively collect duka source files for project kao under dir
pub fn collect_sources(kao: &Kao, dir: &Path) -> Result<Vec<PathBuf>, String> {
    let out_dir = kao.root().join(kao.out_dir());
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let entries = std::fs::read_dir(&cur).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path == out_dir || path.starts_with(&out_dir) {
                    continue;
                }
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some(SOURCE_SUFFIX) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}
