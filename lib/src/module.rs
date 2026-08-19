//! Default module loader helpers for `require()`.

use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use duka_backend::builtin::require::{self, LoadedModule};
use duka_backend::codegen::DefaultGenerator;
use duka_backend::codegen::binary::{DukaBinary, Dump, Load};
use duka_backend::value::DukaProto;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer, TypeChecker, TypeEval};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::LexerWithMacro;
use duka_frontend::parser::Parser;
use duka_shared::config::{DukaConfig, DukaIRConfig};
use duka_shared::constants::{COMPILED_SUFFIX, SOURCE_SUFFIX};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

pub fn compile_file(
    path: &Path,
    config: DukaConfig,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let source = std::fs::read_to_string(path)?;
    from_source(&source, path.to_str().map(|s| s.to_owned()), config)
}

pub fn proto_to_bytes(
    proto: &DukaProto,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(Vec::new());
    DukaBinary::new(proto.clone()).dump(&mut cursor)?;
    Ok(cursor.into_inner())
}

pub fn compile_to_bytes(
    path: &Path,
    config: DukaConfig,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let proto = compile_file(path, config)?;
    proto_to_bytes(&proto)
}

pub fn from_source(
    source: &str,
    name: Option<String>,
    config: DukaConfig,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let lexer = LexerWithMacro::new(Cursor::new(source), name, config.lexer);
    let stream = lexer.tokenize()?;
    let chunk = Parser::parse(stream, config.parser)?;

    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .chain(TypeEval)
        .chain(TypeChecker)
        .analyze(&chunk, config.analyzer)
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(Box::new(err));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);

    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )?;
    let proto = DefaultGenerator::generate(ir, ())?;
    Ok(proto)
}

/// Load an executable `DukaProto` from a file, dispatching on its suffix.
///
/// `{COMPILED_SUFFIX}` files hold pre-compiled bytecode and are read back
/// directly (skipping compilation); anything else is treated as `{SOURCE_SUFFIX}`
/// source and compiled on the fly.
pub fn load_proto(
    path: &Path,
    config: DukaConfig,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    if matches!(path.extension(), Some(t) if t == COMPILED_SUFFIX) {
        let mut file = File::open(path)?;
        let binary = DukaBinary::load(&mut file)?;
        Ok(binary.into_proto())
    } else {
        compile_file(path, config)
    }
}

/// Convert a `require` module name to a relative path: dots become `/`.
///
/// `require("foo.bar")` searches for `foo/bar`, like Lua/Node package names.
fn normalize_name(name: &str) -> String {
    name.replace('.', "/")
}

/// Resolve the search-path templates used by `file_loader`.
///
/// Uses the `<base_dir>/modules`
/// with the `?.duka`, `?.dukac`, `?/init.duka` and `?/init.dukac` templates, and
/// `DUKA_PATH` environment variable (`;`-separated templates, `?` is the module-name placeholder) if finds
pub fn search_paths(base_dir: &Path) -> Vec<String> {
    let mut res = vec![];
    let modules = base_dir.join("modules");
    res.push(format!("{}/?.{SOURCE_SUFFIX}", modules.display()));
    res.push(format!("{}/?.{COMPILED_SUFFIX}", modules.display()));
    res.push(format!("{}/?/init.{SOURCE_SUFFIX}", modules.display()));
    res.push(format!("{}/?/init.{COMPILED_SUFFIX}", modules.display()));
    if let Ok(env) = std::env::var("DUKA_PATH") {
        res.extend(env.split(';').map(|s| s.to_owned()));
    }
    res
}

/// # Duka File Loader (Module System)
/// Build a filesystem-backed loader that resolves each module name against a list of
/// search-path templates.
///
/// For every template the `?` placeholder is replaced with the normalized name
/// (`foo.bar` -> `foo/bar`); the first existing candidate is loaded (source files
/// are compiled, `COMPILED_SUFFIX` bytecode is read directly) and returned as a
/// compiled proto. `require()` executes the proto in the caller's VM.
///
/// Relative patterns (`./x`, `../y`) are resolved against the caller module's
/// directory (`caller_dir`), with the same `?.duka`/`?.dukac`/`?/init.*` template
/// chain plus an exact-file fallback for explicit extensions.
///
/// This is used by `duka_backend::builtin::require::set_loader`
pub fn file_loader(
    templates: impl IntoIterator<Item = String>,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    let templates: Vec<String> = templates.into_iter().collect();
    move |name, caller_dir| {
        if require::is_relative_name(name) {
            resolve_relative(name, caller_dir)
        } else {
            resolve_package(&templates, name)
        }
    }
}

