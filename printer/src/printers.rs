use std::{collections::HashMap, marker::PhantomData, path::PathBuf};

use crate::{
    elements::{Book, Chapter, Content, Element, Inline, Link, Style},
    slug,
};

pub enum RenderFragment {
    Text(String),
    Link(Link),
}
impl From<String> for RenderFragment {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

pub trait Renderer {
    fn render(chapter: &Chapter) -> Vec<RenderFragment>;
}

pub struct MarkdownRenderer;
impl MarkdownRenderer {
    fn render_inlines(content: &[Inline]) -> Vec<RenderFragment> {
        let mut results = vec![];
        for c in content {
            match c {
                Inline::Text(t) => results.push(t.clone().into()),
                Inline::Styled(inlines, style) => match style {
                    Style::Bold => {
                        results.push("**".to_owned().into());
                        results.extend(Self::render_inlines(inlines));
                        results.push("**".to_owned().into());
                    }
                },
                Inline::Code(c) => {
                    results.push(format!("`{c}`").into());
                }
                Inline::Link(inlines, link) => {
                    results.push("[".to_owned().into());
                    results.extend(Self::render_inlines(inlines));
                    results.push("](".to_owned().into());
                    results.push(RenderFragment::Link(link.clone()));
                    results.push(")".to_owned().into());
                }
                Inline::LineBreak => results.push("<br/>".to_owned().into()),
            }
        }
        results
    }
}
impl Renderer for MarkdownRenderer {
    fn render(Chapter { name, content, toc }: &Chapter) -> Vec<RenderFragment> {
        let mut fragments = vec![];
        let mut tocs: Vec<(String, String)> = vec![];

        for e in content {
            match e {
                Element::Header(level, content, toc_name) => {
                    fragments.push(format!("{} ", "#".repeat(*level as usize)).into());
                    fragments.extend(Self::render_inlines(&content.0));
                    fragments.push("\n\n".to_owned().into());
                    if let Some(text) = toc_name {
                        let link_name = slug(text);
                        if *toc {
                            tocs.push((text.clone(), link_name.clone()));
                        }
                        fragments.push(format!("<a id=\"{}\"></a>\n\n", link_name).into());
                    }
                }
                Element::Anchor(text, toc) => {
                    fragments.push(format!("<a id=\"{text}\"></a>\n\n").into());
                }
                Element::Content(content) => {
                    fragments.extend(Self::render_inlines(&content.0));
                    fragments.push("\n\n".to_owned().into());
                }
                Element::List(list) => {
                    for (i, c) in list.items.iter().enumerate() {
                        fragments.push(
                            if list.ordered {
                                format!("{i}. ")
                            } else {
                                "- ".to_owned()
                            }
                            .into(),
                        );
                        fragments.extend(Self::render_inlines(&c.0));
                        fragments.push(" \n".to_owned().into());
                    }
                    fragments.push("\n".to_owned().into());
                }
                Element::Table(_table) => todo!(),
                Element::Divider => fragments.push("---\n\n".to_owned().into()),
                Element::Code(code, lang) => {
                    fragments.push(
                        format!(
                            "```{}\n{code}\n```\n\n",
                            lang.as_deref().unwrap_or_default()
                        )
                        .into(),
                    );
                }
            }
        }

        let mut results = vec![format!("# {name}\n\n").into()];
        for (name, anchor) in tocs {
            results.push(format!("- [{name}](#{anchor})\n").into());
        }
        results.push("\n".to_owned().into());
        results.extend(fragments);

        results
    }
}

pub trait Printer {
    fn print(&mut self, book: &Book) -> Result<(), String>;
}

pub struct FilePrinter<T: Renderer> {
    _data: PhantomData<T>,
    root: PathBuf,
}

impl<T: Renderer> Printer for FilePrinter<T> {
    fn print(&mut self, book: &Book) -> Result<(), String> {
        for c in &book.content {}
        Ok(())
    }
}
