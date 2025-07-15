use crate::shared::types::Spanned;

pub type Token = Spanned<TokenKind>;

#[derive(Debug, PartialEq, Clone)]
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

    Assign,
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,

    BitAnd,
    BitOr,
    BitTilde,
    ShiftL,
    ShiftR,

    And,
    Or,
    Not,
    Xor,

    Plus,
    Minus,
    Multiply,
    Divide,
    IDivide,
    Mod,
    Pow,
    Concat,
    Dot,
    Dots,
    Length,

    LBracket,
    RBracket,
    LBrace,
    RBrace,
    LParen,
    RParen,

    DoubleColon,
    SemiColon,
    Colon,
    Comma,

    // <attr>
    Attr(String),
    Ident(String),
    True,
    False,
    String(String),
    Int(i64),
    Float(f64),
    Nil,

    EOF,
}

impl TokenKind {
    pub fn eof(&self) -> bool {
        match self {
            TokenKind::EOF => true,
            _ => false,
        }
    }
}
