use std::path::PathBuf;

use duka_lib::builtin;
use duka_lib::duka_frontend::analyzer::builtin::TYPE_BUILTINS_META;
use duka_lib::duka_shared::docs::{
    ATTR_DOCS, KEYWORD_DOCS, KeywordDoc, MetaInfo, MetaItemInfo, ReturnMeta, TYPE_DOCS,
};
use duka_printer::elements::{Book, Chapter, Content, Element, Inline, Link, List, Table};
use duka_printer::prelude::*;
use miette::{IntoDiagnostic, Result};

pub fn gen_doc(output: Option<PathBuf>) -> Result<()> {
    let mut metas = builtin::all_builtin_metas();
    metas.push(TYPE_BUILTINS_META);

    let root_path = output.clone().unwrap_or("./docs/references/".into());
    let book = build_book(&metas, "Standard Library".to_owned());

    let mut printer = FilePrinter::new(MarkdownRenderer, root_path, "md".to_owned());
    printer.print(&book).into_diagnostic()?;

    let lang_root = output.unwrap_or("./docs/language/".into());
    let lang_book = build_language_book();
    let mut lang_printer = FilePrinter::new(MarkdownRenderer, lang_root, "md".to_owned());
    lang_printer.print(&lang_book).into_diagnostic()?;
    Ok(())
}

fn build_language_book() -> Book {
    Book {
        title: "Language Reference".to_owned(),
        content: vec![
            build_keyword_chapter("Keywords", KEYWORD_DOCS.iter()),
            build_keyword_chapter("Types", TYPE_DOCS.iter()),
            build_keyword_chapter("Attributes", ATTR_DOCS.iter()),
        ],
        index: true,
    }
}

fn doc_to_elements(doc: &duka_lib::duka_shared::docs::Doc) -> Vec<Element> {
    let mut els = vec![];
    els.push(Element::Header(
        2,
        text(doc.title),
        Some(slugify(doc.title)),
    ));
    if !doc.content.is_empty() {
        els.push(Element::Content(text(doc.content)));
    }
    if let Some(e) = doc.example {
        els.push(Element::Header(3, text("Example"), None));
        els.push(Element::Code(e.to_owned(), Some("lua".to_owned())));
    }
    els
}

fn keyword_doc(d: &KeywordDoc) -> &duka_lib::duka_shared::docs::Doc {
    match d {
        KeywordDoc::Keyword { doc, .. } => doc,
        KeywordDoc::Type { doc, .. } => doc,
        KeywordDoc::Attribute { doc, .. } => doc,
    }
}

fn build_keyword_chapter(name: &str, items: impl Iterator<Item = &'static KeywordDoc>) -> Chapter {
    let collected: Vec<&'static KeywordDoc> = items.collect();
    let mut b = ChapterBuilder::name(name.to_owned()).toc(true);
    for d in collected {
        for e in doc_to_elements(keyword_doc(d)) {
            b = b.push(e);
        }
    }
    b.build()
}

fn build_book(metas: &[MetaInfo], title: String) -> Book {
    let mut chapters = vec![];
    for meta in metas {
        collect_chapters(meta, &mut vec![], &mut chapters);
    }
    Book {
        title,
        content: chapters,
        index: true,
    }
}

fn collect_chapters<'a>(meta: &'a MetaInfo, path: &mut Vec<String>, chapters: &mut Vec<Chapter>) {
    if let MetaItemInfo::Module { inner } = &meta.info {
        path.push(meta.name.to_owned());
        chapters.push(build_module_chapter(path, meta));
        for child in inner.iter() {
            collect_chapters(child, path, chapters);
        }
        path.pop();
    }
}

fn text(s: impl Into<String>) -> Content {
    ContentBuilder::new().str(s).build()
}

fn code(s: impl Into<String>) -> Content {
    Content(vec![Inline::Code(s.into())])
}

