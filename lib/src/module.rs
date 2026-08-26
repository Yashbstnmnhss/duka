//! Default module loader helpers for `require()`.

use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use duka_backend::builtin::require::LoadedModule;
use duka_backend::codegen::DefaultGenerator;
use duka_backend::codegen::binary::{DukaBinary, Dump, Load};
use duka_backend::value::DukaProto;
use duka_frontend::analyzer::{
    Adapter, BasicAnalyzer, ScopeAnalyzer, TypeChecker, TypeEval, build_module_types,
    modules::DukaSourceProvider, prelude::inject_type_prelude,
};
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
    let mut cursor = Cursor::new(vec![]);
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
    let lexer = LexerWithMacro::new(Cursor::new(source), name, config.lexer.clone());
    let stream = lexer.tokenize()?;
    let chunk = Parser::parse(stream, config.parser.clone())?;

    let provider = FileModuleSourceProvider::for_entry(chunk.source_info.name.as_deref());
    let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
    let (data, errs1) = pipeline.analyze(&chunk, config.analyzer.clone());
    let build = build_module_types(
        &chunk,
        data,
        config.analyzer.clone(),
        config.lexer.clone(),
        config.parser.clone(),
        &provider,
    );
    let mut data = build.data;
    data.1.modules = build.modules;
    let mut errors: Vec<_> = errs1.chain(build.errors).collect();
    errors.extend(inject_type_prelude(&mut data.1));
    let (data, errs) = TypeEval.analyze(&chunk, data);
    errors.extend(errs);
    let (_data, errs) = TypeChecker.analyze_with_modules(&chunk, data, Some(&provider));
    errors.extend(errs);
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

pub struct FileModuleSourceProvider {
    entry_dir: Option<std::path::PathBuf>,
    templates: Vec<String>,
}

impl FileModuleSourceProvider {
    pub fn for_entry(entry_path: Option<&str>) -> Self {
        let entry_dir = entry_path.map(std::path::PathBuf::from).and_then(|p| {
            p.parent()
                .map(|d| d.to_path_buf())
                .filter(|d| !d.as_os_str().is_empty())
        });
        let base = entry_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        Self {
            entry_dir,
            templates: search_paths(&base),
        }
    }
}

impl DukaSourceProvider for FileModuleSourceProvider {
    fn load(
        &self,
        name: &str,
        caller_path: Option<&str>,
    ) -> Option<(Box<str>, std::sync::Arc<[u8]>)> {
        let caller_dir = caller_path
            .and_then(|p| std::path::Path::new(p).parent().map(|d| d.to_path_buf()))
            .or_else(|| self.entry_dir.clone());
        let candidates: Vec<String> = if duka_shared::module::is_relative_name(name) {
            let dir = caller_dir?;
            duka_shared::module::relative_candidates(name, &dir)
        } else {
            duka_shared::module::package_candidates(&self.templates, name)
        };
        for candidate in candidates {
            let path = std::path::PathBuf::from(&candidate);
            if path.is_file() {
                let bytes = std::fs::read(&path).ok()?;
                let key: Box<str> = candidate.replace('\\', "/").into();
                return Some((key, bytes.into()));
            }
        }
        None
    }
}

/// Files that return its content instead of path
const RESOURCE_EXTS: &[&str] = &[
    "html", "htm", "css", "txt", "md", "svg", "xml", "csv", "toml",
];

pub fn is_resource(path: &Path) -> bool {
    path.extension()
        .map(|e| RESOURCE_EXTS.contains(&e.to_str().unwrap_or("")))
        .unwrap_or(false)
}

/// Load a non-code resource file as a string value (content)
fn load_resource(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let bytes = std::fs::read(path)?;
    Ok(bytes)
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
        if duka_shared::module::is_relative_name(name) {
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
    for candidate in duka_shared::module::relative_candidates(name, base) {
        let path = PathBuf::from(&candidate);
        if path.is_file() {
            if is_resource(&path) {
                let val = load_resource(&path)
                    .map_err(|e| format!("resource '{name}' load error: {e}"))?;
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .into();
                return Ok(LoadedModule::Resource { bytes: val, ext });
            }
            let proto = load_proto(&path, DukaConfig::default())
                .map_err(|e| format!("module '{name}' load error: {e}"))?;
            return Ok(LoadedModule::Executable {
                proto,
                path: Some(path),
            });
        }
        tried.push(candidate);
    }
    tried.push(joined.display().to_string());
    Err(format!(
        "module '{name}' not found (tried: {})",
        tried.join(", ")
    ))
}

fn resolve_package(templates: &[String], name: &str) -> Result<LoadedModule, String> {
    let mut tried = Vec::with_capacity(templates.len());
    for candidate in duka_shared::module::package_candidates(templates, name) {
        let path = PathBuf::from(&candidate);
        if path.is_file() {
            if is_resource(&path) {
                let bytes = load_resource(&path)
                    .map_err(|e| format!("resource '{name}' load error: {e}"))?;
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .into();
                return Ok(LoadedModule::Resource { bytes, ext });
            }
            let proto = load_proto(&path, DukaConfig::default())
                .map_err(|e| format!("module '{name}' load error: {e}"))?;
            return Ok(LoadedModule::Executable {
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
        let base = if duka_shared::module::is_relative_name(name) {
            let dir = caller_dir.ok_or_else(|| {
                format!("relative require '{name}' outside of a module (no base path)")
            })?;
            duka_shared::module::normalize(&dir.join(Path::new(name)))
        } else {
            PathBuf::from("modules").join(name.replace('.', "/"))
        };
        let mut tried = vec![];
        for candidate in duka_shared::module::module_candidates(&base) {
            if let Some(bytes) = modules.get(&candidate) {
                let path = PathBuf::from(&candidate);
                if is_resource(&path) {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .into();
                    return Ok(LoadedModule::Resource {
                        bytes: bytes.clone(),
                        ext,
                    });
                }
                let proto = DukaBinary::load(&mut Cursor::new(bytes.as_slice()))
                    .map_err(|e| format!("module '{name}' binary error: {e}"))?
                    .into_proto();
                return Ok(LoadedModule::Executable {
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
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/a.duka"))
        );
    }

    #[test]
    fn memory_loader_dotted_package() {
        let mut modules = HashMap::new();
        modules.insert("modules/a/b.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("a.b", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/a/b.duka"))
        );
    }

    #[test]
    fn memory_loader_relative() {
        let mut modules = HashMap::new();
        modules.insert("src/util.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("./util", Some(Path::new("src"))).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("src/util.duka"))
        );
    }

    #[test]
    fn memory_loader_relative_parent() {
        let mut modules = HashMap::new();
        modules.insert("src/common.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("../../common", Some(Path::new("src/net/http"))).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("src/common.duka"))
        );
    }

    #[test]
    fn memory_loader_init_dir() {
        let mut modules = HashMap::new();
        modules.insert("modules/sub/init.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules));
        let loaded = loader("sub", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/sub/init.duka"))
        );
    }

    #[test]
    fn memory_loader_missing() {
        let loader = memory_loader(Arc::new(HashMap::new()));
        assert!(loader("missing", None).is_err());
        assert!(loader("./x", None).is_err());
    }
    fn loaded_exec_path(l: &LoadedModule) -> Option<PathBuf> {
        match l {
            LoadedModule::Executable { path, .. } => path.clone(),
            _ => None,
        }
    }
}
