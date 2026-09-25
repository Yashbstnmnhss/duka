use duka_lib::duka_shared::{
    errors::{Position, Span},
    types::Spanned,
    utils::{MultiPeekable, MultiPeekableExtension, is_valid_ident},
};
use std::num::ParseIntError;

use crate::cdef::{CBaseType, CDecls, CEnum, CFnSig, CStruct, CTag, CType, CUnion, FFIError, Sign};

#[derive(Debug, Clone, thiserror::Error)]
#[error("Error: {kind} at {span}")]
pub struct CDefParserError {
    kind: CDefParserErrorKind,
    span: Span,
}
#[derive(Debug, Clone, thiserror::Error)]
pub enum CDefParserErrorKind {
    #[error("Invalid type: {0}")]
    InvalidType(String),
    #[error("Incomplete input: {0}")]
    Incomplete(String),
    #[error("Unknown input got \"{0}\"")]
    UnknownInput(String),
    #[error("Got invalid number: {0}")]
    InvalidNumber(ParseIntError),
    #[error("Got unexpected token, expected: \"{0}\"")]
    InvalidToken(String),
    #[error("Got EOF, expected: \"{0}\"")]
    UnexpectedEnd(String),
}
impl CDefParserErrorKind {
    pub fn span(self, span: Span) -> CDefParserError {
        CDefParserError { kind: self, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Placeholder,

    Ident(String),
    Number(isize),

    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    Comma,
    SemiColon,
    Colon,
    Assign,

    Pointer,
    VarArg,

    Typedef,
    Struct,
    Enum,
    Union,

    Void,
    Bool,
    Short,
    Int,
    Long,
    Float,
    Double,
    Char,

    Signed,
    Unsigned,

    Const,
    Extern,
    Static,
    Volatile,
}

pub fn parse_ffis(input: &str) -> Result<CDecls, FFIError> {
    let mut parser = Parser::new(tokenize(input)?);
    parser.ffis()?;
    Ok(parser.decls)
}

pub fn tokenize(input: &str) -> Result<Vec<Spanned<Token>>, CDefParserError> {
    let mut res = vec![];
    let mut input = input.chars().multi_peekable();
    let mut pos = Position::default();

    'main: while let Some(ch) = input.next() {
        match ch {
            '\n' => pos.new_line(),
            c if c.is_whitespace() => {
                pos.step();
                continue;
            }

            '$' => res.push((Token::Placeholder, pos + 1)),

            ',' => res.push((Token::Comma, pos + 1)),
            ';' => res.push((Token::SemiColon, pos + 1)),
            ':' => res.push((Token::Colon, pos + 1)),
            '*' => res.push((Token::Pointer, pos + 1)),
            '=' => res.push((Token::Assign, pos + 1)),

            '(' => res.push((Token::LParen, pos + 1)),
            ')' => res.push((Token::RParen, pos + 1)),
            '[' => res.push((Token::LBracket, pos + 1)),
            ']' => res.push((Token::RBracket, pos + 1)),
            '{' => res.push((Token::LBrace, pos + 1)),
            '}' => res.push((Token::RBrace, pos + 1)),

            '/' => match input.next() {
                Some('/') => {
                    pos.step();
                    pos.step();
                    input.by_ref().take_while(|c| *c != '\n').for_each(|_| ());
                    pos.new_line();
                }
                Some('*') => {
                    pos.step();
                    pos.step();
                    while let Some(c) = input.next() {
                        if c == '\n' {
                            pos.new_line();
                        } else if c == '*' {
                            match input.next() {
                                Some('/') => {
                                    pos.step();
                                    continue 'main;
                                }
                                None => break,
                                _ => {
                                    pos.step();
                                    continue;
                                }
                            }
                        } else {
                            pos.step();
                        }
                    }
                    return Err(CDefParserErrorKind::Incomplete(
                        "multiline comment expects \"*/\" for its end".to_string(),
                    )
                    .span(pos + 1));
                }
                _ => return Err(CDefParserErrorKind::UnknownInput("/".to_string()).span(pos + 1)),
            },

            '.' => {
                if input.next().is_some_and(|c| c == '.') && input.next().is_some_and(|c| c == '.')
                {
                    res.push((Token::VarArg, pos + 3));
                    pos.step();
                    pos.step();
                } else {
                    return Err(CDefParserErrorKind::UnknownInput(".".to_string()).span(pos + 1));
                }
            }

            '-' if input.peek_nth(0).is_some_and(char::is_ascii_digit) => {
                let start = pos;
                pos.step();
                let mut buffer = String::new();
                while let Some(ch) = input.peek_nth(0)
                    && ch.is_ascii_digit()
                {
                    buffer.push(*ch);
                    input.next();
                    pos.step();
                }
                let num = buffer.parse::<isize>().map_err(|e| {
                    CDefParserErrorKind::InvalidNumber(e).span(Span { start, end: pos })
                })?;
                res.push((Token::Number(-num), Span { start, end: pos }));
            }
            '0' => {
                let start = pos;
                pos.step();

                res.push((
                    Token::Number(
                        if let Some(&p) = input.peek_nth(0)
                            && matches!(p, 'x' | 'b' | 'o')
                        {
                            input.next();
                            pos.step();

                            let radix = match p {
                                'x' => 16,
                                'b' => 2,
                                'o' => 8,
                                _ => unreachable!(),
                            };

                            let mut buffer = String::new();
                            while let Some(ch) = input.peek_nth(0)
                                && ch.is_digit(radix)
                            {
                                buffer.push(*ch);
                                input.next();
                                pos.step();
                            }
                            isize::from_str_radix(&buffer, radix).map_err(|e| {
                                CDefParserErrorKind::InvalidNumber(e).span(Span { start, end: pos })
                            })?
                        } else {
                            0
                        },
                    ),
                    Span { start, end: pos },
                ))
            }
            n if n.is_ascii_digit() => {
                let start = pos;
                pos.step();
                let mut buffer = String::new();
                buffer.push(n);
                while let Some(ch) = input.peek_nth(0)
                    && ch.is_ascii_digit()
                {
                    buffer.push(*ch);
                    input.next();
                    pos.step();
                }
                let num = buffer.parse::<isize>().map_err(|e| {
                    CDefParserErrorKind::InvalidNumber(e).span(Span { start, end: pos })
                })?;
                res.push((Token::Number(num), Span { start, end: pos }));
            }
            i if is_valid_ident(i as u8, true) => {
                pos.step();
                let start = pos;
                let mut buffer = String::new();
                buffer.push(i);
                while let Some(ch) = input.peek_nth(0) {
                    if is_valid_ident(*ch as u8, false) {
                        buffer.push(*ch);
                        input.next();
                        pos.step();
                        continue;
                    }
                    break;
                }
                res.push((
                    match buffer.as_str() {
                        "typedef" => Token::Typedef,
                        "struct" => Token::Struct,
                        "enum" => Token::Enum,
                        "union" => Token::Union,

                        "unsigned" => Token::Unsigned,
                        "signed" => Token::Signed,

                        "int" => Token::Int,
                        "_Bool" | "bool" => Token::Bool,
                        "float" => Token::Float,
                        "double" => Token::Double,
                        "short" => Token::Short,
                        "long" => Token::Long,
                        "void" => Token::Void,
                        "char" => Token::Char,

                        "const" => Token::Const,
                        "volatile" => Token::Volatile,
                        "static" => Token::Static,
                        "extern" => Token::Extern,

                        _ => Token::Ident(buffer),
                    },
                    Span { start, end: pos },
                ))
            }

            c => return Err(CDefParserErrorKind::UnknownInput(c.to_string()).span(pos + 1)),
        }
    }

    Ok(res)
}

pub struct Parser {
    tokens: MultiPeekable<std::vec::IntoIter<Spanned<Token>>>,
    current: Span,
    pub decls: CDecls,
}
enum DeclOp {
    Pointer,
    Array(Option<usize>),
    Function(Vec<(Option<String>, CType)>, bool),
}
impl Parser {
    pub fn new(tokens: Vec<Spanned<Token>>) -> Self {
        Self {
            current: Span::default(),
            tokens: tokens.into_iter().multi_peekable(),
            decls: CDecls::default(),
        }
    }
    pub fn ffis(&mut self) -> Result<(), FFIError> {
        while self.tokens.peek_nth(0).is_some() {
            if self.then(Token::SemiColon) {
                continue;
            }
            self.ffi()?;
        }
        Ok(())
    }
    pub fn ffi(&mut self) -> Result<(), FFIError> {
        let typedef = self.then(Token::Typedef);

        if !typedef {
            if !self.then(Token::Extern) {
                self.then(Token::Static);
            }
        }

        let base = self
            .declspec()?
            .ok_or(CDefParserErrorKind::InvalidToken("type".to_owned()).span(self.current))?;

        if !typedef && matches!(base, CType::TagRef(..)) && self.then(Token::SemiColon) {
            return Ok(());
        }

        loop {
            let (name, ops) = self.declarator()?;
            let Some(name) = name else {
                return Err(CDefParserErrorKind::InvalidToken("identifier".to_owned())
                    .span(self.current)
                    .into());
            };

            let ty = Self::apply(base.clone(), ops);
            if typedef {
                self.decls.typedefs.push((name, ty));
            } else {
                if let CType::Function(f) = ty {
                    self.decls.declare_function(name, *f)?;
                } else {
                    self.decls.declare_variable(name, ty)?;
                }
            }

            if self.then(Token::Comma) {
                continue;
            }

            break;
        }

        self.must(Token::SemiColon)?;
        Ok(())
    }

