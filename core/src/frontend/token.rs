use duka_macros::Info;

use crate::shared::{
    types::Spanned,
    value::{DukaFloat, DukaInt},
};

pub type Token = Spanned<TokenKind>;

#[derive(Debug, PartialEq, Clone, Info)]
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
    #[tag(binop)]
    Equal,
    #[name("!=")]
    #[tag(binop)]
    NotEqual,
    #[name(">")]
    #[tag(binop)]
    Greater,
    #[name("<")]
    #[tag(binop)]
    Less,
    #[name(">=")]
    #[tag(binop)]
    GreaterEqual,
    #[name("<=")]
    #[tag(binop)]
    LessEqual,

    #[name("&")]
    #[tag(binop)]
    BitAnd,
    #[name("|")]
    #[tag(binop)]
    BitOr,
    #[name("~")]
    #[tag(binop)]
    #[tag(unop)]
    BitTilde,
    #[name("<<")]
    #[tag(binop)]
    ShiftL,
    #[name(">>")]
    #[tag(binop)]
    ShiftR,

    #[tag(binop)]
    And,
    #[tag(binop)]
    Or,
    #[tag(unop)]
    Not,
    #[tag(binop)]
    Xor,

    #[name("+")]
    #[tag(binop)]
    Plus,
    #[name("-")]
    #[tag(unop)]
    #[tag(binop)]
    Minus,
    #[name("*")]
    #[tag(binop)]
    Multiply,
    #[name("/")]
    #[tag(binop)]
    Divide,
    #[name("//")]
    #[tag(binop)]
    IDivide,
    #[name("%")]
    #[tag(binop)]
    Mod,
    #[name("**")]
    #[tag(binop)]
    Pow,
    /// ..
    #[name("..")]
    #[tag(binop)]
    Concat,
    /// .
    #[name(".")]
    Dot,
    /// ...
    #[name("...")]
    Dots,
    #[name("#")]
    #[tag(unop)]
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
    #[tag(terminator)]
    EOF,
    // Ignore
    //Comment(String),
    //Shebang(String),
    // erongI
}

impl TokenKind {
    #[inline]
    pub const fn terminator() -> Self {
        Self::EOF
    }
}
