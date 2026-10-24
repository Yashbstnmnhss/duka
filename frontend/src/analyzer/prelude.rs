use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;
use std::sync::Arc;

use duka_shared::attributes::Attributes;
use duka_shared::errors::{DukaSpannedError, Span};
use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser, SourceInfo, SourceName};

use crate::{
    analyzer::{
        BuiltinRules, ScopeAnalysis, ScopeAnalyzer,
        eval::{EvalCtx, EvalCtxInit},
    },
    lexer::Lexer,
    parser::Parser,
};

pub const TYPE_PRELUDE: &str = include_str!("./builtin/builtin.duka");

use duka_shared::docs::{BuiltinInfo, MetaInfo, MetaItemInfo, MetaScope};
use duka_shared::dtype::Type;
use duka_shared::utils::{SymbolTableViewer, SymbolType};
use duka_shared::value::ConstValue;
use std::collections::HashMap;
use std::sync::LazyLock;

/// The type-context builtins written in DUKA, described the way the editor
/// describes a builtin.
///
/// `TYPE_PRELUDE` is a source file like any other, so what it declares is read by
/// reading it -- the same parse that makes these work is what says what they are
/// called and how many they take. Hand-listing them here would be a second list to
/// keep in step with the file, and the step that gets forgotten is the one where a
/// builtin is written and never offered.
///
/// There is no documentation to carry, because none is written. That is the whole
/// of what is missing: a builtin completion cannot offer is a builtin that does not
/// exist as far as a reader is concerned, and these worked long before they could
/// be listed.
pub fn type_prelude_meta() -> &'static MetaInfo {
    static META: LazyLock<MetaInfo> = LazyLock::new(|| {
        // Leaked rather than kept beside the metadata: `MetaInfo::Module` holds a
        // slice, so the items have to outlive it, and this is built once for the
        // lifetime of the process anyway.
        let mut items: Vec<MetaInfo> = Vec::new();
        let Ok(stream) = Lexer::new(
            Cursor::new(TYPE_PRELUDE),
            SourceName::Virtual("__type_prelude__".into()),
            Default::default(),
        )
        .tokenize() else {
            return empty_prelude_meta();
        };
        let Ok(chunk) = Parser::parse(stream, Default::default()) else {
            return empty_prelude_meta();
        };
        let ((_config, analysis), _errors) = ScopeAnalyzer.analyze(&chunk, Default::default());

        for tf in analysis.type_fns.iter() {
            items.push(type_fn_meta(tf.name.clone().into_string(), tf.body.0.len()));
        }
        for f in analysis.inline_type_fns.iter() {
            items.push(type_fn_meta(f.name.clone().into_string(), f.params.len()));
        }

        let inner: &'static [MetaInfo] = Box::leak(items.into_boxed_slice());
        MetaInfo {
            name: "type-context",
            doc: "Builtins for type-context",
            example: None,
            info: MetaItemInfo::Module { inner },
            flags: &[("feature", &["type-context"])],
        }
    });
    &META
}

/// A stable copy of one item, because the metadata outlives the parse it was read
/// from. `MetaInfo::Module` holds a slice, so the items have to live as long as it.
fn type_fn_meta(name: String, count: usize) -> MetaInfo {
    MetaInfo {
        name: Box::leak(name.into_boxed_str()),
        doc: "",
        example: None,
        info: MetaItemInfo::TypeFunction { param_count: count },
        flags: &[],
    }
}

fn empty_prelude_meta() -> MetaInfo {
    MetaInfo {
        name: "type-context",
        doc: "Builtins for type-context",
        example: None,
        info: MetaItemInfo::Module { inner: &[] },
        flags: &[("feature", &["type-context"])],
    }
}

/// Declares what the runtime provides, so a name the program cannot declare
/// still resolves: `print` is a function, `os` is a table of them.
///
/// Which builtins exist, and whether a module's members are global or sit
/// behind its name, is whatever the runtime this build links against handed
/// over. Nothing here names a builtin, so a runtime that adds one needs no
/// change here, and one compiled without `os` simply does not have it.
pub fn inject_builtins(analysis: &mut ScopeAnalysis, registrations: &[BuiltinInfo]) {
    analysis.userdata_types = userdata_methods(registrations);
    analysis.builtin_rules = builtin_rules(registrations);
    for (scope, meta) in registrations {
        match scope {
            MetaScope::Global => {
                if let MetaItemInfo::Module { inner } = &meta.info {
                    for member in inner.iter() {
                        declare_value(analysis, member);
                    }
                }
            }
            MetaScope::Module => {
                declare_module(analysis, meta);
            }
        }
    }
    let names: Vec<Box<str>> = analysis.userdata_types.keys().cloned().collect();
    for name in names {
        declare(
            analysis,
            &name,
            SymbolType::Variable,
            Type::UserData { name: name.clone() },
            Attributes::default(),
        );
    }
}

