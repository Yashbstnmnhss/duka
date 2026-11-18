//! Library-level compile entry used by the language server.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use duka_lib::duka_shared::{
    constants::{COMPILED_SUFFIX, SOURCE_SUFFIX, cpar, ctype},
    errors::{DukaDiagnose, Span},
    types::{DukaLexer, SourceName, TokenStream},
    value::ConstValue,
};
use duka_lib::{
    duka_frontend::{
        analyzer::{
            ScopeAnalysis, Visit, Visitor,
            modules::{DukaSource, DukaSourceProvider, ModuleBuildCache, ModuleMap, ModuleType},
        },
        lexer::{LexerWithMacro, token::Token},
        parser::{
            IncrementalParseCache, Parser,
            ast::{DukaChunk, Expr, Node, NodeKind, Path as AstPath},
        },
    },
    duka_shared::config::DukaParsingConfig,
};

use crate::roles;

/// Extra module search paths a client can hand over at initialization, as
/// `?.duka`-style templates. They are read where the environment's `DUKA_PATH`
/// is read, so a workspace can carry its module setup itself.
static EXTRA_MODULE_PATHS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn set_extra_module_paths(paths: Vec<String>) {
    let _ = EXTRA_MODULE_PATHS.set(paths);
}

fn extra_module_paths() -> &'static [String] {
    EXTRA_MODULE_PATHS.get().map(Vec::as_slice).unwrap_or(&[])
}

pub struct DocAnalysis {
    pub tokens: TokenStream<Token>,
    pub errors: Vec<DukaDiagnose>,
    pub scope: ScopeAnalysis,
    pub roles: HashMap<Span, roles::Role>,
    pub modules: Option<ModuleResolver>,
    pub require_bindings: HashMap<String, PathBuf>,
    pub parse_cache: Option<IncrementalParseCache>,
}

/// The way from the name written in a `require` to the module it reached. The
/// same name means a different file in a different directory, so this is the
/// provider's answer rather than a lookup.
pub struct ModuleResolver {
    provider: Arc<dyn DukaSourceProvider + Send + Sync>,
    caller_path: Option<PathBuf>,
}

impl ModuleResolver {
    pub fn resolve<'a>(&self, modules: &'a ModuleMap, name: &str) -> Option<&'a ModuleType> {
        duka_lib::duka_frontend::analyzer::modules::resolve_module_type(
            modules,
            name,
            self.caller_path.as_deref(),
            self.provider.as_ref(),
        )
    }
}

impl std::fmt::Debug for ModuleResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleResolver")
            .field("caller_path", &self.caller_path)
            .finish()
    }
}

struct RequireCollector<'a> {
    resolver: &'a ModuleResolver,
    modules: &'a ModuleMap,
    out: HashMap<String, PathBuf>,
}

fn called_name(node: &Node) -> Option<&str> {
    if let NodeKind::Access(path) = &node.0
        && let AstPath::Base((name, _)) = path.as_ref()
    {
        return Some(name.as_str());
    }
    None
}

impl RequireCollector<'_> {
    fn note(&mut self, name: &str, value: &Node) {
        let NodeKind::Call(callee, args) = &value.0 else {
            return;
        };
        let Some(callee) = called_name(callee) else {
            return;
        };
        if callee != cpar::REQUIRE && callee != ctype::REQUIRE {
            return;
        }
        let Some(NodeKind::Literal(ConstValue::String(bytes))) = args.first().map(|arg| &arg.0)
        else {
            return;
        };
        let Ok(module) = std::str::from_utf8(bytes) else {
            return;
        };
        let Some(resolved) = self.resolver.resolve(self.modules, module) else {
            return;
        };
        let Some(path) = resolved.source.name.path() else {
            return;
        };
        self.out.insert(name.to_owned(), path.to_path_buf());
    }
}

fn path_base_name(path: &AstPath) -> Option<&str> {
    match path {
        AstPath::Base((name, _)) => Some(name.as_str()),
        AstPath::Chain(rest, _) => path_base_name(rest),
        AstPath::Expr(_) => None,
    }
}

