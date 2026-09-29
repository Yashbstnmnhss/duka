//! One place every piece of documentation is looked up in, and one shape it is
//! shown in.
//!
//! There are three sources and none of them is the symbol table: the keyword
//! and type tables the compiler ships, the type context builtins, and the
//! standard library. `DocView` is what they all turn into, so a hover reads the
//! same whether it is describing a keyword, a type function or `string.upper`,
//! and whatever produces documentation later reads the same view.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::LazyLock;

use duka_lib::duka_frontend::analyzer::builtin::TYPE_BUILTINS_META;
use duka_lib::builtin::all_builtin_registrations;
use duka_lib::duka_shared::docs::{
    Doc, MetaInfo, MetaInfoFlag, MetaItemInfo, attr_doc, keyword_doc, type_doc,
};
use duka_lib::duka_shared::dtype::Type;

/// What a hover shows, independent of where the documentation came from.
pub struct DocView<'a> {
    /// The name it is written under, which is what a completion item is labelled
    /// with
    pub name: Cow<'a, str>,
    /// The declaration, in the language itself so the editor highlights it
    pub signature: Option<String>,
    /// Prose, already markdown
    pub content: &'a str,
    pub example: Option<&'a str>,
    /// Label and value pairs worth listing under the prose
    pub details: Vec<(String, String)>,
    /// Where the documentation came from, so a reader can tell a builtin apart
    /// from something written in this project
    pub source: Option<&'static str>,
    /// a module holds other things rather than being one itself
    pub is_module: bool,
}

impl DocView<'_> {
    pub fn render(&self) -> String {
        let mut out = String::new();
        if let Some(signature) = &self.signature {
            let _ = writeln!(out, "```duka\n{signature}\n```");
        }
        if !self.content.is_empty() {
            out.push_str(self.content);
            out.push('\n');
        }
        for (label, value) in &self.details {
            let _ = writeln!(out, "\n- `{label}`: {value}");
        }
        if let Some(example) = self.example {
            let _ = writeln!(out, "\n```duka\n{example}\n```");
        }
        if let Some(source) = self.source {
            let _ = writeln!(out, "\n*{source}*");
        }
        out
    }

    fn plain(doc: &Doc) -> DocView<'_> {
        DocView {
            name: Cow::Borrowed(doc.title),
            signature: Some(doc.title.to_owned()),
            content: doc.content,
            example: doc.example,
            details: vec![],
            source: None,
            is_module: false,
        }
    }
}

pub fn keyword_view(name: &str) -> Option<DocView<'static>> {
    keyword_doc(name).map(DocView::plain)
}

pub fn type_view(ty: &Type) -> Option<DocView<'static>> {
    type_doc(ty).map(DocView::plain)
}

/// An attribute is worth describing whether or not the table has an entry for
/// it: `@name` is a syntactic form of its own, so letting an undocumented one
/// fall through to the unresolved hover would be describing the wrong thing.
pub fn attr_view(name: &str) -> DocView<'static> {
    match attr_doc(name) {
        Some(doc) => DocView::plain(doc),
        None => DocView {
            name: Cow::Owned(name.to_owned()),
            signature: Some(format!("@{name}")),
            content: "",
            example: None,
            details: vec![],
            source: None,
            is_module: false,
        },
    }
}

static VALUE_BUILTINS: LazyLock<Vec<MetaInfo>> = LazyLock::new(|| {
    all_builtin_registrations()
        .into_iter()
        .map(|(_, m)| m)
        .collect()
});

/// The name of the module whose members are global rather than reached through
/// a module table.
const CORE_MODULE: &str = "core";

fn module_at<'a>(metas: &'a [MetaInfo], path: &str) -> Option<&'a MetaInfo> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = metas.iter().find(|m| m.name == first)?;
    for part in parts {
        let MetaItemInfo::Module { inner } = &current.info else {
            return None;
        };
        current = inner.iter().find(|m| m.name == part)?;
    }
    Some(current)
}

/// The member of the core module a global name refers to, which is where a
/// bare builtin lives: `print` is `core.print`.
fn core_member<'a>(metas: &'a [MetaInfo], name: &str) -> Option<&'a MetaInfo> {
    metas
        .iter()
        .find(|m| m.name == CORE_MODULE)?
        .info
        .as_module()?
        .iter()
        .find(|m| m.name == name)
}

/// The documentation of a value the standard library provides, addressed by the
/// path it is written with: `print`, `os`, or `string.upper`.
pub fn value_builtin_view(path: &str) -> Option<DocView<'static>> {
    let metas = &*VALUE_BUILTINS;
    let found = if path.contains('.') {
        module_at(metas, path)?
    } else {
        // a global builtin is a member of the core module, a module is named
        // on its own
        core_member(metas, path).or_else(|| metas.iter().find(|m| m.name == path))?
    };
    Some(builtin_view(found))
}

/// What the standard library puts in the value namespace at global scope: the
/// core functions and constants, plus the modules that hang off them.
pub fn value_builtin_globals() -> Vec<DocView<'static>> {
    let metas = &*VALUE_BUILTINS;
    let mut out = vec![];
    let Some(MetaItemInfo::Module { inner }) = module_at(metas, CORE_MODULE).map(|m| &m.info)
    else {
        return out;
    };
    for member in inner.iter() {
        out.push(builtin_view(member));
    }
    for module in metas.iter().filter(|m| m.name != CORE_MODULE) {
        out.push(builtin_view(module));
    }
    out
}