/// What every builtin says about its own call, keyed by the name it is called
/// by.
///
/// A builtin may write a `where` list and a set of constraints, and neither is
/// a type, so neither travels in the signature. They are read from the
/// metadata and kept here, and the call that has to discharge them reads them
/// from here.
fn builtin_rules(registrations: &[BuiltinInfo]) -> HashMap<Box<str>, BuiltinRules> {
    let mut out = HashMap::new();
    for (_, meta) in registrations {
        let MetaItemInfo::Module { inner } = &meta.info else {
            continue;
        };
        for member in inner.iter() {
            let MetaItemInfo::Function {
                where_clauses,
                constraints,
                ..
            } = &member.info
            else {
                continue;
            };
            if where_clauses.is_empty() && constraints.is_empty() {
                continue;
            }
            out.insert(
                member.name.into(),
                BuiltinRules {
                    where_clauses: where_clauses.to_vec().into(),
                    constraints: constraints.to_vec().into(),
                },
            );
        }
    }
    out
}

/// The methods of every userdata the runtime offers, keyed by the name that
/// tells one userdata from another.
///
/// A userdata is a payload the runtime owns, and the methods are how a program
/// reaches it. The type says which payload it is -- a file is not a regular
/// expression -- and the methods are what a call on it may reach.
fn userdata_methods(registrations: &[BuiltinInfo]) -> HashMap<Box<str>, Box<[(Box<str>, Type)]>> {
    let mut out = HashMap::new();
    for (_, meta) in registrations {
        let MetaItemInfo::Module { inner } = &meta.info else {
            continue;
        };
        for member in inner.iter() {
            let MetaItemInfo::UserData { ty_name, methods } = &member.info else {
                continue;
            };
            let methods: Box<[(Box<str>, Type)]> = methods
                .iter()
                .filter_map(|m| {
                    let ty = match &m.info {
                        MetaItemInfo::Function { .. } | MetaItemInfo::Constant { .. } => {
                            m.get_type()
                        }
                        _ => return None,
                    };
                    Some((m.name.into(), ty))
                })
                .collect();
            out.insert((*ty_name).into(), methods);
        }
    }
    out
}

/// The type context builtins. They are callable in a type position but they are
/// not types, so they get their own kind instead of resolving as one.
pub fn inject_type_builtins(analysis: &mut ScopeAnalysis, meta: &MetaInfo) {
    let MetaItemInfo::Module { inner } = &meta.info else {
        return;
    };
    for builtin in inner.iter() {
        if matches!(builtin.info, MetaItemInfo::TypeFunction { .. })
            && analysis.symbols.lookup_anywhere(builtin.name).is_none()
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
            .map(|m| member_field(analysis, m))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    };
    let Some(fields) = fields else { return };
    declare(
        analysis,
        meta.name,
        SymbolType::Variable,
        Type::TypeTable(fields),
        meta.attributes(),
    );
}

/// Everything a module can hold as a value, or `None` for something that is
/// only meaningful inside a type position.
fn member_field(analysis: &ScopeAnalysis, meta: &MetaInfo) -> Option<(ConstValue, Box<Type>)> {
    let ty = match &meta.info {
        MetaItemInfo::Function { .. } | MetaItemInfo::Constant { .. } => type_of(analysis, meta),
        // a userdata is a payload of its own: the type says which one, and the
        // methods it has are kept beside the class table
        MetaItemInfo::UserData { ty_name, .. } => Type::UserData {
            name: (*ty_name).into(),
        },
        MetaItemInfo::Static { inner, .. } => return member_field(analysis, inner),
        MetaItemInfo::Module { .. } | MetaItemInfo::TypeFunction { .. } => return None,
    };
    Some((
        ConstValue::String(meta.name.as_bytes().to_vec().into_boxed_slice()),
        Box::new(ty),
    ))
}

/// The type of a builtin, with the names it holds read.
///
/// A builtin may name a type the language declares -- `type(CoStatus)` -- and
/// the scope that says what the name is has to be asked, because the metadata
/// itself knows only the name.
fn type_of(analysis: &ScopeAnalysis, meta: &MetaInfo) -> Type {
    meta.get_type_with(&|name, args| resolve_named_type(analysis, name, args))
}

