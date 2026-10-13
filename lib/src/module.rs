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
use duka_frontend::analyzer::modules::{DukaSource, ModuleBuildCache};
use duka_frontend::analyzer::{
    Adapter, AnalyzerData, BasicAnalyzer, DukaTypeChecker, ScopeAnalyzer, TypeChecker,
    build_module_types, build_module_types_cached, modules::DukaSourceProvider,
};
use duka_frontend::expander::BangExpanderRegistry;
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::LexerWithMacro;
use duka_frontend::lexer::token::Token;
use duka_frontend::parser::Parser;
use duka_frontend::parser::ast::DukaChunk;
use duka_shared::config::{DukaConfig, DukaIRConfig, DukaParserConfig};
use duka_shared::constants::{COMPILED_SUFFIX, SOURCE_SUFFIX};
use duka_shared::errors::DukaSpannedError;
use duka_shared::types::{
    DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, SourceName, TokenStream,
};

use crate::kao::find_kao;

pub fn compile_file(
    path: &Path,
    config: DukaConfig,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let source = std::fs::read_to_string(path)?;
    from_source(
        &source,
        SourceName::File(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            path.into(),
        ),
        config,
    )
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

/// Parse, having first told the parser which keywords are bang names.
///
/// A `keyword! { .. }` is collected whole by the parser, before the expander
/// stage runs at all, so a name the expander knows but the parser does not is a
/// parse error and never reaches it.
pub fn parse_bangs(
    stream: TokenStream<Token>,
    config: DukaParserConfig,
    expander: &BangExpanderRegistry,
) -> Result<DukaChunk, DukaSpannedError> {
    expander
        .names()
        .register_with(Parser::from_tokens(stream, config))
        .parse_into_chunk()
}

/// Frontend entry point, source in and a codegen ready `DukaProto` out
/// the name decides where a relative `require` resolves from, a chunk that has
/// no file behind it should stay `SourceName::Unnamed`
///
/// ```
/// use duka_lib::module::{from_source, proto_to_bytes};
/// use duka_shared::types::SourceName;
///
/// let proto = from_source("return 7", SourceName::Unnamed, Default::default()).unwrap();
/// assert!(!proto.instructions.is_empty());
///
/// let bytes = proto_to_bytes(&proto).unwrap();
/// assert!(bytes.len() > 12); // 12 bytes of header at the very least
/// ```
/// The whole type-level pipeline, in the one order it is written in.
///
/// Adapt unwrites the sugar, the analyzer reads what is left, and the sugar it
/// unwrote is handed over so the analyzer knows what it meant -- the two cannot
/// come apart, because the tree the analyzer walks no longer carries an `object`
/// or an `export`. Module types are built next, then the prelude, then the
/// checker. Both the errors and the analysis come back, whether or not there
/// were errors: the editor reports them and still needs the types, while a
/// build stops at the first.
///
/// `chunk` is adapted in place, so a caller that wants the desugared tree for
/// its own purposes reads it after this returns.
///
/// `adapted` is what an adapter step already produced, for a caller that runs
/// the adapter as a step of its own -- the CLI's pipeline, where the adapt is
/// something to see and name. `None` adapts here, which is what a caller that
/// just wants the whole pipeline done should pass.
pub fn analyze_and_check(
    chunk: &mut DukaChunk,
    config: &DukaConfig,
    provider: &dyn DukaSourceProvider,
    build_cache: Option<&mut ModuleBuildCache>,
    adapted: Option<duka_frontend::analyzer::AdaptedData>,
) -> (AnalyzerData, Vec<DukaSpannedError>) {
    let (adapted, adapt_errors) = match adapted {
        Some(adapted) => (adapted, Vec::new()),
        None => {
            let (adapted, errs) = Adapter.adapt(chunk);
            (adapted, errs.collect())
        }
    };
    let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
    let (mut data, errs1) = pipeline.analyze(chunk, config.analyzer.clone());
    adapted.seed(&mut data.1);
    let build = match build_cache {
        Some(cache) => build_module_types_cached(
            chunk,
            data,
            config.analyzer.clone(),
            config.lexer.clone(),
            config.parser.clone(),
            provider,
            cache,
        ),
        None => build_module_types(
            chunk,
            data,
            config.analyzer.clone(),
            config.lexer.clone(),
            config.parser.clone(),
            provider,
        ),
    };
    let (mut data, build_errors) = (build.data, build.errors);
    data.1.modules = build.modules;
    let mut errors: Vec<_> = adapt_errors
        .into_iter()
        .chain(errs1)
        .chain(build_errors)
        .collect();
    errors.extend(crate::prelude::inject(&mut data.1));
    let (data, errs) = TypeChecker.check(chunk, data, Some(provider));
    errors.extend(errs);
    (data, errors)
}

/// The same pipeline, with the choices named instead of positional.
///
/// Two of them have a sensible answer for most callers and a reason to differ
/// for the rest: an editor re-checks the same files over and over and wants its
/// module cache carried across, and the CLI runs the adapter as a step of its
/// own pipeline and hands the result in rather than having it done twice.
///
/// ```no_run
/// # use duka_lib::module::{Check, FileModuleSourceProvider};
/// # use duka_shared::config::DukaConfig;
/// # use duka_frontend::analyzer::modules::ModuleBuildCache;
/// # let mut cache = ModuleBuildCache::default();
/// # let mut chunk: duka_frontend::parser::ast::DukaChunk = unimplemented!();
/// let provider = FileModuleSourceProvider::for_entry(None);
/// let (_data, errors) = Check::new(&provider)
///     .config(&DukaConfig::default())
///     .build_cache(&mut cache)      // an editor re-checking the same files
///     .run(&mut chunk);
/// ```
pub struct Check<'a> {
    config: DukaConfig,
    provider: &'a dyn DukaSourceProvider,
    build_cache: Option<&'a mut ModuleBuildCache>,
    adapted: Option<duka_frontend::analyzer::AdaptedData>,
}

