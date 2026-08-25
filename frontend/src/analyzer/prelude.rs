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
/// Locally defined names always shadow prelude definitions
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

    let mut new_fns = vec![];
    for tf in prelude_analysis.type_fns.into_iter() {
        if analysis.symbols.lookup(tf.name.as_ref()).is_none() {
            let id = analysis.type_fns.len();
            analysis
                .symbols
                .declare_type_function(tf.name.clone(), tf.span, id);
            analysis.type_fns.push(tf);
            new_fns.push(id);
        }
    }

    //let mut new_aliases: Vec<(Box<str>, crate::parser::ast::TypeDescriptor)> = vec![];
    for (name, tv) in prelude_analysis.aliases.into_iter() {
        if analysis.symbols.lookup(name.as_ref()).is_none() {
            let id = analysis.aliases.len();
            analysis
                .symbols
                .declare_type_alias(name.as_ref(), Span::default(), id);
            analysis.aliases.push((name, tv));
        }
    }

    for f in prelude_analysis.inline_type_fns.into_iter() {
        if analysis.symbols.lookup(f.name.as_ref()).is_none() {
            let id = analysis.inline_type_fns.len();
            analysis
                .symbols
                .declare_inline_type_function(f.name.clone(), f.span, id);
            analysis.inline_type_fns.push(f);
        }
    }

    errs.into_iter().collect()
}
