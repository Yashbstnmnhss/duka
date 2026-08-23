use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use duka_lib::builtin;
use duka_lib::duka_shared::docs::{MetaInfo, MetaInfoFlag, MetaItemInfo, ParamMeta, ReturnMeta};
use miette::{IntoDiagnostic, Result};

pub fn gen_doc(output: Option<PathBuf>) -> Result<()> {
    let metas = builtin::all_builtin_metas();

    let root_path = output.unwrap_or("./docs/references/".into());
    if !root_path.exists() {
        std::fs::create_dir_all(&root_path).into_diagnostic()?;
    }

    let mut pages: BTreeMap<String, String> = BTreeMap::new();
    pages.insert("index.md".to_owned(), render_index(&metas));

    for meta in &metas {
        collect_pages(meta, &mut vec![], &mut pages);
    }

    let before = count_md(&root_path)?;
    write_pages(&root_path, &pages)?;
    let after = count_md(&root_path)?;
    println!(
        "Generated {} pages in '{}' ({} file(s) removed)",
        pages.len(),
        root_path.display(),
        before.saturating_sub(after)
    );
    Ok(())
}

fn render_index(metas: &[MetaInfo]) -> String {
    let mut out = String::new();
    out.push_str("# Standard Library\n\n");
    out.push_str("_Generated documentation for the DUKA standard library._\n");

    let modules: Vec<&MetaInfo> = metas
        .iter()
        .filter(|m| matches!(m.info, MetaItemInfo::Module { .. }))
        .collect();
    if !modules.is_empty() {
        out.push_str("\n## Modules\n\n");
        for m in modules {
            out.push_str(&format!("- [{}](./{}.md)\n", m.name, m.name));
        }
    }

    let globals: Vec<&MetaInfo> = metas
        .iter()
        .filter(|m| !matches!(m.info, MetaItemInfo::Module { .. }))
        .collect();
    if !globals.is_empty() {
        out.push_str("\n## Globals\n");
        for m in globals {
            out.push_str(&render_item(m, 2, &slugify(m.name)));
        }
    }
    out
}

fn collect_pages(meta: &MetaInfo, path: &mut Vec<String>, pages: &mut BTreeMap<String, String>) {
    match &meta.info {
        MetaItemInfo::Module { inner } => {
            path.push(meta.name.to_owned());
            pages.insert(path.join("/") + ".md", render_module_page(path, meta));
            for child in inner.iter() {
                collect_pages(child, path, pages);
            }
            path.pop();
        }
        _ => {}
    }
}

