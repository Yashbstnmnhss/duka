//! Library-level compile entry used by the language server.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use duka_lib::duka_frontend::{
    analyzer::{
        Adapter, BasicAnalyzer, DukaTypeChecker, ScopeAnalysis, ScopeAnalyzer, TypeChecker,
        build_module_types_cached,
        modules::{DukaSource, DukaSourceProvider, ModuleBuildCache, ModuleMap, ModuleType},
    },
    lexer::{LexerWithMacro, token::Token},
    parser::{Parser, ast::DukaChunk},
};
use duka_lib::duka_shared::{
    config::DukaLexerConfig,
    constants::{COMPILED_SUFFIX, SOURCE_SUFFIX},
    errors::{DukaSpannedError, Span},
    types::{DukaAdapter, DukaAnalyzer, DukaLexer, SourceName, TokenStream},
};

use crate::roles;

pub struct DocAnalysis {
    pub tokens: TokenStream<Token>,
    pub errors: Vec<DukaSpannedError>,
    /// 浣滅敤鍩熻〃
    pub scope: ScopeAnalysis,
    pub roles: HashMap<Span, roles::Role>,
    /// How the modules in `scope.modules` were reached, kept because
    /// `require "m"` names a module while that map is keyed by the path the name
    /// resolved to. Without it there is no way back from one to the other, so a
    /// request that needs a module rather than a type -- the attributes on what a
    /// module exports, say -- cannot find which one it meant.
    pub modules: Option<ModuleResolver>,
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

impl DocAnalysis {
    /// Source names this analysis was built from, the entry plus every module
    /// it pulled in
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
    open: HashMap<PathBuf, String>,
}

impl LspFileProvider {
    fn for_entry(entry_path: Option<&Path>, open: HashMap<PathBuf, String>) -> Self {
        let entry_dir = entry_path.map(PathBuf::from).and_then(|p| {
            p.parent()
                .map(|d| d.to_path_buf())
                .filter(|d| !d.as_os_str().is_empty())
        });
        let base = entry_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        let mut templates = vec![
            format!("{}/?.{SOURCE_SUFFIX}", base.join("modules").display()),
            format!("{}/?.{COMPILED_SUFFIX}", base.join("modules").display()),
            format!("{}/?/init.{SOURCE_SUFFIX}", base.join("modules").display()),
            format!(
                "{}/?/init.{COMPILED_SUFFIX}",
                base.join("modules").display()
            ),
        ];
        if let Ok(env) = std::env::var("DUKA_PATH") {
            templates.extend(env.split(';').map(|s| s.to_owned()));
        }
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

pub fn analyze(
    text: &str,
    name: &str,
    file_path: Option<&Path>,
    open: &HashMap<PathBuf, String>,
    build_cache: &mut ModuleBuildCache,
) -> DocAnalysis {
    #[cfg(test)]
    ANALYSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut errors = vec![];
    let lexer_cfg = DukaLexerConfig { keep_comment: true };
    let source_name = match file_path {
        Some(p) => SourceName::File(name.into(), p.into()),
        None => SourceName::Virtual(name.into()),
    };
    let lexer = LexerWithMacro::new(Cursor::new(text), source_name, lexer_cfg.clone());
    let tokens = match lexer.tokenize() {
        Ok(stream) => stream,
        Err(err) => {
            errors.push(err);
            return DocAnalysis {
                tokens: TokenStream::new(Box::new([]), Default::default()),
                errors,
                scope: ScopeAnalysis::default(),
                roles: HashMap::new(),
                modules: None,
            };
        }
    };

    let (chunk, parse_errors) = Parser::parse_lenient(tokens.clone(), Default::default());
    errors.extend(parse_errors);

    let provider = Arc::new(LspFileProvider::for_entry(
        chunk.source_info.name.path(),
        open.clone(),
    ));
    let resolver = ModuleResolver {
        provider: provider.clone(),
        caller_path: chunk.source_info.name.path().map(Path::to_path_buf),
    };
    let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
    let (data, errs1) = pipeline.analyze(&chunk, Default::default());
    let build = build_module_types_cached(
        &chunk,
        data,
        Default::default(),
        lexer_cfg,
        Default::default(),
        provider.as_ref(),
        build_cache,
    );
    let mut data = build.data;
    data.1.modules = build.modules;
    let mut all_errors: Vec<_> = errors
        .into_iter()
        .chain(errs1)
        .chain(build.errors)
        .collect();
    all_errors.extend(duka_lib::prelude::inject(&mut data.1));
    // the provider is what lets a `RequireType` be resolved here rather than
    // only by the checker afterwards: the editor must see the same errors and
    // the same types the compiler does, and a chained `RequireType("m").A.B`
    // evaluated without one is an error rather than a type
    let (data, errs) = TypeChecker.check(&chunk, data, provider.as_ref());
    all_errors.extend(errs);
    let mut scope = data.1;
    // The pass above ran on the file as written, which is where an `object` is a
    // declaration and a method has an owner. The expanded tree has neither -- the
    // desugarer flattens an object into a table and a few functions -- so it is
    // handed the objects the first pass found before the types are worked out.
    // That is the only thing the two shapes share: everything else the first pass
    // said is about names, and the second says it again.
    //
    // Only worth doing when the first pass left a name without a type, because
    // this is a whole second analysis and its types are only read for those.
    if scope.has_untyped() {
        let written_objects = std::mem::take(&mut scope.objects);
        let mut expanded = chunk.clone();
        Adapter.adapt(&mut expanded);
        let expanded_scope =
            expanded_analysis(&expanded, build_cache, provider.as_ref(), &written_objects);
        scope.objects = written_objects;
        scope.fill_types_from(expanded_scope);
    }
    DocAnalysis {
        tokens,
        errors: all_errors,
        scope,
        roles: roles::collect(&chunk),
        modules: Some(resolver),
    }
}

/// The types of a file after it has been expanded, and the errors that pass found.
///
/// Nothing here is reported: the pass the editor reads from has already said
/// whatever is wrong with the file, and saying it twice would show the reader the
/// same message at two places.
///
/// `objects` are the ones the written shape declared. The expanded shape has no
/// `object` statement left in it, so without these the checker would type a class
/// reference as a plain table and a method call on one would find nothing.
fn expanded_analysis(
    expanded: &DukaChunk,
    build_cache: &mut ModuleBuildCache,
    provider: &dyn duka_lib::duka_frontend::analyzer::modules::DukaSourceProvider,
    objects: &[duka_lib::duka_frontend::analyzer::objects::ObjectType],
) -> ScopeAnalysis {
    let (data, _) = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .analyze(expanded, Default::default());
    let build = build_module_types_cached(
        expanded,
        data,
        Default::default(),
        Default::default(),
        Default::default(),
        provider,
        build_cache,
    );
    let mut data = build.data;
    data.1.modules = build.modules;
    data.1.objects = objects.to_vec();
    let _ = duka_lib::prelude::inject(&mut data.1);
    TypeChecker.check(expanded, data, provider).0.1
}
