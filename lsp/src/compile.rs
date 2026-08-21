//! Library-level compile entry used by the language server.

use std::{collections::HashMap, io::Cursor, path::{Path, PathBuf}, sync::Arc};

use duka_frontend::{
    analyzer::{
        BasicAnalyzer, ScopeAnalysis, ScopeAnalyzer, TypeChecker, TypeEval, build_module_types,
        modules::DukaSourceProvider,
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
            format!("{}/?/init.{COMPILED_SUFFIX}", base.join("modules").display()),
        ];
        if let Ok(env) = std::env::var("DUKA_PATH") {
            templates.extend(env.split(';').map(|s| s.to_owned()));
        }
        Self { entry_dir, templates }
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
    let mut errors = Vec::new();
    let lexer_cfg = DukaLexerConfig { keep_comment: true };
    let lexer = LexerWithMacro::new(
        Cursor::new(text),
        Some(name.to_owned()),
        lexer_cfg.clone(),
    );
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

    match Parser::parse(tokens.clone(), Default::default()) {
        Ok(chunk) => {
            let provider = LspFileProvider::for_entry(chunk.source_info.name.as_deref());
            let pipeline = ScopeAnalyzer.chain(BasicAnalyzer);
            let (data, errs1) = pipeline.analyze(&chunk, Default::default());
            let build = build_module_types(
                &chunk,
                data,
                Default::default(),
                lexer_cfg,
                Default::default(),
                &provider,
            );
            let mut data = build.data;
            data.1.modules = build.modules;
            let mut errors: Vec<_> = errs1.chain(build.errors).collect();
            let (data, errs) = TypeEval.analyze(&chunk, data);
            errors.extend(errs);
            let (data, errs) = TypeChecker.analyze_with_modules(&chunk, data, Some(&provider));
            errors.extend(errs);
            DocAnalysis {
                tokens,
                errors,
                scope: data.1,
                roles: roles::collect(&chunk),
            }
        }
        Err(err) => {
            errors.push(err);
            DocAnalysis {
                tokens,
                errors,
                scope: ScopeAnalysis::default(),
                roles: HashMap::new(),
            }
        }
    }
}