/// What a name in a builtin's signature is.
///
/// It is read the way a type position reads it: an alias is the type it stands
/// for, and a type function is applied to the arguments it was given.
fn resolve_named_type(analysis: &ScopeAnalysis, name: &str, args: &[Type]) -> Option<Type> {
    let init = EvalCtxInit {
        source: Arc::new(SourceInfo {
            name: SourceName::Virtual("__builtins__".into()),
            source: Arc::from(Vec::new()),
        }),
        contributed: Rc::new(RefCell::new(Vec::new())),
        viewer: SymbolTableViewer::new(&analysis.symbols),
        type_fns: &analysis.type_fns,
        inline_type_fns: &analysis.inline_type_fns,
        objects: &analysis.objects,
        aliases: &analysis.aliases,
        results: analysis.call_cache.clone(),
        closures: analysis.closures.clone(),
        modules: Some(&analysis.modules),
        provider: None,
        report_errors: false,
        module: None,
    };
    let mut ev = EvalCtx::new(init);
    let span = Span::default();
    // what the name is decides how it is read: a type function is applied, and
    // anything else is the type it names
    let symbol = analysis.symbols.lookup_anywhere(name);
    let applied = matches!(
        symbol.map(|s| &s.symbol_type),
        Some(SymbolType::TypeFunction(_))
    );
    let desc = if applied {
        let callee = crate::parser::ast::Node(
            crate::parser::ast::NodeKind::Access(Box::new(crate::parser::ast::Path::Base((
                name.into(),
                span,
            )))),
            span,
        );
        let args = args
            .iter()
            .map(|a| {
                crate::parser::ast::Node(
                    crate::parser::ast::NodeKind::TypeLit(crate::parser::ast::TypeDesc::Pure(
                        a.clone(),
                    )),
                    span,
                )
            })
            .collect::<Vec<_>>();
        crate::parser::ast::TypeDesc::Expr(Box::new(crate::parser::ast::Node(
            crate::parser::ast::NodeKind::Call(Box::new(callee), args.into_boxed_slice()),
            span,
        )))
    } else {
        crate::parser::ast::TypeDesc::Named(name.into(), span)
    };
    Some(ev.eval_type(&desc).to_type())
}

fn declare_value(analysis: &mut ScopeAnalysis, meta: &MetaInfo) {
    let (ty, kind) = match &meta.info {
        MetaItemInfo::Function { .. } | MetaItemInfo::Constant { .. } => {
            let ty = type_of(analysis, meta);
            let kind = match ty {
                // a callable reads as a function, the same as one written in the
                // file, rather than as a bare table of properties
                Type::Function(_) => SymbolType::Function,
                _ => SymbolType::Variable,
            };
            (ty, kind)
        }
        MetaItemInfo::UserData { ty_name, .. } => {
            // the name of a userdata is the name of its type
            (
                Type::UserData {
                    name: (*ty_name).into(),
                },
                SymbolType::Variable,
            )
        }
        MetaItemInfo::Static { inner } => return declare_value(analysis, inner),
        MetaItemInfo::Module { .. } | MetaItemInfo::TypeFunction { .. } => return,
    };
    declare(analysis, meta.name, kind, ty, meta.attributes());
}

fn declare(
    analysis: &mut ScopeAnalysis,
    name: &str,
    symbol_type: SymbolType,
    ty: Type,
    attributes: Attributes,
) {
    // a name the file declares itself wins, which is how a local shadows a
    // builtin
    if analysis.symbols.lookup_anywhere(name).is_some() {
        return;
    }
    analysis
        .symbols
        .declare_builtin_with(name, symbol_type, ty, attributes);
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
        if analysis.symbols.lookup_anywhere(tf.name.as_ref()).is_none() {
            let id = analysis.type_fns.len();
            // `Span::default()`, like the aliases below and for the same reason. A
            // prelude declaration is written in another file, so its span is a line
            // in *that* file -- and a name lookup only accepts a declaration that
            // comes before the reference, which put every one of these out of reach
            // of `Pick` in a file that used it. A declaration that is in scope
            // everywhere has to be written as being at the start of everywhere.
            //
            // A local declaration of the same name still wins: the lookup takes the
            // latest one that is not after the cursor.
            analysis
                .symbols
                .declare_type_function(tf.name.clone(), Span::default(), id);
            analysis.type_fns.push(tf);
            new_fns.push(id);
        }
    }

    //let mut new_aliases: Vec<(Box<str>, crate::parser::ast::TypeDescriptor)> = vec![];
    for (name, tv) in prelude_analysis.aliases.into_iter() {
        if analysis.symbols.lookup_anywhere(name.as_ref()).is_none() {
            let id = analysis.aliases.len();
            analysis
                .symbols
                .declare_type_alias(name.as_ref(), Span::default(), id);
            analysis.aliases.push((name, tv));
        }
    }

    for f in prelude_analysis.inline_type_fns.into_iter() {
        if analysis.symbols.lookup_anywhere(f.name.as_ref()).is_none() {
            let id = analysis.inline_type_fns.len();
            analysis
                .symbols
                .declare_inline_type_function(f.name.clone(), Span::default(), id);
            analysis.inline_type_fns.push(f);
        }
    }

    errs.into_iter().collect()
}