    #[inline]
    fn apply(mut base: CType, ops: Vec<DeclOp>) -> CType {
        for ops in ops {
            match ops {
                DeclOp::Pointer => base = CType::Ptr(Box::new(base)),
                DeclOp::Array(len) => base = CType::Array(Box::new(base), len),
                DeclOp::Function(params, var_arg) => {
                    base = CType::Function(Box::new(CFnSig {
                        ret: base,
                        params,
                        var_arg,
                    }))
                }
            }
        }
        base
    }

    fn single_decl(&mut self) -> Result<(Option<String>, CType), FFIError> {
        let base = self
            .declspec()?
            .ok_or(CDefParserErrorKind::InvalidToken("type".to_owned()).span(self.current))?;
        let (name, ops) = self.declarator()?;
        Ok((name, Self::apply(base, ops)))
    }

    fn declarator(&mut self) -> Result<(Option<String>, Vec<DeclOp>), FFIError> {
        let mut prefix = vec![];
        let mut inner = vec![];

        while self.then(Token::Pointer) {
            prefix.push(DeclOp::Pointer);
        }

        let name = if let Some(Token::Ident(_)) = self.peek(0) {
            let Some(Token::Ident(n)) = self.next() else {
                unreachable!()
            };
            Some(n)
        } else if self.then(Token::LParen) {
            let (inner_name, ops2) = self.declarator()?;
            self.must(Token::RParen)?;
            inner = ops2;
            inner_name
        } else {
            None
        };

        let mut postfix = vec![];

        loop {
            if self.then(Token::LBracket) {
                let n = if self.then(Token::RBracket) {
                    None
                } else {
                    let Some(Token::Number(n)) = self.next() else {
                        return Err(CDefParserErrorKind::InvalidToken(
                            "constant unsigned integer".to_owned(),
                        )
                        .span(self.current)
                        .into());
                    };
                    self.must(Token::RBracket)?;

                    if n < 0 {
                        return Err(CDefParserErrorKind::InvalidType(format!(
                            "{n} is less than zero"
                        ))
                        .span(self.current)
                        .into());
                    }
                    Some(n as usize)
                };

                postfix.push(DeclOp::Array(n))
            } else if self.then(Token::LParen) {
                if self.then(Token::RParen) {
                    postfix.push(DeclOp::Function(vec![], false));
                } else {
                    let mut var_arg = false;
                    let mut params = vec![];

                    loop {
                        if self.then(Token::VarArg) {
                            var_arg = true;
                            self.must(Token::RParen)?;
                            break;
                        }

                        let param = self.single_decl()?;
                        if matches!(param, (_, CType::Base(CBaseType::Void))) {
                            self.must(Token::RParen)?;
                            break;
                        }
                        params.push(param);

                        if self.then(Token::RParen) {
                            break;
                        }
                        self.must(Token::Comma)?;
                    }

                    postfix.push(DeclOp::Function(params, var_arg))
                }
            } else {
                break;
            }
        }

        prefix.extend(postfix.into_iter().rev());
        prefix.append(&mut inner);

        Ok((name, prefix))
    }