impl Visitor for RequireCollector<'_> {
    fn visit_node(&mut self, expr: &Expr) {
        match &expr.0 {
            NodeKind::Define(names, values, _, _) => {
                for (name, value) in names.iter().zip(values.iter()) {
                    self.note(&name.0.0.0, value);
                }
            }
            NodeKind::Assign(paths, values) => {
                for (path, value) in paths.iter().zip(values.iter()) {
                    if let Some(name) = path_base_name(path) {
                        self.note(name, value);
                    }
                }
            }
            _ => {}
        }
    }
}

fn require_bindings_of(
    chunk: &DukaChunk,
    resolver: &ModuleResolver,
    modules: &ModuleMap,
) -> HashMap<String, PathBuf> {
    let mut collector = RequireCollector {
        resolver,
        modules,
        out: HashMap::new(),
    };
    chunk.visit(&mut collector);
    collector.out
}

impl DocAnalysis {
    /// Source names this analysis was built from, the entry plus every module
    /// it pulled in
    #[allow(unused)]
    pub fn sources(&self) -> Vec<String> {
        let mut names: Vec<String> = self.scope.modules.keys().map(|k| k.to_string()).collect();
        names.sort();
        names.dedup();
        names
    }
}

struct LspFileProvider {
    entry_dir: Option<PathBuf>,
    templates: Vec<String>,
    /// open documents win over what is on disk
    open: HashMap<PathBuf, Arc<String>>,
}

impl LspFileProvider {
    fn for_entry(entry_path: Option<&Path>, open: HashMap<PathBuf, Arc<String>>) -> Self {
        let entry_dir = entry_path.map(PathBuf::from).and_then(|p| {
            p.parent()
                .map(|d| d.to_path_buf())
                .filter(|d| !d.as_os_str().is_empty())
        });
        let base = entry_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        // the owning project's manifest wins, else modules beside the entry
        let modules = match duka_lib::kao::find_kao(&base) {
            Ok(k) => k.root().join(k.modules_dir()),
            Err(_) => base.join("modules"),
        };
        let mut templates = vec![
            format!("{}/?.{SOURCE_SUFFIX}", modules.display()),
            format!("{}/?.{COMPILED_SUFFIX}", modules.display()),
            format!("{}/?/init.{SOURCE_SUFFIX}", modules.display()),
            format!("{}/?/init.{COMPILED_SUFFIX}", modules.display()),
        ];
        if let Ok(env) = std::env::var("DUKA_PATH") {
            templates.extend(env.split(';').map(|s| s.to_owned()));
        }
        templates.extend(extra_module_paths().iter().cloned());
        Self {
            entry_dir,
            templates,
            open,
        }
    }
}

