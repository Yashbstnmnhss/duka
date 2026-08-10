//! Library-level compile entry used by the language server.

use std::{collections::HashMap, io::Cursor};

use duka_frontend::{
    analyzer::{ScopeAnalysis, ScopeAnalyzer, TypeChecker},
    lexer::{token::Token, LexerWithMacro},
    parser::Parser,
};
use duka_shared::{
    config::DukaLexerConfig,
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

pub fn analyze(text: &str, name: &str) -> DocAnalysis {
    let mut errors = Vec::new();
    let lexer = LexerWithMacro::new(
        Cursor::new(text),
        Some(name.to_owned()),
        DukaLexerConfig { keep_comment: true },
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
            let analyzer = ScopeAnalyzer.chain(TypeChecker);
            let (analysis, semantic) = analyzer.analyze(&chunk, Default::default());
            errors.extend(semantic);
            DocAnalysis {
                tokens,
                errors,
                scope: analysis.1,
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