    pub fn declspec(&mut self) -> Result<Option<CType>, FFIError> {
        while self.then(Token::Const) || self.then(Token::Volatile) {}

        if self.then(Token::Struct) {
            self.ty_struct().map(Some)
        } else if self.then(Token::Enum) {
            self.ty_enum().map(Some)
        } else if self.then(Token::Union) {
            self.ty_union().map(Some)
        } else {
            self.ty_simple()
        }
    }

    fn try_name(&mut self) -> Option<String> {
        if let Some(Token::Ident(_)) = self.peek(0) {
            let Some(Token::Ident(name)) = self.next() else {
                unreachable!()
            };
            Some(name)
        } else {
            None
        }
    }

    fn declarators(&mut self) -> Result<Vec<(String, Vec<DeclOp>)>, FFIError> {
        let mut nos = vec![];
        loop {
            let (name, ops) = self.declarator()?;

            let Some(name) = name else {
                return Err(CDefParserErrorKind::InvalidToken("identifier".to_owned())
                    .span(self.current)
                    .into());
            };
            nos.push((name, ops));

            if self.then(Token::Comma) {
                continue;
            } else {
                break;
            }
        }
        Ok(nos)
    }
    // `enum` is consumed
    fn ty_enum(&mut self) -> Result<CType, FFIError> {
        let name = self.try_name();

        let items = if self.then(Token::LBrace) {
            if let Some(name) = &name {
                self.get_or_declare(name.clone())?;
            }

            if self.then(Token::RBrace) {
                Some(vec![])
            } else {
                let mut items = vec![];
                loop {
                    let Some(Token::Ident(item)) = self.next() else {
                        return Err(CDefParserErrorKind::InvalidToken("enum item".to_owned())
                            .span(self.current)
                            .into());
                    };

                    let val = if self.then(Token::Assign) {
                        let Some(Token::Number(n)) = self.next() else {
                            return Err(CDefParserErrorKind::InvalidToken(
                                "constant integer".to_owned(),
                            )
                            .span(self.current)
                            .into());
                        };
                        Some(n)
                    } else {
                        None
                    };

                    items.push((item, val));

                    if self.then(Token::RBrace) {
                        break;
                    }

                    self.must(Token::Comma)?;
                }
                Some(items)
            }
        } else {
            None
        };

        match (name, items) {
            (Some(name), None) => Ok(CType::TagRef(self.get_or_declare(name)?)),

            (Some(name), Some(items)) => Ok(if let Some(idx) = self.decls.tag_mapper.get(&name) {
                self.decls.tags[*idx] = CTag::Enum(CEnum(Some(name), items));
                CType::TagRef(*idx)
            } else {
                CType::TagRef(self.declare_tag(name.clone(), CTag::Enum(CEnum(Some(name), items)))?)
            }),
            (None, Some(ps)) => Ok(CType::Enum(CEnum(None, ps))),
            (None, None) => Err(CDefParserErrorKind::InvalidType(
                "neither name nor body for enum".to_owned(),
            )
            .span(self.current)
            .into()),
        }
    }
    // `union` is consumed
    fn ty_union(&mut self) -> Result<CType, FFIError> {
        let name = self.try_name();

        let ps = if self.then(Token::LBrace) {
            if let Some(name) = &name {
                self.get_or_declare(name.clone())?;
            }

            if self.then(Token::RBrace) {
                Some(vec![])
            } else {
                let mut items = vec![];
                loop {
                    let base = self.declspec()?.ok_or(
                        CDefParserErrorKind::InvalidToken("type".to_owned()).span(self.current),
                    )?;

                    for no in self.declarators()? {
                        items.push((no.0, Self::apply(base.clone(), no.1)));
                    }

                    self.must(Token::SemiColon)?;

                    if self.then(Token::RBrace) {
                        break;
                    }
                }
                Some(items)
            }
        } else {
            None
        };

        match (name, ps) {
            (Some(name), None) => Ok(CType::TagRef(self.get_or_declare(name)?)),
            (Some(name), Some(ps)) => Ok(if let Some(idx) = self.decls.tag_mapper.get(&name) {
                self.decls.tags[*idx] = CTag::Union(CUnion(Some(name), ps));
                CType::TagRef(*idx)
            } else {
                CType::TagRef(self.declare_tag(name.clone(), CTag::Union(CUnion(Some(name), ps)))?)
            }),
            (None, Some(ps)) => Ok(CType::Union(CUnion(None, ps))),
            (None, None) => Err(CDefParserErrorKind::InvalidType(
                "neither name nor body for union".to_owned(),
            )
            .span(self.current)
            .into()),
        }
    }