fn render_flags(flags: &[MetaInfoFlag]) -> String {
    if flags.is_empty() {
        return "".to_owned();
    }
    format!(
        "\n## Flags\n{}\n",
        flags
            .iter()
            .map(|i| {
                format!(
                    "@{}({})",
                    i.0,
                    i.1.iter()
                        .map(|i| (*i).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn render_module_page(path: &[String], meta: &MetaInfo) -> String {
    let full_name = path.join(".");
    let index_link = rel_link(path, "index.md");

    let mut out = String::new();
    out.push_str(&heading(1, &full_name));
    out.push('\n');
    out.push_str(&format!("\n[Index]({index_link})\n"));
    if !meta.doc.is_empty() {
        out.push_str(&format!("\n<blockquote>\n{}\n</blockquote>\n", meta.doc));
    }
    out.push_str(&render_example(meta.example));
    out.push_str(&render_flags(meta.flags));

    let children = child_modules(meta);
    if !children.is_empty() {
        out.push_str("\n## Modules\n\n");
        for child in children {
            let mut child_path = path.to_vec();
            child_path.push(child.name.to_owned());
            let link = rel_link(path, &(child_path.join("/") + ".md"));
            out.push_str(&format!("- [{}]({})\n", child.name, link));
        }
    }

    let members: Vec<&MetaInfo> = child_members(meta);
    if !members.is_empty() {
        out.push_str("\n## Contents\n\n");
        for m in &members {
            out.push_str(&format!("- [{}](#{})\n", m.name, slugify(m.name)));
        }
        out.push_str("\n## Members\n");
        for m in members {
            out.push_str(&render_item(m, 3, &slugify(m.name)));
        }
    }
    out
}

fn child_modules(meta: &MetaInfo) -> Vec<&MetaInfo> {
    match &meta.info {
        MetaItemInfo::Module { inner } => inner
            .iter()
            .filter(|i| matches!(i.info, MetaItemInfo::Module { .. }))
            .collect(),
        _ => vec![],
    }
}

fn child_members(meta: &MetaInfo) -> Vec<&MetaInfo> {
    match &meta.info {
        MetaItemInfo::Module { inner } => inner
            .iter()
            .filter(|i| !matches!(i.info, MetaItemInfo::Module { .. }))
            .collect(),
        _ => vec![],
    }
}

fn render_item(meta: &MetaInfo, level: usize, anchor: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("\n<a id=\"{anchor}\"></a>\n"));

    let title = match &meta.info {
        MetaItemInfo::Function { .. } => render_signature(meta),
        MetaItemInfo::UserData { ty_name, .. } => format!("UserData `{ty_name}`"),
        MetaItemInfo::Constant { ty, .. } => format!("Constant `{}: {}`", meta.name, ty),
        MetaItemInfo::Static { inner } => format!("Static `{}`({})", meta.name, inner.name),
        _ => meta.name.to_owned(),
    };
    out.push_str(&heading(level, &title));
    out.push('\n');
    if !meta.doc.is_empty() {
        out.push_str(&format!("\n<blockquote>\n{}\n</blockquote>\n", meta.doc));
    }
    out.push_str(&render_flags(meta.flags));

    match &meta.info {
        MetaItemInfo::Constant { ty, val } => {
            out.push_str(&format!("\n- Type: {ty}\n- Value: `{val}`\n"));
        }
        MetaItemInfo::Function { params, returns } => {
            if !params.is_empty() {
                out.push_str("\n## Params\n\n");
                out.push_str("| Name | Type | VarArg? | Optional? | Default | Doc |\n");
                out.push_str("| :--- | :---: | :---: | :---: | :---: | :--- |\n");
                for p in params.iter() {
                    out.push_str(&render_param_row(p));
                }
            }
            if render_returns(returns).is_some() || !returns.text.is_empty() {
                out.push_str("\n## Returns\n\n");
                if let Some(summary) = render_returns(returns) {
                    out.push_str(&format!("`{summary}`\n\n"));
                }
                out.push_str(&returns.text);
                out.push('\n');
                out.push_str("\n| Index | Type |\n| :--- | :---: |\n");
                for (i, t) in returns.tys.iter().enumerate() {
                    out.push_str(&format!("| {i} | {t} |\n"));
                }
                if returns.var_arg {
                    out.push_str("| - | `...` |\n");
                }
            }
        }
        MetaItemInfo::UserData { ty_name, methods } if !methods.is_empty() => {
            out.push_str("\n## Methods\n");
            for m in methods.iter() {
                out.push_str(&render_item(
                    m,
                    level + 1,
                    &slugify(&format!("{ty_name}.{}", m.name)),
                ));
            }
        }
        MetaItemInfo::Static { inner } => {
            out.push_str(&format!("\nSee [here](#{})\n", slugify(inner.name)));
        }
        _ => (),
    }

    out.push_str(&render_example(meta.example));
    out
}

fn render_param_row(p: &ParamMeta) -> String {
    let shown = if p.var_arg {
        format!("...{}", p.name)
    } else {
        p.name.to_owned()
    };
    let name = format!("`{shown}`");
    let ty = escape_cell(&p.ty.to_string());
    let var_arg = if p.var_arg { "*true*" } else { "*false*" };
    let optional = if p.optional { "*true*" } else { "*false*" };
    let default = p
        .default
        .map(|d| format!("`{}`", escape_cell(d)))
        .unwrap_or_else(|| {
            if p.var_arg {
                "-".to_owned()
            } else {
                "*required*".to_owned()
            }
        });
    let doc = escape_cell(p.doc.unwrap_or("-"));
    format!("| {name} | {ty} | {var_arg} | {optional} | {default} | {doc} |\n")
}

fn render_signature(meta: &MetaInfo) -> String {
    match &meta.info {
        MetaItemInfo::Function { params, returns } => {
            let params = params
                .iter()
                .map(render_param)
                .collect::<Vec<_>>()
                .join(", ");
            match render_returns(returns) {
                Some(r) => format!("`{}({}) -> {}`", meta.name, params, r),
                None => format!("`{}({})`", meta.name, params),
            }
        }
        _ => meta.name.to_owned(),
    }
}

fn render_param(p: &ParamMeta) -> String {
    let name = if p.var_arg {
        format!("...{}", p.name)
    } else {
        p.name.to_owned()
    };
    let ty = p.ty.to_string();
    if p.var_arg {
        format!("{name}: {ty}")
    } else if let Some(def) = p.default {
        format!("{name}: {ty} = {def}")
    } else if p.optional {
        format!("{name}?: {ty}")
    } else {
        format!("{name}: {ty}")
    }
}

fn render_returns(r: &ReturnMeta) -> Option<String> {
    let mut parts = r.tys.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    if r.var_arg {
        parts.push("...".to_owned());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

fn render_example(example: Option<&'static str>) -> String {
    match example {
        Some(e) => format!("\n## Example\n\n```lua\n{e}\n```\n"),
        None => String::new(),
    }
}

fn heading(level: usize, title: &str) -> String {
    format!("{} {title}", "#".repeat(level))
}

fn rel_link(from_path: &[String], to_rel: &str) -> String {
    let depth = from_path.len().saturating_sub(1);
    "../".repeat(depth) + to_rel
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

fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn write_pages(root: &Path, pages: &BTreeMap<String, String>) -> Result<()> {
    for (rel, content) in pages {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).into_diagnostic()?;
        }
        std::fs::write(path, content).into_diagnostic()?;
    }
    cleanup_stale(root, pages)
}

fn count_md(root: &Path) -> Result<usize> {
    let mut n = 0;
    walk_files(root, &mut |_| n += 1)?;
    Ok(n)
}

fn walk_files(dir: &Path, f: &mut impl FnMut(&Path)) -> Result<()> {
    for entry in std::fs::read_dir(dir).into_diagnostic()? {
        let entry = entry.into_diagnostic()?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, f)?;
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            f(&path);
        }
    }
    Ok(())
}

fn cleanup_stale(root: &Path, keep: &BTreeMap<String, String>) -> Result<()> {
    for entry in std::fs::read_dir(root).into_diagnostic()? {
        let entry = entry.into_diagnostic()?;
        let path = entry.path();
        if path.is_dir() {
            cleanup_stale(&path, keep)?;
            if std::fs::read_dir(&path).into_diagnostic()?.next().is_none() {
                std::fs::remove_dir(&path).into_diagnostic()?;
            }
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            let rel = path.strip_prefix(root).into_diagnostic()?;
            let key = rel.to_string_lossy().replace('\\', "/");
            if !keep.contains_key(&key) {
                std::fs::remove_file(&path).into_diagnostic()?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_lib::duka_shared::{docs::DocType, dtype::Type};

    fn fn_meta(name: &'static str) -> MetaInfo {
        MetaInfo {
            name,
            doc: "docs `here`",
            example: Some("print(1)"),
            info: MetaItemInfo::Function {
                returns: ReturnMeta {
                    text: "the result",
                    var_arg: false,
                    tys: &[DocType::Base(Type::Int)],
                },
                params: &[ParamMeta {
                    name: "x",
                    ty: DocType::Base(Type::Int),
                    optional: false,
                    default: None,
                    var_arg: false,
                    doc: Some("first | param"),
                }],
            },
            flags: &[],
        }
    }

    #[test]
    fn signature_plain() {
        assert_eq!(render_signature(&fn_meta("f")), "`f(x: int) -> int`");
    }

    #[test]
    fn param_forms() {
        let base = |name: &'static str,
                    optional: bool,
                    default: Option<&'static str>,
                    var_arg: bool| ParamMeta {
            name,
            ty: DocType::Base(Type::Any),
            optional,
            default,
            var_arg,
            doc: None,
        };
        assert_eq!(render_param(&base("a", false, None, false)), "a: any");
        assert_eq!(render_param(&base("a", true, None, false)), "a?: any");
        assert_eq!(
            render_param(&base("a", true, Some("1"), false)),
            "a: any = 1"
        );
        assert_eq!(render_param(&base("a", false, None, true)), "...a: any");
    }

    #[test]
    fn returns_none_when_empty() {
        let r = ReturnMeta {
            text: "",
            var_arg: false,
            tys: &[],
        };
        assert_eq!(render_returns(&r), None);
    }

    #[test]
    fn slugify_ident() {
        assert_eq!(slugify("raw_get"), "raw_get");
        assert_eq!(slugify("Type.Name"), "type-name");
    }

    #[test]
    fn cell_escapes_pipe_and_newline() {
        assert_eq!(escape_cell("a | b\nc"), "a \\| b c");
    }

    #[test]
    fn item_anchor_and_example() {
        let out = render_item(&fn_meta("f"), 3, "f");
        assert!(out.contains("<a id=\"f\"></a>"));
        assert!(out.contains("### `f(x: int) -> int`"));
        assert!(out.contains("```lua"));
        assert!(out.contains("first \\| param"));
    }

    #[test]
    fn rel_link_depth() {
        assert_eq!(rel_link(&["a".to_owned()], "index.md"), "index.md");
        assert_eq!(
            rel_link(&["a".to_owned(), "b".to_owned()], "index.md"),
            "../index.md"
        );
        assert_eq!(rel_link(&["a".to_owned()], "a/b.md"), "a/b.md");
    }

    #[test]
    fn module_renders_children_and_contents() {
        let inner = MetaInfo {
            flags: &[],
            name: "g",
            doc: "",
            example: None,
            info: MetaItemInfo::Function {
                returns: ReturnMeta {
                    text: "",
                    var_arg: false,
                    tys: &[],
                },
                params: &[],
            },
        };
        let nested = MetaInfo {
            flags: &[],
            name: "sub",
            doc: "sub module",
            example: None,
            info: MetaItemInfo::Module { inner: &[] },
        };
        let module = MetaInfo {
            flags: &[],
            name: "m",
            doc: "module doc",
            example: None,
            info: MetaItemInfo::Module {
                inner: Box::leak(Box::new([inner, nested])),
            },
        };
        let path = vec!["m".to_owned()];
        let out = render_module_page(&path, &module);
        assert!(out.contains("# m"));
        assert!(out.contains("<blockquote>\nmodule doc"));
        assert!(out.contains("## Modules"));
        assert!(out.contains("- [sub](m/sub.md)"));
        assert!(out.contains("## Contents"));
        assert!(out.contains("- [g](#g)"));
        assert!(out.contains("## Members"));
        assert!(out.contains("[Index](index.md)"));
    }
}