fn build_module_chapter(path: &[String], meta: &MetaInfo) -> Chapter {
    let full_name = path.join(".");
    let mut b = ChapterBuilder::name(full_name.clone()).toc(true);

    b = b.anchor(meta.name);
    if !meta.doc.is_empty() {
        b = b.content(text(format!("> {}", meta.doc.replace('\n', "\n> "))));
    }
    b = push_example(b, meta.example);
    b = push_flags(b, meta.flags);

    let children: Vec<&MetaInfo> = match &meta.info {
        MetaItemInfo::Module { inner } => inner
            .iter()
            .filter(|i| matches!(i.info, MetaItemInfo::Module { .. }))
            .collect(),
        _ => vec![],
    };
    if !children.is_empty() {
        b = b.header(2, text("Modules"), None);
        for child in children {
            let mut child_path = path.to_vec();
            child_path.push(child.name.to_owned());
            b = b.content(Content(vec![Inline::Link(
                vec![Inline::Text(child.name.to_owned())],
                Link::URL(format!("./{}.md", child_path.join("/"))),
            )]));
        }
    }

    let members: Vec<&MetaInfo> = match &meta.info {
        MetaItemInfo::Module { inner } => inner
            .iter()
            .filter(|i| !matches!(i.info, MetaItemInfo::Module { .. }))
            .collect(),
        _ => vec![],
    };
    if !members.is_empty() {
        b = b.header(2, text("Contents"), None);
        for m in &members {
            b = b.content(Content(vec![Inline::Link(
                vec![Inline::Text(m.name.to_owned())],
                Link::Anchor(slugify(&m.name)),
            )]));
        }
        b = b.header(2, text("Members"), None);
        for m in members {
            for e in build_item(m, 3) {
                b = b.push(e);
            }
        }
    }
    b.build()
}

fn build_item(meta: &MetaInfo, level: u8) -> Vec<Element> {
    let mut els = vec![];
    els.push(Element::Anchor(slugify(meta.name), true));
    els.push(Element::Header(level, build_title(meta), None));

    if !meta.doc.is_empty() {
        els.push(Element::Content(text(format!(
            "> {}",
            meta.doc.replace('\n', "\n> ")
        ))));
    }
    if !meta.flags.is_empty() {
        els.push(Element::Content(build_flags_content(meta.flags)));
    }

    match &meta.info {
        MetaItemInfo::Function { params, returns } => {
            if !params.is_empty() {
                els.push(Element::Header(4, text("Params"), None));
                els.push(Element::Table(build_param_table(params)));
            }
            if render_returns(returns).is_some() || !returns.text.is_empty() {
                els.push(Element::Header(4, text("Returns"), None));
                let mut cb = ContentBuilder::new();
                if let Some(summary) = render_returns(returns) {
                    cb = cb.code(summary);
                    cb = cb.newline();
                }
                if !returns.text.is_empty() {
                    cb = cb.str(returns.text);
                }
                els.push(Element::Content(cb.build()));
                els.push(Element::Table(build_return_table(returns)));
            }
        }
        MetaItemInfo::Constant { ty, val } => {
            let mut list = List {
                ordered: false,
                items: vec![],
            };
            list.items.push(text(format!("Type: `{ty}`")));
            list.items.push(code(format!("Value: {val}")));
            els.push(Element::List(list));
        }
        MetaItemInfo::UserData { ty_name, methods } if !methods.is_empty() => {
            els.push(Element::Header(4, text("Methods"), None));
            for m in methods.iter() {
                els.extend(build_item(m, level + 1));
            }
        }
        MetaItemInfo::Static { inner } => {
            els.push(Element::Content(Content(vec![Inline::Link(
                vec![Inline::Text("See here".to_owned())],
                Link::Anchor(slugify(inner.name)),
            )])));
        }
        _ => {}
    }

    if let Some(e) = meta.example {
        els.push(Element::Header(4, text("Example"), None));
        els.push(Element::Code(e.to_owned(), Some("lua".to_owned())));
    }
    els
}

