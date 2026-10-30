//! Hover cards.
//!
//! A hover is a small document: a fenced signature, the prose a declaration or
//! a builtin wrote about itself, and sometimes an example. Rendering it through
//! the same printer the documentation uses is what keeps the two from drifting:
//! one place says how a code block, a list and a line of prose are written.

use duka_printer::elements::{Chapter, Content, Element, Inline, List, Style};
use duka_printer::printers::{MarkdownRenderer, RenderFragment, Renderer};

/// One hover, built element by element and rendered at the end.
pub struct Card {
    elements: Vec<Element>,
}

impl Card {
    pub fn new() -> Self {
        Self { elements: vec![] }
    }

    /// A fenced block in the language the reader writes.
    pub fn signature(mut self, code: impl Into<String>) -> Self {
        self.elements
            .push(Element::Code(code.into(), Some("duka".to_owned())));
        self
    }

    /// A fenced block in another language.
    pub fn code(mut self, lang: &str, code: impl Into<String>) -> Self {
        self.elements
            .push(Element::Code(code.into(), Some(lang.to_owned())));
        self
    }

    /// A paragraph. Markdown written by a declaration is passed through as-is.
    pub fn prose(mut self, text: impl Into<String>) -> Self {
        self.elements
            .push(Element::Content(Content(vec![Inline::Text(text.into())])));
        self
    }

    /// A bullet per label, the label set as code.
    pub fn details(mut self, details: Vec<(String, String)>) -> Self {
        if details.is_empty() {
            return self;
        }
        let items = details
            .into_iter()
            .map(|(label, value)| {
                let mut line = vec![Inline::Code(label)];
                if !value.is_empty() {
                    line.push(Inline::Text(": ".to_owned()));
                    line.push(Inline::Text(value));
                }
                Content(line)
            })
            .collect();
        self.elements.push(Element::List(List {
            ordered: false,
            items,
        }));
        self
    }

    /// A bullet per line, each line already written as markdown.
    pub fn bullets(mut self, lines: Vec<String>) -> Self {
        if lines.is_empty() {
            return self;
        }
        let items = lines
            .into_iter()
            .map(|line| Content(vec![Inline::Text(line)]))
            .collect();
        self.elements.push(Element::List(List {
            ordered: false,
            items,
        }));
        self
    }

    /// Where a builtin came from, set apart from what it says.
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.elements
            .push(Element::Content(Content(vec![Inline::Styled(
                vec![Inline::Text(source.into())],
                Style::Italic,
            )])));
        self
    }

    pub fn render(self) -> String {
        render(self.elements)
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

/// The rendered text of a card's elements.
pub fn render(elements: Vec<Element>) -> String {
    let chapter = Chapter {
        name: String::new(),
        content: elements,
        toc: false,
    };
    let mut renderer = MarkdownRenderer;
    let text: String = renderer
        .render_chapter(&chapter)
        .into_iter()
        .filter_map(|fragment| match fragment {
            RenderFragment::Text(text) => Some(text.into_owned()),
            _ => None,
        })
        .collect();
    text.trim_end().to_owned()
}