    #[inline(always)]
    fn get_or_declare(&mut self, name: String) -> Result<usize, FFIError> {
        self.decls
            .tag_mapper
            .get(&name)
            .map(|i| Ok(*i))
            .unwrap_or_else(|| self.declare_placeholder(name))
    }
    #[inline(always)]
    fn declare_placeholder(&mut self, name: String) -> Result<usize, FFIError> {
        self.declare_tag(name, CTag::Struct(CStruct(None, vec![])))
    }
    #[inline(always)]
    fn declare_tag(&mut self, name: String, tag: CTag) -> Result<usize, FFIError> {
        self.decls.declare_tag(name, tag)
    }

    // `struct` is consumed
    fn ty_struct(&mut self) -> Result<CType, FFIError> {
        let name = self.try_name();

        let ps = if self.then(Token::LBrace) {
            if let Some(name) = &name {
                self.get_or_declare(name.clone())?;
            }

            if self.then(Token::RBrace) {
                Some(vec![])
            } else {
                let mut properties = vec![];
                loop {
                    let base = self.declspec()?.ok_or(
                        CDefParserErrorKind::InvalidToken("type".to_owned()).span(self.current),
                    )?;

                    for no in self.declarators()? {
                        properties.push((no.0, Self::apply(base.clone(), no.1)));
                    }

                    self.must(Token::SemiColon)?;
                    if self.then(Token::RBrace) {
                        break;
                    }
                }
                Some(properties)
            }
        } else {
            None
        };

        match (name, ps) {
            (Some(name), None) => Ok(CType::TagRef(self.get_or_declare(name)?)),
            (Some(name), Some(ps)) => Ok(if let Some(idx) = self.decls.tag_mapper.get(&name) {
                self.decls.tags[*idx] = CTag::Struct(CStruct(Some(name), ps));
                CType::TagRef(*idx)
            } else {
                CType::TagRef(
                    self.declare_tag(name.clone(), CTag::Struct(CStruct(Some(name), ps)))?,
                )
            }),
            (None, Some(ps)) => Ok(CType::Struct(CStruct(None, ps))),
            (None, None) => Err(CDefParserErrorKind::InvalidType(
                "neither name nor body for struct".to_owned(),
            )
            .span(self.current)
            .into()),
        }
    }