fn build_title(meta: &MetaInfo) -> Content {
    match &meta.info {
        MetaItemInfo::Function { params, returns } => {
            let sig = signature(meta.name, params, returns);
            Content(vec![Inline::Code(sig)])
        }
        MetaItemInfo::UserData { ty_name, .. } => {
            text(format!("UserData `{}:{ty_name}`", meta.name))
        }
        MetaItemInfo::Constant { ty, .. } => text(format!("Constant `{}`: {ty}", meta.name)),
        MetaItemInfo::Static { inner } => text(format!("Static `{}`({})", meta.name, inner.name)),
        MetaItemInfo::TypeFunction { param_count } => text(format!(
            "type function `{}`({})",
            meta.name,
            (0..*param_count)
                .map(|_| "type")
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => text(meta.name),
    }
}

fn signature(
    name: &str,
    params: &[duka_lib::duka_shared::docs::ParamMeta],
    returns: &ReturnMeta,
) -> String {
    let ps = params
        .iter()
        .map(|p| {
            let n = if p.var_arg {
                format!("...{}", p.name)
            } else {
                p.name.to_owned()
            };
            if p.var_arg {
                format!("{n}: {}", p.ty)
            } else if let Some(d) = p.default {
                format!("{n}: {} = {d}", p.ty)
            } else if p.optional {
                format!("{n}?: {}", p.ty)
            } else {
                format!("{n}: {}", p.ty)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match render_returns(returns) {
        Some(r) => format!("{name}({ps}) -> {r}"),
        None => format!("{name}({ps})"),
    }
}

fn build_param_table(params: &[duka_lib::duka_shared::docs::ParamMeta]) -> Table {
    let mut tb = TableHeaderBuilder::start()
        .header(text("Name"))
        .header(text("Type"))
        .header(text("VarArg?"))
        .header(text("Optional?"))
        .header(text("Default"))
        .header(text("Doc"))
        .end();
    for p in params.iter() {
        let name = if p.var_arg {
            format!("...{}", p.name)
        } else {
            p.name.to_owned()
        };
        let default = p.default.map(|d| format!("`{d}`")).unwrap_or_else(|| {
            if p.var_arg {
                "-".into()
            } else {
                "*required*".into()
            }
        });
        tb = tb
            .row()
            .item(code(name))
            .item(code(p.ty.to_string()))
            .item(text(bool_cell(p.var_arg)))
            .item(text(bool_cell(p.optional)))
            .item(text(default))
            .item(text(p.doc.unwrap_or("-")))
            .end();
    }
    tb.build()
}

fn build_return_table(returns: &ReturnMeta) -> Table {
    let mut tb = TableHeaderBuilder::start()
        .header(text("Index"))
        .header(text("Type"))
        .end();
    for (i, t) in returns.tys.iter().enumerate() {
        tb = tb
            .row()
            .item(text(i.to_string()))
            .item(code(t.to_string()))
            .end();
    }
    if returns.var_arg {
        tb = tb.row().item(text("-")).item(code("...")).end();
    }
    tb.build()
}

fn bool_cell(b: bool) -> String {
    if b { "*true*" } else { "*false*" }.to_owned()
}

fn render_returns(r: &ReturnMeta) -> Option<String> {
    let mut parts = r.tys.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    if r.var_arg {
        parts.push("...".to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn push_example(b: ChapterBuilder, example: Option<&'static str>) -> ChapterBuilder {
    match example {
        Some(e) => b.code(Some("lua".to_owned()), e.to_owned()),
        None => b,
    }
}

fn push_flags(
    b: ChapterBuilder,
    flags: &[duka_lib::duka_shared::docs::MetaInfoFlag],
) -> ChapterBuilder {
    if flags.is_empty() {
        return b;
    }
    b.content(build_flags_content(flags))
}

fn build_flags_content(flags: &[duka_lib::duka_shared::docs::MetaInfoFlag]) -> Content {
    let joined = flags
        .iter()
        .map(|(k, v)| {
            format!(
                "@{}({})",
                k,
                v.iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    text(format!("Flags: `{joined}`"))
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}
