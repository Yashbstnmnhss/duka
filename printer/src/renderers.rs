use crate::elements::Book;

pub trait Printer {
    fn print(book: &Book) -> String;
}

pub struct MarkdownPrinter;

impl Printer for MarkdownPrinter {
    fn print(book: &Book) -> String {
        "".to_owned()
    }
}