impl<'a> Check<'a> {
    pub fn new(provider: &'a dyn DukaSourceProvider) -> Self {
        Self {
            config: DukaConfig::default(),
            provider,
            build_cache: None,
            adapted: None,
        }
    }

    pub fn config(mut self, config: &DukaConfig) -> Self {
        self.config = config.clone();
        self
    }

    /// Carries module types across checks, so a file that has not changed is
    /// not read and re-analyzed on every keystroke.
    pub fn build_cache(mut self, cache: &'a mut ModuleBuildCache) -> Self {
        self.build_cache = Some(cache);
        self
    }

    /// Takes an adapter step's own output instead of adapting again.
    pub fn already_adapted(mut self, adapted: duka_frontend::analyzer::AdaptedData) -> Self {
        self.adapted = Some(adapted);
        self
    }

    pub fn run(self, chunk: &mut DukaChunk) -> (AnalyzerData, Vec<DukaSpannedError>) {
        analyze_and_check(
            chunk,
            &self.config,
            self.provider,
            self.build_cache,
            self.adapted,
        )
    }
}

pub fn from_source(
    source: &str,
    name: SourceName,
    config: DukaConfig,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let expander = BangExpanderRegistry::new();
    let lexer = LexerWithMacro::new(Cursor::new(source), name, config.lexer.clone());
    let stream = lexer.tokenize()?;
    // the parser collects a `!` body before the expander stage sees it, so it
    // has to be told which keywords are bang names or it rejects them outright
    let mut chunk = parse_bangs(stream, config.parser.clone(), &expander)?;

    expander
        .expand_chunk(&mut chunk)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    let provider = FileModuleSourceProvider::for_entry(chunk.source_info.name.path());
    let (_data, errors) = analyze_and_check(&mut chunk, &config, &provider, None, None);
    if let Some(err) = errors.into_iter().next() {
        return Err(Box::new(err));
    }

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
    pub fn for_entry(entry_path: Option<&Path>) -> Self {
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
            templates: search_paths(&base, "modules"),
        }
    }
}

impl DukaSourceProvider for FileModuleSourceProvider {
    fn load(&self, name: &str, caller_path: Option<&Path>) -> Option<DukaSource> {
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
                return Some(DukaSource {
                    name: key,
                    path: Some(path.into()),
                    source: bytes.into(),
                });
            }
        }
        None
    }
}

/// Files that return its content instead of path
const RESOURCE_EXTS: &[&str] = &[
    "html", "htm", "css", "txt", "md", "svg", "xml", "csv", "toml", "json",
];

pub fn is_resource(path: &Path) -> bool {
    path.extension()
        .map(|e| RESOURCE_EXTS.contains(&e.to_str().unwrap_or("")))
        .unwrap_or(false)
}

