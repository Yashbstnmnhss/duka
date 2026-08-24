use std::io::Cursor;

use duka_shared::errors::{DukaSpannedError, Span};
use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser};

use crate::{
    analyzer::{ScopeAnalysis, ScopeAnalyzer},
    lexer::Lexer,
    parser::Parser,
};

pub const TYPE_PRELUDE: &str = include_str!("./builtin/builtin.duka");

/// Inject prelude types into analysis data
pub fn inject_type_prelude(analysis: &mut ScopeAnalysis) -> Vec<DukaSpannedError> {
    if TYPE_PRELUDE.trim().is_empty() {
        return vec![];
    }
    let lexer = Lexer::new(
        Cursor::new(TYPE_PRELUDE),
        Some("__type_prelude__".to_owned()),
        Default::default(),
    );
    let stream = match lexer.tokenize() {
        Ok(s) => s,
        Err(e) => return vec![e],
    };
    let chunk = match Parser::parse(stream, Default::default()) {
        Ok(c) => c,
        Err(e) => return vec![e],
    };
    let (prelude_data, errs) = ScopeAnalyzer.analyze(&chunk, Default::default());
    let (_, prelude_analysis) = prelude_data;

    let offset = analysis.type_fns.len();
    let offset2 = analysis.aliases.len();

    for (i, tf) in prelude_analysis.type_fns.iter().enumerate() {
        analysis
            .symbols
            .declare_type_function(tf.name.clone(), tf.span, offset + i);
    }
    for (i, (name, _)) in prelude_analysis.aliases.iter().enumerate() {
        analysis
            .symbols
            .declare_type_alias(name.as_ref(), Span::default(), offset2 + i);
    }

    let inline_offset = analysis.inline_type_fns.len();
    for (i, f) in prelude_analysis.inline_type_fns.iter().enumerate() {
        analysis
            .symbols
            .declare_inline_type_function(f.name.clone(), f.span, inline_offset + i);
        analysis.inline_type_fns.push(f.clone());
    }
    let _ = offset;

    errs.into_iter().collect()
}
