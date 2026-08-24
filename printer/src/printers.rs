//! Printers & renderers to convert documents into strings & files

use std::{borrow::Cow, io::Write, path::PathBuf};

use crate::{
    elements::{Book, Chapter, Element, Inline, Link, Style},
    slug,
};

pub enum ExternalLink<'a> {
    Chapter(&'a str),
}
pub enum RenderFragment<'a> {
    Text(Cow<'a, str>),
    Link(ExternalLink<'a>),
}
impl<'a> From<&'a str> for RenderFragment<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(Cow::Borrowed(value))
    }
}
impl From<String> for RenderFragment<'_> {
    fn from(value: String) -> Self {
        Self::Text(Cow::Owned(value))
    }
}

pub trait Renderer {
    fn render_book_index<'a>(&mut self, book: &'a Book) -> Vec<RenderFragment<'a>>;
    fn render_chapter<'a>(&mut self, chapter: &'a Chapter) -> Vec<RenderFragment<'a>>;
}

pub struct MarkdownRenderer;
impl MarkdownRenderer {
    fn render_inlines(content: &[Inline]) -> Vec<RenderFragment<'_>> {
        let mut results = vec![];
        for c in content {
            match c {
                Inline::Text(t) => results.push(t.as_str().into()),
                Inline::Styled(inlines, style) => match style {
                    Style::Bold => {
                        results.push("**".into());
                        results.extend(Self::render_inlines(inlines));
                        results.push("**".into());
                    }
                    Style::Quote => {
                        results.push("> ".into());
                        results.extend(Self::render_inlines(inlines));
                        results.push("\n\n".into()); // END QUOTE!
                    }
                },
                Inline::Code(c) => {
                    results.push(format!("`{c}`").into());
                }
                Inline::Link(inlines, link) => {
                    results.push("[".into());
                    results.extend(Self::render_inlines(inlines));
                    match link {
                        Link::Anchor(a) => {
                            results.push(format!("](#{})", slug(a)).into());
                        }
                        Link::Chapter(c, a) => {
                            results.push("](".into());
                            results.push(RenderFragment::Link(ExternalLink::Chapter(c)));
                            results.push(format!("{})", a.as_deref().unwrap_or_default()).into());
                        }
                        Link::URL(u) => results.push(format!("]({u})").into()),
                    }
                }
                Inline::LineBreak => results.push("<br/>".into()),
            }
        }
        results
    }
}
impl Renderer for MarkdownRenderer {
    fn render_book_index<'a>(&mut self, book: &'a Book) -> Vec<RenderFragment<'a>> {
        let mut results = vec![format!("# {}\n", book.title).into()];
        for c in &book.content {
            results.push(format!("- [{}](", c.name).into());
            results.push(RenderFragment::Link(ExternalLink::Chapter(&c.name)));
            results.push(")\n".into());
        }
        results
    }
    fn render_chapter<'a>(
        &mut self,
        Chapter { name, content, toc }: &'a Chapter,
    ) -> Vec<RenderFragment<'a>> {
        let mut fragments = vec![];
        let mut tocs: Vec<(&str, String)> = vec![];

        for e in content {
            match e {
                Element::Header(level, content, toc_name) => {
                    fragments.push(format!("{} ", "#".repeat(*level as usize)).into());
                    fragments.extend(Self::render_inlines(&content.0));
                    fragments.push("\n\n".into());
                    if let Some(text) = toc_name {
                        let link_name = slug(text);
                        fragments.push(format!("<a id=\"{}\"></a>\n\n", link_name).into());
                        if *toc {
                            tocs.push((text, link_name));
                        }
                    }
                }
                Element::Anchor(text, _) => {
                    fragments.push(format!("<a id=\"{}\"></a>\n\n", slug(text)).into());
                }
                Element::Content(content) => {
                    fragments.extend(Self::render_inlines(&content.0));
                    fragments.push("\n\n".into());
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
                        fragments.push(" \n".into());
                    }
                    fragments.push("\n".into());
                }
                Element::Table(table) => {
                    fragments.push("| ".into());
                    fragments.extend(Self::render_inlines(&table.headers[0].0));
                    for h in table.headers.iter().skip(1) {
                        fragments.push(" | ".into());
                        fragments.extend(Self::render_inlines(&h.0));
                    }
                    fragments.push(" |\n".into());
                    fragments.push("|".into());
                    for _ in &table.headers {
                        fragments.push(" :--- |".into());
                    }
                    fragments.push("\n".into());
                    for row in &table.rows {
                        fragments.push("| ".into());
                        if let Some(first) = row.first() {
                            fragments.extend(Self::render_inlines(&first.0));
                        }
                        for cell in row.iter().skip(1) {
                            fragments.push(" | ".into());
                            fragments.extend(Self::render_inlines(&cell.0));
                        }
                        fragments.push(" |\n".into());
                    }
                    fragments.push("\n".into());
                }
                Element::Divider => fragments.push("---\n\n".into()),
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
        results.push("\n".into());
        results.extend(fragments);

        results
    }
}

pub trait Printer {
    type Error;
    fn print(&mut self, book: &Book) -> Result<(), Self::Error>;
}

pub struct FilePrinter<T: Renderer> {
    renderer: T,
    root: PathBuf,
    suffix: String,
}

impl<T: Renderer> FilePrinter<T> {
    /// SUFFIX WITHOUT `.` DOT
    pub fn new(renderer: T, root: PathBuf, suffix: String) -> Self {
        Self {
            renderer,
            root,
            suffix,
        }
    }
    fn write(&mut self, name: &str, fragments: Vec<RenderFragment>) -> Result<(), std::io::Error> {
        let path = self.root.join(name).with_extension(&self.suffix);
        let mut file = std::fs::File::create(path)?;
        for f in fragments {
            match f {
                RenderFragment::Link(l) => match l {
                    ExternalLink::Chapter(name) => {
                        file.write_all(format!("./{name}.{}", self.suffix).as_bytes())?;
                    }
                },
                RenderFragment::Text(t) => {
                    file.write_all(t.as_bytes())?;
                }
            }
        }
        file.flush()?;
        Ok(())
    }
}

impl<T: Renderer> Printer for FilePrinter<T> {
    type Error = std::io::Error;
    fn print(&mut self, book: &Book) -> Result<(), Self::Error> {
        if !self.root.exists() {
            std::fs::create_dir_all(&self.root)?;
        }
        if book.index {
            let f = self.renderer.render_book_index(book);
            self.write("index", f)?;
        }
        for c in &book.content {
            let f = self.renderer.render_chapter(c);
            self.write(&slug(&c.name), f)?;
        }
        Ok(())
    }
}
