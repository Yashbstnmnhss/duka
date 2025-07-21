use duka_macros::NameTag;

use crate::shared::{
    types::Spanned,
    value::{DukaFloat, DukaInt},
};

pub type Token = Spanned<TokenKind>;

#[derive(Debug, PartialEq, Clone, NameTag)]
pub enum TokenKind {
    Local,
    Function,
    Return,
    End,
    If,
    Else,
    Elseif,
    Goto,
    For,
    While,
    Break,
    Continue,
    In,
    Then,
    Do,

    #[name("=")]
    Assign,
    #[name("==")]
    Equal,
    #[name("!=")]
    NotEqual,
    #[name(">")]
    Greater,
    #[name("<")]
    Less,
    #[name(">=")]
    GreaterEqual,
    #[name("<=")]
    LessEqual,

    #[name("&")]
    BitAnd,
    #[name("|")]
    BitOr,
    #[name("~")]
    BitTilde,
    #[name("<<")]
    ShiftL,
    #[name(">>")]
    ShiftR,

    And,
    Or,
    Not,
    Xor,

    #[name("+")]
    Plus,
    #[name("-")]
    Minus,
    #[name("*")]
    Multiply,
    #[name("/")]
    Divide,
    #[name("//")]
    IDivide,
    #[name("%")]
    Mod,
    #[name("**")]
    Pow,
    /// ..
    #[name("..")]
    Concat,
    /// .
    #[name(".")]
    Dot,
    /// ...
    #[name("...")]
    Dots,
    #[name("#")]
    Length,

    /// [
    #[name("[")]
    LBracket,
    /// [
    #[name("]")]
    RBracket,
    #[name("{")]
    /// {
    LBrace,
    #[name("}")]
    /// }
    RBrace,
    #[name("(")]
    /// (
    LParen,
    #[name(")")]
    /// )
    RParen,

    #[name("::")]
    DoubleColon,
    #[name(";")]
    SemiColon,
    #[name(":")]
    Colon,
    #[name(",")]
    Comma,

    // <attr>
    // Do not use it
    //Attr(String),
    #[name("<identifier>")]
    Ident(String),
    True,
    False,
    #[name("<string>")]
    String(Vec<u8>),
    #[name("<integer>")]
    Int(DukaInt),
    #[name("<float>")]
    Float(DukaFloat),
    Nil,

    /// ## Special mark
    #[name("End of file marker")]
    EOF,
    // Ignore
    //Comment(String),
    //Shebang(String),
    // erongI
}

impl TokenKind {
    #[inline]
    pub const fn is_end(&self) -> bool {
        match self {
            TokenKind::EOF => true,
            _ => false,
        }
    }
}
