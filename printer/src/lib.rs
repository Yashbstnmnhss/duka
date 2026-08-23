use crate::elements::{Chapter, Content, Element, Inline, List, Style, Table};

pub mod elements;
pub mod printers;

pub fn slug(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() || c != '_', "_")
}

pub struct ChapterBuilder {
    name: String,
    elements: Vec<Element>,
}

impl ChapterBuilder {
    pub fn name(name: String) -> Self {
        Self {
            name,
            elements: vec![],
        }
    }
    #[inline]
    pub fn divider(self) -> Self {
        self.push(Element::Divider)
    }
    #[inline]
    pub fn code(self, lang: Option<String>, code: String) -> Self {
        self.push(Element::Code(code, lang))
    }
    #[inline]
    pub fn list(self, list: List) -> Self {
        self.push(Element::List(list))
    }
    #[inline]
    pub fn table(self, table: Table) -> Self {
        self.push(Element::Table(table))
    }
    #[inline]
    pub fn content(self, content: Content) -> Self {
        self.push(Element::Content(content))
    }
    #[inline]
    pub fn header(self, level: u8, content: Content, toc: Option<String>) -> Self {
        self.push(Element::Header(level, content, toc))
    }
    pub fn push(mut self, element: Element) -> Self {
        self.elements.push(element);
        self
    }
    pub fn build(self) -> Chapter {
        Chapter {
            name: self.name,
            content: vec![],

            toc: false,
        }
    }
}

pub struct ContentBuilder {
    inner: Vec<Inline>,
}
impl ContentBuilder {
    pub fn new() -> Self {
        Self { inner: vec![] }
    }
    #[inline]
    pub fn styled(self, style: Style, content: Content) -> Self {
        self.push(Inline::Styled(content.0, style))
    }
    #[inline]
    pub fn code(self, str: impl Into<String>) -> Self {
        self.push(Inline::Code(str.into()))
    }
    #[inline]
    pub fn newline(self) -> Self {
        self.push(Inline::LineBreak)
    }
    #[inline]
    pub fn str(self, str: impl Into<String>) -> Self {
        self.push(Inline::Text(str.into()))
    }
    pub fn push(mut self, inline: Inline) -> Self {
        self.inner.push(inline);
        self
    }
    pub fn build_inline(self) -> Vec<Inline> {
        self.inner
    }
    pub fn build(self) -> Content {
        Content(self.inner)
    }
}

pub struct TableBuilder {
    headers: Vec<Content>,
    rows: Vec<Vec<Content>>,
    cols: usize,
}

impl TableBuilder {
    pub fn row(self) -> TableRowBuilder {
        TableRowBuilder {
            row: Vec::with_capacity(self.cols),
            inner: self,
        }
    }
    pub fn build(self) -> Table {
        Table {
            headers: self.headers,
            rows: self.rows,
        }
    }
}

pub struct TableRowBuilder {
    row: Vec<Content>,
    inner: TableBuilder,
}
impl TableRowBuilder {
    pub fn item(mut self, header: Content) -> Self {
        self.row.push(header);
        self
    }
    pub fn end(mut self) -> TableBuilder {
        self.inner.rows.push(self.row);
        self.inner
    }
}
pub struct TableHeaderBuilder {
    headers: Vec<Content>,
}
impl TableHeaderBuilder {
    pub fn start() -> Self {
        Self { headers: vec![] }
    }
    pub fn header(mut self, header: Content) -> Self {
        self.headers.push(header);
        self
    }
    pub fn end(self) -> TableBuilder {
        TableBuilder {
            cols: self.headers.len(),
            headers: self.headers,
            rows: vec![],
        }
    }
}
