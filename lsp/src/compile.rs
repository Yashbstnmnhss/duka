//! Library-level compile entry used by the language server.

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use duka_frontend::{
    analyzer::{
        build_module_types_cached,
        modules::{DukaSourceProvider, ModuleBuildCache},
        prelude::inject_type_prelude,
        BasicAnalyzer, ScopeAnalysis, ScopeAnalyzer, TypeChecker, TypeEval,
    },
    lexer::{token::Token, LexerWithMacro},
    parser::Parser,
};
use duka_shared::{
    config::DukaLexerConfig,
    constants::{COMPILED_SUFFIX, SOURCE_SUFFIX},
    errors::{DukaSpannedError, Span},
    types::{DukaAnalyzer, DukaLexer, DukaParser, TokenStream},
};

use crate::roles;

pub struct DocAnalysis {
    pub tokens: TokenStream<Token>,
    pub errors: Vec<DukaSpannedError>,
    /// 作用域表
    pub scope: ScopeAnalysis,
    pub roles: HashMap<Span, roles::Role>,
}

struct LspFileProvider {
    entry_dir: Option<PathBuf>,
    templates: Vec<String>,
}

impl LspFileProvider {
    fn for_entry(entry_path: Option<&str>) -> Self {
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
        }
    }
}

impl DukaSourceProvider for LspFileProvider {
    fn load(&self, name: &str, caller_path: Option<&str>) -> Option<(Box<str>, Arc<[u8]>)> {
        let caller_dir = caller_path
            .and_then(|p| Path::new(p).parent().map(|d| d.to_path_buf()))
            .or_else(|| self.entry_dir.clone());
        let candidates: Vec<String> = if duka_shared::module::is_relative_name(name) {
            let dir = caller_dir?;
            duka_shared::module::relative_candidates(name, &dir)
        } else {
            duka_shared::module::package_candidates(&self.templates, name)
        };
        for candidate in candidates {
            let path = PathBuf::from(&candidate);
            if path.is_file() {
                let bytes = std::fs::read(&path).ok()?;
                let key: Box<str> = candidate.replace('\\', "/").into();
                return Some((key, bytes.into()));
            }
        }
        None
    }
}

pub fn analyze(text: &str, name: &str) -> DocAnalysis {
    static BUILD_CACHES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, ModuleBuildCache>>> =
        std::sync::OnceLock::new();
    let caches = BUILD_CACHES.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut caches_guard = caches.lock().unwrap();
    let mut build_cache = caches_guard.entry(name.to_owned()).or_default();
    let mut errors = vec![];
    let lexer_cfg = DukaLexerConfig { keep_comment: true };
    let lexer = LexerWithMacro::new(Cursor::new(text), Some(name.to_owned()), lexer_cfg.clone());
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

    let provider = LspFileProvider::for_entry(chunk.source_info.name.as_deref());
    let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
    let (data, errs1) = pipeline.analyze(&chunk, Default::default());
    let build = build_module_types_cached(
        &chunk,
        data,
        Default::default(),
        lexer_cfg,
        Default::default(),
        &provider,
        &mut build_cache,
    );
    let mut data = build.data;
    data.1.modules = build.modules;
    let mut all_errors: Vec<_> = errors
        .into_iter()
        .chain(errs1)
        .chain(build.errors)
        .collect();
    all_errors.extend(inject_type_prelude(&mut data.1));
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