fn resolve_pkg_entry(pkg_path: &str, modules: &HashMap<String, Vec<u8>>) -> Option<PathBuf> {
    let kao_key = format!("{}/kao.toml", pkg_path);
    let kao_bytes = modules.get(&kao_key)?;
    let kao_str = std::str::from_utf8(kao_bytes).ok()?;
    let manifest = toml::from_str::<crate::kao::KaoManifest>(kao_str).ok()?;
    let entry = manifest.build.entry.as_deref().unwrap_or("src/init.duka");
    Some(PathBuf::from(format!("{}/{}", pkg_path, entry)))
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
pub fn search_paths(base_dir: &Path, modules_dir: &str) -> Vec<String> {
    let mut res = vec![];
    let modules = base_dir.join(modules_dir);
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
    config: DukaConfig,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    let templates: Vec<String> = templates.into_iter().collect();
    move |name, caller_dir| {
        if duka_shared::module::is_relative_name(name) {
            resolve_relative(name, caller_dir, &config)
        } else {
            resolve_package(&templates, name, &config)
        }
    }
}

fn resolve_relative(
    name: &str,
    caller_dir: Option<&Path>,
    config: &DukaConfig,
) -> Result<LoadedModule, String> {
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
            let proto = load_proto(&path, config.clone())
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

fn resolve_package(
    templates: &[String],
    name: &str,
    config: &DukaConfig,
) -> Result<LoadedModule, String> {
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
            let proto = load_proto(&path, config.clone())
                .map_err(|e| format!("module '{name}' load error: {e}"))?;
            return Ok(LoadedModule::Executable {
                proto,
                path: Some(path),
            });
        }
        tried.push(candidate);
    }
    for template in templates {
        if let Some(dir) = template
            .strip_suffix("/init.duka")
            .or_else(|| template.strip_suffix("/init.dukac"))
        {
            let root_dir = PathBuf::from(dir.replace('?', package_root_name(name)));
            if root_dir.is_dir() {
                if let Ok(kao) = find_kao(&root_dir) {
                    if let Some(rel) = package_relative(name) {
                        let src_dir = kao.src_dir();
                        let rel_path = root_dir.join(&src_dir).join(rel);
                        for candidate in duka_shared::module::module_candidates(&rel_path) {
                            let path = PathBuf::from(&candidate);
                            if path.is_file() {
                                let proto = load_proto(&path, config.clone())
                                    .map_err(|e| format!("module '{name}' load error: {e}"))?;
                                return Ok(LoadedModule::Executable {
                                    proto,
                                    path: Some(path),
                                });
                            }
                            tried.push(candidate);
                        }
                    } else {
                        let entry = kao.entry();
                        let entry_path = root_dir.join(&entry);
                        if entry_path.is_file() {
                            let proto = load_proto(&entry_path, config.clone())
                                .map_err(|e| format!("package '{name}' load error: {e}"))?;
                            return Ok(LoadedModule::Executable {
                                proto,
                                path: Some(entry_path),
                            });
                        }
                        tried.push(entry_path.display().to_string());
                    }
                }
                tried.push(root_dir.display().to_string());
            }
        }
    }
    Err(format!(
        "module '{name}' not found (tried: {})",
        tried.join(", ")
    ))
}