impl DukaSourceProvider for LspFileProvider {
    fn load(&self, name: &str, caller_path: Option<&Path>) -> Option<DukaSource> {
        let caller_dir = caller_path
            .and_then(|p| Path::new(p).parent().map(|d| d.to_path_buf()))
            .or_else(|| self.entry_dir.clone());
        let candidates: Vec<String> = if duka_lib::duka_shared::module::is_relative_name(name) {
            let dir = caller_dir?;
            duka_lib::duka_shared::module::relative_candidates(name, &dir)
        } else {
            duka_lib::duka_shared::module::package_candidates(&self.templates, name)
        };
        for candidate in candidates {
            let path = PathBuf::from(&candidate);
            let key: Box<str> = candidate.replace('\\', "/").into();
            if let Some(text) = self.open.get(&path) {
                return Some(DukaSource {
                    name: key,
                    path: Some(path.into()),
                    source: text.as_bytes().to_vec().into(),
                });
            }
            if path.is_file() {
                let bytes = std::fs::read(&path).ok()?;
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

/// Every module name the require templates can reach, which is what the
/// language server offers inside `require "..."`: the `modules/` directory next
/// to the entry plus whatever `DUKA_PATH` adds.
pub fn module_candidates(entry_path: Option<&Path>) -> Vec<String> {
    let provider = LspFileProvider::for_entry(entry_path, HashMap::new());
    let mut out: Vec<String> = vec![];
    for template in &provider.templates {
        let Some(star) = template.rfind('?') else {
            continue;
        };
        let head = &template[..star];
        let tail = &template[star + 1..];
        let (dir, suffix) = match tail.rfind('/') {
            Some(slash) => (format!("{}{}", head, &tail[..slash]), &tail[slash + 1..]),
            None => (head.to_string(), tail),
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(suffix) {
                continue;
            }
            let stem = name[..name.len() - suffix.len()].to_string();
            if stem.is_empty() || stem.contains('.') {
                continue;
            }
            if !out.contains(&stem) {
                out.push(stem);
            }
        }
    }
    out.sort();
    out
}

/// How often the pipeline ran, so a test can tell a reused snapshot from a
/// repeated analysis. Analysis is the whole cost of a language server request,
/// so its absence is worth asserting on.
#[cfg(test)]
pub fn analysis_count() -> usize {
    ANALYSES.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
static ANALYSES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub fn analyze(
    text: &str,
    name: &str,
    file_path: Option<&Path>,
    open: &HashMap<PathBuf, Arc<String>>,
    build_cache: &mut ModuleBuildCache,
) -> DocAnalysis {
    analyze_incremental(
        text,
        name,
        file_path,
        open,
        build_cache,
        None,
        None,
        &DukaParsingConfig::default(),
    )
}

/// Runs the pipeline with the choices a project makes: its edition, features
/// and strictness come from `kao.toml`, while the editor always keeps comments
/// because hover, folding and semantic highlighting read them.
#[allow(clippy::too_many_arguments)]
pub fn analyze_incremental(
    text: &str,
    name: &str,
    file_path: Option<&Path>,
    open: &HashMap<PathBuf, Arc<String>>,
    build_cache: &mut ModuleBuildCache,
    previous: Option<IncrementalParseCache>,
    edit_at_char: Option<u32>,
    config: &DukaParsingConfig,
) -> DocAnalysis {
    #[cfg(test)]
    ANALYSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut errors = vec![];
    let mut config = config.clone();
    config.lexer.keep_comment = true;
    let source_name = match file_path {
        Some(p) => SourceName::File(name.into(), p.into()),
        None => SourceName::Virtual(name.into()),
    };
    let lexer = LexerWithMacro::new(Cursor::new(text), source_name, config.lexer.clone());
    //lexer_macros_prelude(&mut lexer);
    let tokens = match lexer.tokenize() {
        Ok(stream) => stream,
        Err(err) => {
            errors.push(DukaDiagnose::from(err));
            return DocAnalysis {
                tokens: TokenStream::new(Box::new([]), Default::default()),
                errors,
                scope: ScopeAnalysis::default(),
                roles: HashMap::new(),
                modules: None,
                require_bindings: HashMap::new(),
                parse_cache: None,
            };
        }
    };

    let (mut chunk, parse_errors, parse_cache) = Parser::parse_lenient_incremental(
        tokens.clone(),
        config.parser.clone(),
        previous,
        edit_at_char,
    );
    errors.extend(parse_errors.into_iter().map(DukaDiagnose::from));

    let provider = Arc::new(LspFileProvider::for_entry(
        chunk.source_info.name.path(),
        open.clone(),
    ));
    let resolver = ModuleResolver {
        provider: provider.clone(),
        caller_path: chunk.source_info.name.path().map(Path::to_path_buf),
    };
    let (data, pipeline_errors) = duka_lib::module::adapt_analyze_check(
        &mut chunk,
        &config,
        provider.as_ref() as &dyn DukaSourceProvider,
        Some(build_cache),
        None,
    );
    errors.extend(pipeline_errors);
    errors.extend(crate::lints::collect(
        &chunk,
        &data.1,
        &tokens.tokens,
        text,
        &resolver,
        &data.1.modules,
    ));
    let require_bindings = require_bindings_of(&chunk, &resolver, &data.1.modules);
    DocAnalysis {
        tokens,
        errors,
        scope: data.1,
        roles: roles::collect(&chunk),
        modules: Some(resolver),
        require_bindings,
        parse_cache: Some(parse_cache),
    }
}