/// The members of a standard library module, for completion after a `.`.
pub fn value_builtin_members(path: &str) -> Vec<DocView<'static>> {
    let Some(MetaItemInfo::Module { inner }) = module_at(&VALUE_BUILTINS, path).map(|m| &m.info)
    else {
        return vec![];
    };
    inner.iter().map(builtin_view).collect()
}

/// The type context builtins. They are never symbols, so nothing else can reach
/// them, and they only mean anything in a type position.
pub fn type_builtin_views() -> Vec<DocView<'static>> {
    let MetaItemInfo::Module { inner } = &TYPE_BUILTINS_META.info else {
        return vec![];
    };
    inner.iter().map(builtin_view).collect()
}

/// The documentation of a type context builtin such as `Error` or `Assert`.
pub fn type_builtin_view(name: &str) -> Option<DocView<'static>> {
    type_builtin_views()
        .into_iter()
        .find(|view| view.name == name)
}

/// The parameter names a call to `path` has to be labelled with, which the
/// inlay hints need for a builtin: no declaration exists to read them off.
pub fn value_builtin_params(path: &str) -> Option<Vec<String>> {
    let metas = &*VALUE_BUILTINS;
    let meta = if path.contains('.') {
        module_at(metas, path)?
    } else {
        core_member(metas, path)?
    };
    let params = match &meta.info {
        MetaItemInfo::Function { params, .. } => *params,
        MetaItemInfo::Static { inner } => match &inner.info {
            MetaItemInfo::Function { params, .. } => *params,
            _ => return None,
        },
        _ => return None,
    };
    Some(
        params
            .iter()
            .map(|p| {
                if p.var_arg {
                    "...".to_owned()
                } else {
                    p.name.to_owned()
                }
            })
            .collect(),
    )
}

fn builtin_view(meta: &MetaInfo) -> DocView<'static> {
    DocView {
        name: Cow::Borrowed(meta.name),
        signature: signature_of(meta),
        content: meta.doc,
        example: meta.example,
        details: details_of(meta),
        source: Some("standard library"),
        is_module: matches!(meta.info, MetaItemInfo::Module { .. }),
    }
}

/// `Substring of a string`, written the way the language writes it so the
/// editor can highlight it and the reader can compare it with their own code.
/// The declaration, in exactly the shape a hover for the same thing written in
/// a project uses. A builtin goes through the same type the analyser would
/// build, so the two never drift apart.
fn signature_of(meta: &MetaInfo) -> Option<String> {
    Some(match &meta.info {
        MetaItemInfo::Function { .. } => format!("function {}: {}", meta.name, meta.get_type()),
        MetaItemInfo::Constant { ty, val } => format!("const {}: {} = {}", meta.name, ty, val),
        MetaItemInfo::TypeFunction { .. } => {
            format!("type function {}: {}", meta.name, meta.get_type())
        }
        MetaItemInfo::UserData { ty_name, methods } => {
            let mut out = format!("userdata {ty_name}");
            if !methods.is_empty() {
                let _ = write!(
                    out,
                    "({})",
                    methods
                        .iter()
                        .map(|m| m.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            out
        }
        MetaItemInfo::Module { .. } => format!("module {}", meta.name),
        MetaItemInfo::Static { inner } => return signature_of(inner),
    })
}

/// What the signature leaves out and the prose does not say: what each
/// parameter is called, what its default is, and the flags the builtin carries.
fn details_of(meta: &MetaInfo) -> Vec<(String, String)> {
    let mut out = vec![];
    let (params, flags) = match &meta.info {
        MetaItemInfo::Function { params, .. } => (Some(*params), meta.flags),
        MetaItemInfo::Static { inner } => match &inner.info {
            MetaItemInfo::Function { params, .. } => (Some(*params), meta.flags),
            _ => (None, meta.flags),
        },
        _ => (None, meta.flags),
    };
    for p in params.unwrap_or_default() {
        let mut label = if p.var_arg {
            "...".to_owned()
        } else if p.optional {
            format!("{}?", p.name)
        } else {
            p.name.to_owned()
        };
        if let Some(default) = p.default {
            let _ = write!(label, " = {default}");
        }
        match p.doc {
            Some(doc) => out.push((label, doc.to_owned())),
            None => out.push((label, p.ty.to_string())),
        }
    }
    for (key, values) in flags {
        out.push((key.to_string(), values.join(", ")));
    }
    out
}

/// The flags a builtin declares, for tests and for anything that has to answer
/// "is this only available here".
pub fn builtin_flags(meta: &MetaInfo) -> &'static [MetaInfoFlag] {
    meta.flags
}

trait AsModule {
    fn as_module(&self) -> Option<&'static [MetaInfo]>;
}

impl AsModule for MetaItemInfo {
    fn as_module(&self) -> Option<&'static [MetaInfo]> {
        match self {
            MetaItemInfo::Module { inner } => Some(inner),
            _ => None,
        }
    }
}
