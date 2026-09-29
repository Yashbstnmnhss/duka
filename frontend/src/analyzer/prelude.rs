use std::io::Cursor;

use duka_shared::errors::{DukaSpannedError, Span};
use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser, SourceName};

use crate::{
    analyzer::{ScopeAnalysis, ScopeAnalyzer},
    lexer::Lexer,
    parser::Parser,
};

pub const TYPE_PRELUDE: &str = include_str!("./builtin/builtin.duka");

use duka_shared::docs::{MetaInfo, MetaItemInfo, MetaScope};
use duka_shared::dtype::Type;
use duka_shared::utils::SymbolType;
use duka_shared::value::ConstValue;

/// Declares what the runtime provides, so a name the program cannot declare
/// still resolves: `print` is a function, `os` is a table of them.
///
/// Which builtins exist, and whether a module's members are global or sit
/// behind its name, is whatever the runtime this build links against handed
/// over. Nothing here names a builtin, so a runtime that adds one needs no
/// change here, and one compiled without `os` simply does not have it.
pub fn inject_builtins(analysis: &mut ScopeAnalysis, registrations: &[(MetaScope, MetaInfo)]) {
    for (scope, meta) in registrations {
        match scope {
            MetaScope::Global => {
                if let MetaItemInfo::Module { inner } = &meta.info {
                    for member in inner.iter() {
                        declare_value(analysis, member);
                    }
                }
            }
            MetaScope::Module => declare_module(analysis, meta),
        }
    }
}

/// The type context builtins. They are callable in a type position but they are
/// not types, so they get their own kind instead of resolving as one.
pub fn inject_type_builtins(analysis: &mut ScopeAnalysis, meta: &MetaInfo) {
    let MetaItemInfo::Module { inner } = &meta.info else {
        return;
    };
    for builtin in inner.iter() {
        if matches!(builtin.info, MetaItemInfo::TypeFunction { .. })
            && analysis.symbols.lookup(builtin.name).is_none()
        {
            analysis
                .symbols
                .declare_builtin(builtin.name, SymbolType::TypeBuiltin, Type::Any);
        }
    }
}

/// A module is reached through its name and holds its members, so reading one is
/// a field read, which is what `Type::TypeTable` already means.
fn declare_module(analysis: &mut ScopeAnalysis, meta: &MetaInfo) {
    let fields = match &meta.info {
        MetaItemInfo::Module { inner } => inner
            .iter()
            .map(|m| member_field(m))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    };
    let Some(fields) = fields else { return };
    declare(
        analysis,
        meta.name,
        SymbolType::Variable,
        Type::TypeTable(fields),
    );
}

/// Everything a module can hold as a value, or `None` for something that is
/// only meaningful inside a type position.
fn member_field(meta: &MetaInfo) -> Option<(ConstValue, Box<Type>)> {
    let ty = match &meta.info {
        MetaItemInfo::Function { .. } | MetaItemInfo::Constant { .. } => meta.get_type(),
        MetaItemInfo::UserData { methods, .. } => Type::TypeTable(
            methods
                .iter()
                .map(|m| member_field(m))
                .collect::<Option<Vec<_>>>()?,
        ),
        MetaItemInfo::Static { inner } => return member_field(inner),
        MetaItemInfo::Module { .. } | MetaItemInfo::TypeFunction { .. } => return None,
    };
    Some((
        ConstValue::String(meta.name.as_bytes().to_vec().into_boxed_slice()),
        Box::new(ty),
    ))
}

fn declare_value(analysis: &mut ScopeAnalysis, meta: &MetaInfo) {
    let (ty, kind) = match &meta.info {
        MetaItemInfo::Function { .. } | MetaItemInfo::Constant { .. } => {
            let ty = meta.get_type();
            let kind = match ty {
                // a callable reads as a function, the same as one written in the
                // file, rather than as a bare table of properties
                Type::Function(_) => SymbolType::Function,
                _ => SymbolType::Variable,
            };
            (ty, kind)
        }
        MetaItemInfo::UserData { methods, .. } => {
            let fields = methods
                .iter()
                .map(|m| member_field(m))
                .collect::<Option<Vec<_>>>();
            let Some(fields) = fields else { return };
            (Type::TypeTable(fields), SymbolType::Variable)
        }
        MetaItemInfo::Static { inner } => return declare_value(analysis, inner),
        MetaItemInfo::Module { .. } | MetaItemInfo::TypeFunction { .. } => return,
    };
    declare(analysis, meta.name, kind, ty);
}

fn declare(analysis: &mut ScopeAnalysis, name: &str, kind: SymbolType, ty: Type) {
    // a name the file declares itself wins, which is how a local shadows a
    // builtin
    if analysis.symbols.lookup(name).is_some() {
        return;
    }
    analysis.symbols.declare_builtin(name, kind, ty);
}

/// Inject prelude types into analysis data
/// Locally defined names always shadow prelude definitions
pub fn inject_type_prelude(analysis: &mut ScopeAnalysis) -> Vec<DukaSpannedError> {
    if TYPE_PRELUDE.trim().is_empty() {
        return vec![];
    }
    let lexer = Lexer::new(
        Cursor::new(TYPE_PRELUDE),
        SourceName::Virtual("__type_prelude__".into()),
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