    fn ty_simple(&mut self) -> Result<Option<CType>, FFIError> {
        let start = self.current;
        let mut sign = Sign::Default;
        let mut short = false;
        let mut base = None;
        let mut long_count = 0;

        while let Some(tk) = self.peek(0) {
            match tk {
                Token::Signed => {
                    if sign != Sign::Default {
                        self.next();
                        return Err(CDefParserErrorKind::InvalidType(
                            "duplicated \"signed\" or \"unsigned\"".to_owned(),
                        )
                        .span(start + self.current)
                        .into());
                    }
                    sign = Sign::Signed
                }
                Token::Unsigned => {
                    if sign != Sign::Default {
                        self.next();
                        return Err(CDefParserErrorKind::InvalidType(
                            "duplicated \"signed\" or \"unsigned\"".to_owned(),
                        )
                        .span(start + self.current)
                        .into());
                    }
                    sign = Sign::Unsigned
                }

                Token::Const | Token::Volatile => {} // ignored

                Token::Int
                | Token::Bool
                | Token::Char
                | Token::Double
                | Token::Float
                | Token::Void => {
                    if base.is_some() {
                        self.next();
                        return Err(CDefParserErrorKind::InvalidType(
                            "duplicated base types".to_owned(),
                        )
                        .span(start + self.current)
                        .into());
                    }
                    base = self.next();
                    continue;
                }

                Token::Short => {
                    if short {
                        self.next();
                        return Err(CDefParserErrorKind::InvalidType(
                            "duplicated \"short\"".to_owned(),
                        )
                        .span(start + self.current)
                        .into());
                    }
                    short = true
                }
                Token::Long => long_count += 1,

                Token::Ident(_)
                    if base.is_none() && !short && long_count == 0 && sign == Sign::Default =>
                {
                    let Some(Token::Ident(name)) = self.next() else {
                        unreachable!()
                    };
                    return Ok(Some(CType::TypeRef(
                        self.decls
                            .typedefs
                            .iter()
                            .position(|t| &t.0 == &name)
                            .ok_or(FFIError::UnknownType(name))?,
                    )));
                }
                _ => break,
            }
            self.next();
        }
        let span = start + self.current;

        Ok(match (base, short, long_count, sign) {
            (_, true, l, _) if l > 0 => {
                return Err(CDefParserErrorKind::InvalidType(
                    "both \"short\" and \"long\" in specifiers".to_owned(),
                )
                .span(span)
                .into());
            }

            (None, false, 0, Sign::Default) => None,
            (None, true, 0, sig) => Some(CType::Base(CBaseType::Short(sig))),
            (None, false, l, sig) if l > 0 => Some(CType::Base(match l {
                1 => CBaseType::Long(sig),
                2 => CBaseType::LongLong(sig),
                _ => {
                    return Err(CDefParserErrorKind::InvalidType(
                        "too long for duka ffi!".to_owned(),
                    )
                    .span(span)
                    .into());
                }
            })),

            (base @ None | base @ Some(Token::Int), false, 0, sig)
                if sig != Sign::Default || base.is_some() =>
            {
                Some(CType::Base(CBaseType::Int(sig)))
            }

            (Some(Token::Double), false, l, Sign::Default) if l > 0 => Some(CType::Base(match l {
                1 => CBaseType::LongDouble,
                _ => {
                    return Err(CDefParserErrorKind::InvalidType(
                        "too long for double in duka ffi!".to_owned(),
                    )
                    .span(span)
                    .into());
                }
            })),
            (Some(Token::Void | Token::Float | Token::Double | Token::Bool), s, l, sig)
                if s || l > 0 || sig != Sign::Default =>
            {
                return Err(CDefParserErrorKind::InvalidType(
                    "bad declaration specifier(s)".to_owned(),
                )
                .span(span)
                .into());
            }
            (Some(Token::Int), true, 0, sig) => Some(CType::Base(CBaseType::Short(sig))),
            (Some(Token::Int), false, l, sig) if l > 0 => Some(CType::Base(match l {
                1 => CBaseType::Long(sig),
                2 => CBaseType::LongLong(sig),
                _ => {
                    return Err(CDefParserErrorKind::InvalidType(
                        "too long for duka ffi!".to_owned(),
                    )
                    .span(span)
                    .into());
                }
            })),
            (Some(tk), false, 0, sig) => Some(CType::Base(match tk {
                Token::Void => CBaseType::Void,
                Token::Double => CBaseType::Double,
                Token::Float => CBaseType::Float,
                Token::Bool => CBaseType::Bool,
                Token::Int => CBaseType::Int(sig),
                Token::Char => CBaseType::Char(sig),
                _ => unreachable!(),
            })),

            _ => {
                return Err(CDefParserErrorKind::InvalidType(
                    "bad declaration specifier(s)".to_owned(),
                )
                .span(span)
                .into());
            }
        })
    }

