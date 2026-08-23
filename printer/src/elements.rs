pub struct Book {
    content: Vec<Chapter>,
    title: String,
    index: bool,
}

pub struct Chapter {
    name: String,
    content: Vec<Element>,
    toc: bool,
}

pub struct Table {
    headers: Vec<Content>,
    contents: Vec<Vec<Content>>,
}

pub enum Element {
    Header(u8, Content),
    Anchor(String),
    Content(Content),
    List(Vec<Content>, bool),
    Table(Table),
    Divider,
    Code(String, Option<String>),
}
pub type Content = Vec<Inline>;

pub enum Inline {
    Text(String),
    Styled(Vec<Inline>, Style),
    Code(Vec<Inline>),
    Link(Vec<Inline>, String),
    LineBreak,
}

pub enum Style {
    Bold,
}
pub enum Link {
    URL(String),
    Chapter(String),
    Anchor(String),
}