fn resolve_relative(name: &str, caller_dir: Option<&Path>) -> Result<LoadedModule, String> {
    let base = caller_dir
        .ok_or_else(|| format!("relative require '{name}' outside of a module (no base path)"))?;
    let joined = base.join(Path::new(name));
    let mut tried = vec![];
    let mut candidates = vec![];
    for ext in [SOURCE_SUFFIX, COMPILED_SUFFIX] {
        candidates.push(format!("{}.{}", joined.display(), ext));
    }
    for ext in [SOURCE_SUFFIX, COMPILED_SUFFIX] {
        candidates.push(format!("{}/init.{}", joined.display(), ext));
    }
    for candidate in candidates {
        let path = PathBuf::from(&candidate);
        if path.is_file() {
            let proto = load_proto(&path, DukaConfig::default())
                .map_err(|e| format!("module '{name}' load error: {e}"))?;
            return Ok(LoadedModule {
                proto,
                path: Some(path),
            });
        }
        tried.push(candidate);
    }
    if joined.is_file() {
        let proto = load_proto(&joined, DukaConfig::default())
            .map_err(|e| format!("module '{name}' load error: {e}"))?;
        return Ok(LoadedModule {
            proto,
            path: Some(joined),
        });
    }
    tried.push(joined.display().to_string());
    Err(format!(
        "module '{name}' not found (tried: {})",
        tried.join(", ")
    ))
}

fn resolve_package(templates: &[String], name: &str) -> Result<LoadedModule, String> {
    let n = normalize_name(name);
    let mut tried = Vec::with_capacity(templates.len());
    for template in templates {
        let candidate = template.replace('?', &n);
        let path = PathBuf::from(&candidate);
        if path.is_file() {
            let proto = load_proto(&path, DukaConfig::default())
                .map_err(|e| format!("module '{name}' load error: {e}"))?;
            return Ok(LoadedModule {
                proto,
                path: Some(path),
            });
        }
        tried.push(candidate);
    }
    Err(format!(
        "module '{name}' not found (tried: {})",
        tried.join(", ")
    ))
}

/// Build an in-memory module loader backed by a table of pre-compiled modules
/// keyed by their slash-separated project-relative path (e.g. `src/main.duka`).
///
/// Resolves package names (`a.b`) against the `modules/` prefix and relative
/// patterns (`./x`, `../y`) against the caller module's directory, mirroring
/// `file_loader` without touching the filesystem.
pub fn memory_loader(
    modules: Arc<HashMap<String, Vec<u8>>>,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    move |name, caller_dir| {
        let base = if require::is_relative_name(name) {
            let dir = caller_dir.ok_or_else(|| {
                format!("relative require '{name}' outside of a module (no base path)")
            })?;
            normalize(&dir.join(Path::new(name)))
        } else {
            PathBuf::from("modules").join(name.replace('.', "/"))
        };
        let mut tried = Vec::new();
        for candidate in module_candidates(&base) {
            if let Some(bytes) = modules.get(&candidate) {
                let proto = DukaBinary::load(&mut Cursor::new(bytes.as_slice()))
                    .map_err(|e| format!("module '{name}' binary error: {e}"))?
                    .into_proto();
                return Ok(LoadedModule {
                    proto,
                    path: Some(PathBuf::from(&candidate)),
                });
            }
            tried.push(candidate);
        }
        Err(format!(
            "module '{name}' not found (tried: {})",
            tried.join(", ")
        ))
    }
}

fn normalize(path: &Path) -> PathBuf {
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

fn module_candidates(base: &Path) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn proto_bytes(source: &str) -> Vec<u8> {
        let proto = from_source(source, None, DukaConfig::default()).unwrap();
        proto_to_bytes(&proto).unwrap()
    }

    #[test]
    fn memory_loader_package_name() {
        let mut modules = HashMap::new();
        modules.insert("modules/a.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("a", None).unwrap();
        assert_eq!(loaded.path, Some(PathBuf::from("modules/a.duka")));
    }

    #[test]
    fn memory_loader_dotted_package() {
        let mut modules = HashMap::new();
        modules.insert("modules/a/b.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("a.b", None).unwrap();
        assert_eq!(loaded.path, Some(PathBuf::from("modules/a/b.duka")));
    }

    #[test]
    fn memory_loader_relative() {
        let mut modules = HashMap::new();
        modules.insert("src/util.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("./util", Some(Path::new("src"))).unwrap();
        assert_eq!(loaded.path, Some(PathBuf::from("src/util.duka")));
    }

    #[test]
    fn memory_loader_relative_parent() {
        let mut modules = HashMap::new();
        modules.insert("src/common.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("../../common", Some(Path::new("src/net/http"))).unwrap();
        assert_eq!(loaded.path, Some(PathBuf::from("src/common.duka")));
    }

    #[test]
    fn memory_loader_init_dir() {
        let mut modules = HashMap::new();
        modules.insert("modules/sub/init.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("sub", None).unwrap();
        assert_eq!(loaded.path, Some(PathBuf::from("modules/sub/init.duka")));
    }

    #[test]
    fn memory_loader_missing() {
        let loader = memory_loader(Arc::new(HashMap::new()));
        assert!(loader("missing", None).is_err());
        assert!(loader("./x", None).is_err());
    }
}