    fn must(&mut self, who: Token) -> Result<(), CDefParserError> {
        if let Some(tk) = self.next() {
            if tk == who {
                Ok(())
            } else {
                Err(CDefParserErrorKind::InvalidToken(format!("{who:?}")).span(self.current))
            }
        } else {
            Err(CDefParserErrorKind::UnexpectedEnd(format!("{who:?}")).span(self.current))
        }
    }
    fn then(&mut self, who: Token) -> bool {
        if let Some(tk) = self.peek(0) {
            if tk == &who {
                self.next();
                return true;
            }
        }
        false
    }
    fn peek(&mut self, nth: usize) -> Option<&Token> {
        let (tk, sp) = self.tokens.peek_nth(nth)?;
        self.current = *sp;
        Some(tk)
    }
    fn next(&mut self) -> Option<Token> {
        let (tk, sp) = self.tokens.next()?;
        self.current = sp;
        Some(tk)
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::{Parser, tokenize};

    #[test]
    fn test_parser() {
        let mut parser = Parser::new(
            tokenize(
                r#"
typedef int (*Handler)(int, ...);
typedef Handler (*Middleware)(Handler);

struct Config {
    Handler handlers[8];
    Middleware mw;
    int (*matrix[2][3])(void);
    int plain_3d[2][3][4];
};

enum Color { RED = 1, GREEN, BLUE = 10, RESET };

union Value {
    int i;
    double d;
    struct Config cfg;
};

typedef struct Config Config;
typedef Config *ConfigP;

extern Config global_config;
extern union Value current_value;
extern int plain_array[2][3];

Handler get_handler(int idx);
Middleware get_middleware(void);
ConfigP alloc_config(void);
"#,
            )
            .unwrap(),
        );
        parser.ffis().expect("");
        println!("{:?}", parser.decls)
    }
}
