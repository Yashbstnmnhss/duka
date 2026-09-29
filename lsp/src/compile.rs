//! Library-level compile entry used by the language server.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
};

use duka_lib::duka_frontend::{
    analyzer::{
        BasicAnalyzer, ScopeAnalysis, ScopeAnalyzer, TypeChecker, TypeEval,
        build_module_types_cached,
        modules::{DukaSource, DukaSourceProvider, ModuleBuildCache},
    },
    lexer::{LexerWithMacro, token::Token},
    parser::Parser,
};
use duka_lib::duka_shared::{
    config::DukaLexerConfig,
    constants::{COMPILED_SUFFIX, SOURCE_SUFFIX},
    errors::{DukaSpannedError, Span},
    types::{DukaAnalyzer, DukaLexer, SourceName, TokenStream},
};

use crate::roles;

pub struct DocAnalysis {
    pub tokens: TokenStream<Token>,
    pub errors: Vec<DukaSpannedError>,
    /// 作用域表
    pub scope: ScopeAnalysis,
    pub roles: HashMap<Span, roles::Role>,
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
            };
        }
    };

    let (chunk, parse_errors) = Parser::parse_lenient(tokens.clone(), Default::default());
    errors.extend(parse_errors);

    let provider = LspFileProvider::for_entry(chunk.source_info.name.path(), open.clone());
    let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
    let (data, errs1) = pipeline.analyze(&chunk, Default::default());
    let build = build_module_types_cached(
        &chunk,
        data,
        Default::default(),
        lexer_cfg,
        Default::default(),
        &provider,
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
    let (data, errs) = TypeEval.analyze(&chunk, data);
    all_errors.extend(errs);
    let (data, errs) = TypeChecker.analyze_with_modules(&chunk, data, Some(&provider));
    all_errors.extend(errs);
    DocAnalysis {
        tokens,
        errors: all_errors,
        scope: data.1,
        roles: roles::collect(&chunk),
    }
}