fn package_root_name(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

fn package_relative(name: &str) -> Option<String> {
    let mut parts = name.splitn(2, '.');
    parts.next()?;
    parts.next().map(|rest| rest.replace('.', "/"))
}

/// Build an in-memory module loader backed by a table of pre-compiled modules
/// keyed by their slash-separated project-relative path (e.g. `src/main.duka`).
///
/// Resolves package names (`a.b`) against the `modules/` prefix and relative
/// patterns (`./x`, `../y`) against the caller module's directory, mirroring
/// `file_loader` without touching the filesystem.
///
/// Falls back to kao-based resolution: if flat `module_candidates` lookup fails
/// for a package name, looks for `kao.toml` in the package root directory
/// (within the HashMap) and uses its `entry` and `src_dir` fields.
pub fn memory_loader(
    modules: Arc<HashMap<String, Vec<u8>>>,
    modules_dir: &str,
) -> impl Fn(&str, Option<&Path>) -> Result<LoadedModule, String> + Send + Sync + 'static {
    let modules_dir = modules_dir.to_owned();
    move |name, caller_dir| {
        let base = if duka_shared::module::is_relative_name(name) {
            let dir = caller_dir.ok_or_else(|| {
                format!("relative require '{name}' outside of a module (no base path)")
            })?;
            duka_shared::module::normalize(&dir.join(Path::new(name)))
        } else {
            PathBuf::from(&modules_dir).join(name.replace('.', "/"))
        };
        let mut tried = vec![];

        // Flat lookup via module_candidates
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
                // If this is a bare package path (no extension), resolve the
                // actual entry path from kao.toml so relative requires inside
                // the entry module resolve correctly.
                let mut resolved_path = path;
                if resolved_path.extension().is_none()
                    && let Some(p) = resolve_pkg_entry(&candidate, &modules)
                {
                    resolved_path = p;
                }
                return Ok(LoadedModule::Executable {
                    proto,
                    path: Some(resolved_path),
                });
            }
            tried.push(candidate);
        }

        // Kao-based fallback for packages (non-relative names only)
        if !duka_shared::module::is_relative_name(name) {
            let pkg_root = format!("{}/{}", modules_dir, package_root_name(name));
            let kao_key = format!("{}/kao.toml", pkg_root);
            if let Some(kao_bytes) = modules.get(&kao_key) {
                if let Ok(kao_str) = std::str::from_utf8(kao_bytes)
                    && let Ok(manifest) = toml::from_str::<crate::kao::KaoManifest>(kao_str)
                {
                    if let Some(rel) = package_relative(name) {
                        // Sub-module: require("duka-ui.vnode")
                        let src_dir = manifest.build.src_dir.as_deref().unwrap_or("src");
                        let rel_path = format!("{}/{}/{}", pkg_root, src_dir, rel);
                        for candidate in
                            duka_shared::module::module_candidates(&PathBuf::from(&rel_path))
                        {
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
                    } else {
                        // Top-level: require("duka-ui")
                        let entry = manifest.build.entry.as_deref().unwrap_or("src/main.duka");
                        let entry_path = format!("{}/{}", pkg_root, entry);
                        if let Some(bytes) = modules.get(&entry_path) {
                            let path = PathBuf::from(&entry_path);
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
                                .map_err(|e| format!("package '{name}' binary error: {e}"))?
                                .into_proto();
                            return Ok(LoadedModule::Executable {
                                proto,
                                path: Some(path),
                            });
                        }
                        tried.push(entry_path);
                    }
                }
                tried.push(kao_key);
            }
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
        let proto = from_source(source, SourceName::Unnamed, DukaConfig::default()).unwrap();
        proto_to_bytes(&proto).unwrap()
    }

    #[test]
    fn memory_loader_package_name() {
        let mut modules = HashMap::new();
        modules.insert("modules/a.duka".to_owned(), proto_bytes("return 1"));
        let loader = memory_loader(Arc::new(modules), "modules");
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
        let loader = memory_loader(Arc::new(modules), "modules");
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
        let loader = memory_loader(Arc::new(modules), "./");
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
        let loader = memory_loader(Arc::new(modules), "modules");
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
        let loader = memory_loader(Arc::new(modules), "modules");
        let loaded = loader("sub", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/sub/init.duka"))
        );
    }

    #[test]
    fn memory_loader_missing() {
        let loader = memory_loader(Arc::new(HashMap::new()), "./");
        assert!(loader("missing", None).is_err());
        assert!(loader("./x", None).is_err());
    }

    #[test]
    fn memory_loader_kao_package() {
        let mut modules = HashMap::new();
        // kao.toml with custom entry
        modules.insert(
            "modules/my-pkg/kao.toml".to_owned(),
            br#"[kao]
name = "my-pkg"
[build]
entry = "src/init.duka"
"#
            .to_vec(),
        );
        // Compiled entry at the custom path
        modules.insert(
            "modules/my-pkg/src/init.duka".to_owned(),
            proto_bytes("return 42"),
        );
        let loader = memory_loader(Arc::new(modules), "modules");
        let loaded = loader("my-pkg", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/my-pkg/src/init.duka"))
        );
    }

    #[test]
    fn memory_loader_kao_submodule() {
        let mut modules = HashMap::new();
        // kao.toml
        modules.insert(
            "modules/my-pkg/kao.toml".to_owned(),
            br#"[kao]
name = "my-pkg"
[build]
entry = "src/init.duka"
"#
            .to_vec(),
        );
        // Sub-module at src/vnode.duka
        modules.insert(
            "modules/my-pkg/src/vnode.duka".to_owned(),
            proto_bytes("return 'vnode'"),
        );
        let loader = memory_loader(Arc::new(modules), "modules");
        let loaded = loader("my-pkg.vnode", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/my-pkg/src/vnode.duka"))
        );
    }

    #[test]
    fn memory_loader_kao_default_entry() {
        let mut modules = HashMap::new();
        // kao.toml without entry field (defaults to src/main.duka)
        modules.insert(
            "modules/pkg2/kao.toml".to_owned(),
            br#"[kao]
name = "pkg2"
"#
            .to_vec(),
        );
        // Default entry path
        modules.insert(
            "modules/pkg2/src/main.duka".to_owned(),
            proto_bytes("return 'default'"),
        );
        let loader = memory_loader(Arc::new(modules), "modules");
        let loaded = loader("pkg2", None).unwrap();
        assert_eq!(
            loaded_exec_path(&loaded),
            Some(PathBuf::from("modules/pkg2/src/main.duka"))
        );
    }

    fn loaded_exec_path(l: &LoadedModule) -> Option<PathBuf> {
        match l {
            LoadedModule::Executable { path, .. } => path.clone(),
            _ => None,
        }
    }
}
