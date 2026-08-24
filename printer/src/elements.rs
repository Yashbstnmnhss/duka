//! Structural definitions of documents

#[derive(Debug)]
pub struct Book {
    pub title: String,
    pub content: Vec<Chapter>,
    pub index: bool,
}

#[derive(Debug)]
pub struct Chapter {
    pub name: String,
    pub content: Vec<Element>,
    pub toc: bool,
}

#[derive(Debug)]
pub struct Table {
    pub headers: Vec<Content>,
    pub rows: Vec<Vec<Content>>,
}
#[derive(Debug)]
pub struct List {
    pub ordered: bool,
    pub items: Vec<Content>,
}

#[derive(Debug)]
pub enum Element {
    Header(u8, Content, Option<String>),
    Anchor(String, bool /* for toc */),
    Content(Content),
    List(List),
    Table(Table),
    Divider,
    Code(String, Option<String>),
}
#[derive(Debug)]
pub struct Content(pub Vec<Inline>);

impl From<String> for Content {
    fn from(value: String) -> Self {
        Self(vec![value.into()])
    }
}
impl From<String> for Inline {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

#[derive(Debug)]
pub enum Inline {
    Text(String),
    Styled(Vec<Inline>, Style),
    Code(String),
    Link(Vec<Inline>, Link),
    LineBreak,
}

#[derive(Debug)]
pub enum Style {
    Bold,
    Quote,
}
#[derive(Clone, Debug)]
pub enum Link {
    URL(String),
    Chapter(String, Option<String>),
    Anchor(String),
}
