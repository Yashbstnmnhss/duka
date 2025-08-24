use duka_macros::Info;

use crate::{
    error::Span,
    types::Spanned,
    value::{DukaFloat, DukaInt},
};

pub type Token = Spanned<TokenKind>;
pub static EMPTY_TOKEN: Token = (TokenKind::terminator(), Span::EMPTY);

#[derive(Debug, PartialEq, Clone, Info, Default)]
pub enum TokenKind {
    #[name("!")]
    Bang,
    #[name("?")]
    Question,

    //TODO
    #[tag(keyword)]
    Match,
    #[tag(keyword)]
    Object,
    //ODOT
    #[tag(keyword)]
    Global,
    #[tag(keyword)]
    Local,
    #[tag(keyword)]
    Function,
    #[tag(keyword)]
    Return,
    #[tag(keyword)]
    End,
    #[tag(keyword)]
    If,
    #[tag(keyword)]
    Else,
    #[tag(keyword)]
    Elseif,
    #[tag(keyword)]
    Goto,
    #[tag(keyword)]
    For,
    #[tag(keyword)]
    While,
    #[tag(keyword)]
    Break,
    #[tag(keyword)]
    Continue,
    #[tag(keyword)]
    In,
    #[tag(keyword)]
    Then,
    #[tag(keyword)]
    Do,

    #[name("=")]
    Assign,
    #[name("==")]
    #[tag(binop)]
    #[tag(compare)]
    Equal,
    #[name("!=")]
    #[tag(binop)]
    #[tag(compare)]
    NotEqual,
    #[name(">")]
    #[tag(binop)]
    #[tag(compare)]
    Greater,
    #[name("<")]
    #[tag(binop)]
    #[tag(compare)]
    Less,
    #[name(">=")]
    #[tag(binop)]
    #[tag(compare)]
    GreaterEqual,
    #[name("<=")]
    #[tag(binop)]
    #[tag(compare)]
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
    #[tag(keyword)]
    #[tag(patop)]
    And,
    #[tag(binop)]
    #[tag(keyword)]
    #[tag(patop)]
    Or,
    #[tag(unop)]
    #[tag(keyword)]
    Not,
    #[tag(binop)]
    #[tag(keyword)]
    #[tag(patop)]
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
    #[name("^")]
    #[tag(binop)]
    Pow,
    /// ..
    #[name("..")]
    #[tag(binop)]
    Concat,
    #[name("|>")]
    #[tag(binop)]
    Pipeline,
    #[name("<|")]
    #[tag(binop)]
    PipelineL,
    /// ->
    #[name("->")]
    Arrow,
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
    #[tag(left)]
    LBracket,
    /// [
    #[name("]")]
    #[tag(right)]
    RBracket,
    #[name("{")]
    #[tag(left)]
    /// {
    LBrace,
    #[name("}")]
    #[tag(right)]
    /// }
    RBrace,
    #[name("(")]
    #[tag(left)]
    /// (
    LParen,
    #[name(")")]
    #[tag(right)]
    /// )
    RParen,

    #[name("[:")]
    #[tag(_macro)]
    #[tag(left)]
    LSplicer,
    #[name(":]")]
    #[tag(_macro)]
    #[tag(right)]
    RSplicer,
    #[name("^#")]
    #[tag(_macro)]
    Reflex,
    #[name("$")]
    #[tag(_macro)]
    Dollar,
    #[name("@")]
    At,

    #[name("::")]
    DoubleColon,
    #[name(";")]
    #[tag(logic_binop)]
    SemiColon,
    #[name(":")]
    Colon,
    #[name(",")]
    #[tag(logic_binop)]
    Comma,

    #[name("<identifier>")]
    Ident(String),
    #[tag(keyword)]
    True,
    #[tag(keyword)]
    False,
    #[name("<string>")]
    String(Vec<u8>),
    #[name("<integer>")]
    Int(DukaInt),
    #[name("<float>")]
    Float(DukaFloat),
    #[tag(keyword)]
    Nil,

    /// ## Special mark
    #[name("End of file marker")]
    #[tag(terminator)]
    #[default]
    EOF,
}

impl TokenKind {
    #[inline]
    pub const fn terminator() -> Self {
        Self::EOF
    }

    #[inline]
    pub fn stringify(&self) -> String {
        match self {
            Self::Ident(id) => id.to_owned(),
            Self::Int(i) => i.to_string(),
            Self::Float(i) => i.to_string(),
            Self::String(str) => String::from_utf8_lossy(str).into_owned(),
            _ => self.name().to_owned(),
        }
    }
}
