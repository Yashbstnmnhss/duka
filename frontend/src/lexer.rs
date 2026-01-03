use std::{
    collections::HashMap,
    io::{Bytes, Read},
    mem, vec,
};

use duka_shared::{
    constants::clex,
    error::{DukaLexerError, DukaMacroError, DukaSpannedError, Position, Span},
    token::{Token, TokenKind},
    types::DukaLexer,
    utils::{
        Action, MultiPeekable, MultiPeekableExtension, OrError, check_identifier, check_utf8_body,
        check_utf8_head, encode_utf8_bytes, get_radix, is_newline, is_valid_ident, is_valid_radix,
        is_valid_unicode, len_utf8_by_head,
    },
    value::{DukaFloat, DukaInt},
};

const DEFAULT_BYTE: u8 = b'\0';
const INIT_CAPACITY_LIMIT: usize = 64;

#[derive(Debug)]
enum ReaderStatus {
    UTF8(u8),
    Default,
}

/// Duka's basic lexer
#[derive(Debug)]
pub struct Lexer<Source>
where
    Source: Read,
{
    input: MultiPeekable<Bytes<Source>>,
    current_byte: u8,
    current_position: Position,
    start_position: Position,
    cursor: usize,
    /// 处理utf8用
    status: ReaderStatus,
    /// 集中复用缓冲 (除escaped外)
    buffer: Vec<u8>,
}

impl<Source: Read> Lexer<Source> {
    pub fn new(source: Source) -> Self {
        Self {
            input: source.bytes().multi_peekable(),
            current_byte: DEFAULT_BYTE,
            current_position: Position::START,
            start_position: Position::START,
            cursor: 0,
            status: ReaderStatus::Default,
            buffer: vec![],
        }
    }

    pub fn next_kind(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.start_position = self.current_position.clone();

        // #[cfg(target_family = "unix")]
        if self.current_position.is_start() {
            self.try_skip_bom()?;
            self.try_skip_shebang()?;
        }

        self.read_byte()?
            .map_or(Ok(TokenKind::terminator()), |c| self.do_match(c))
    }

    #[inline]
    fn then_if<F: FnOnce(u8) -> bool>(&mut self, condition: F) -> Result<bool, DukaLexerError> {
        Ok(match self.peek_byte()? {
            Some(&b) if condition(b) => {
                self.read_byte()?;
                true
            }
            _ => false,
        })
    }

    #[inline(always)]
    fn then(&mut self, target: u8) -> Result<bool, DukaLexerError> {
        self.then_if(|b| b == target)
    }

    #[inline]
    fn try_skip_bom(&mut self) -> Result<(), DukaLexerError> {
        const BOM1: u8 = 0xEF;
        const BOM2: u8 = 0xBB;
        const BOM3: u8 = 0xBF;
        if let Some(&BOM1) = self.peek_byte()?
            && let Some(&BOM2) = self.peek_byte_nth(1)?
            && let Some(&BOM3) = self.peek_byte_nth(2)?
        {
            self.read_byte()?;
            self.read_byte()?;
            self.read_byte()?;
        }
        Ok(())
    }

    #[inline]
    fn try_skip_shebang(&mut self) -> Result<(), DukaLexerError> {
        if let Some(b'#') = self.peek_byte()?
            && let Some(b'!') = self.peek_byte_nth(1)?
        {
            self.read_byte()?;
            self.read_byte()?;

            loop {
                if self.read_byte()?.is_none_or(is_newline) {
                    break;
                }
            }
        }
        Ok(())
    }

    fn do_match(&mut self, ch: u8) -> Result<TokenKind, DukaLexerError> {
        match ch {
            b if b.is_ascii_whitespace() => self.next_kind(),

            b'@' => Ok(TokenKind::At),
            b'$' => Ok(TokenKind::Dollar),
            b'+' => Ok(TokenKind::Plus),
            b'-' => {
                if self.then(b'>')? {
                    Ok(TokenKind::Arrow)
                } else if self.then(b'-')? {
                    if self.then(b'[')?
                        && let Action::Success(depth) =
                            self.try_count_until_terminator(b'=', b'[')?
                    {
                        self.do_ml_comment(depth)?;
                    } else {
                        self.do_sl_comment()?;
                    }
                    self.next_kind()
                } else {
                    Ok(TokenKind::Minus)
                }
            }
            b'*' => Ok(TokenKind::Multiply),
            b'/' => Ok(if self.then(b'/')? {
                TokenKind::IDivide
            } else {
                TokenKind::Divide
            }),
            b'%' => Ok(TokenKind::Mod),
            b'^' => Ok(if self.then(b'#')? {
                TokenKind::Reflex
            } else {
                TokenKind::Pow
            }),
            b'#' => Ok(TokenKind::Length),
            b'.' => Ok(if self.then(b'.')? {
                if self.then(b'.')? {
                    TokenKind::Dots
                } else {
                    TokenKind::Concat
                }
            } else {
                TokenKind::Dot
            }),
            b',' => Ok(TokenKind::Comma),
            b':' => Ok(if self.then(b':')? {
                TokenKind::DoubleColon
            } else if self.then(b']')? {
                TokenKind::RSplicer
            } else {
                TokenKind::Colon
            }),
            // wtf "l"? typo难绷
            b';' => Ok(TokenKind::SemiColon),
            b'(' => Ok(TokenKind::LParen),
            b')' => Ok(TokenKind::RParen),
            b'[' => {
                if self.then(b':')? {
                    Ok(TokenKind::LSplicer)
                } else if let Action::Success(depth) =
                    self.try_count_until_terminator(b'=', b'[')?
                {
                    self.read_byte()?; // remember to consume the [
                    self.do_ml_string(depth)
                } else {
                    Ok(TokenKind::LBracket)
                }
            }

            b']' => Ok(TokenKind::RBracket),
            b'{' => Ok(TokenKind::LBrace),
            b'}' => Ok(TokenKind::RBrace),
            b'<' => Ok(if self.then(b'=')? {
                TokenKind::LessEqual
            } else if self.then(b'<')? {
                TokenKind::ShiftL
            } else if self.then(b'|')? {
                TokenKind::PipelineL
            } else {
                TokenKind::Less
            }),
            b'>' => Ok(if self.then(b'=')? {
                TokenKind::GreaterEqual
            } else if self.then(b'>')? {
                TokenKind::ShiftR
            } else {
                TokenKind::Greater
            }),
            b'=' => Ok(if self.then(b'=')? {
                TokenKind::Equal
            } else {
                TokenKind::Assign
            }),
            b'~' => Ok(if self.then(b'=')? {
                TokenKind::NotEqual
            } else {
                // unary or binary
                TokenKind::BitTilde
            }),
            b'|' => Ok(if self.then(b'>')? {
                TokenKind::Pipeline
            } else {
                TokenKind::BitOr
            }),
            b'&' => Ok(TokenKind::BitAnd),
            b'!' => Ok(TokenKind::Bang),
            b'0'..=b'9' => self.do_number(),
            b'\'' => self.do_sl_string(b'\''),
            b'"' => self.do_sl_string(b'"'),
            b if is_valid_ident(b, true) => self.do_ident_or_keyword(),
            // maybe unreachable
            _ => Err(DukaLexerError::UnknownCharacter((ch as char).to_string())),
        }
    }

    /// this function does **NOT** consume terminator
    fn try_count_until_terminator(
        &mut self,
        target: u8,
        terminator: u8,
    ) -> Result<Action<usize>, DukaLexerError> {
        let mut count: usize = 0;
        loop {
            match self.peek_byte()? {
                Some(b) if *b == target => {
                    count += 1;
                    self.read_byte()?;
                }
                Some(b) if *b == terminator => break Ok(Action::Success(count)),
                _ => break Ok(Action::Failure(count)),
            }
        }
    }

    fn do_sl_comment(&mut self) -> Result<(), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b'\n') | None => break Ok(()),
                Some(_) => continue,
            }
        }
    }
    fn do_ml_comment(&mut self, depth: usize) -> Result<(), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b']') => {
                    if let Action::Success(depth2) = self.try_count_until_terminator(b'=', b']')?
                        && depth == depth2
                    {
                        // only when the counts are equal then we will consume the ]
                        // in order to prevent situation like ]==]====]
                        self.read_byte()?;
                        break Ok(());
                    }
                }
                Some(_) => continue,
                None => {
                    break Err(DukaLexerError::UnfinishedComment(format!(
                        "expected ]{}]",
                        "=".repeat(depth)
                    )));
                }
            }
        }
    }

    fn do_sl_string(&mut self, terminator: u8) -> Result<TokenKind, DukaLexerError> {
        // " has already been consumed
        self.begin_buffer();

        loop {
            match self.read_byte()? {
                Some(b) => match b {
                    b'\\' => {
                        let mut escaped = self.do_escaped(terminator)?;
                        self.buffer.append(&mut escaped)
                    }
                    _ if is_newline(b) => {
                        break Err(DukaLexerError::UnfinishedString(format!(
                            "expected {}",
                            (terminator as char)
                        )));
                    }
                    _ if b == terminator => {
                        break Ok(TokenKind::String(self.end_buffer()));
                    }
                    _ => self.buffer.push(b),
                },
                None => {
                    break Err(DukaLexerError::UnfinishedString(format!(
                        "expected {}",
                        (terminator as char)
                    )));
                }
            }
        }
    }
    fn do_ml_string(&mut self, depth: usize) -> Result<TokenKind, DukaLexerError> {
        self.begin_buffer();
        self.then_if(is_newline)?;

        loop {
            match self.read_byte()? {
                Some(b']') => {
                    match self.try_count_until_terminator(b'=', b']')? {
                        Action::Success(depth2) if depth == depth2 => {
                            // only when the counts are equal then we will consume the ]
                            // in order to prevent situation like ]==]====]
                            self.read_byte()?;
                            break;
                        }
                        Action::Success(depth2) | Action::Failure(depth2) => {
                            self.buffer.push(b']'); // restore it
                            for _ in 0..depth2 {
                                self.buffer.push(b'=')
                            }
                        }
                    }
                }
                Some(b) => self.buffer.push(b),
                None => {
                    return Err(DukaLexerError::UnfinishedString(format!(
                        "expected ]{}]",
                        "=".repeat(depth)
                    )));
                }
            }
        }
        Ok(TokenKind::String(self.end_buffer()))
    }

    fn do_escaped(&mut self, terminator: u8) -> Result<Vec<u8>, DukaLexerError> {
        let mut vec: Vec<u8> = Vec::with_capacity(1);

        match self.read_byte()? {
            Some(b) => match b {
                b if b == terminator => vec.push(terminator),
                b'\\' => vec.push(b'\\'),

                b'a' => vec.push(007),
                b'b' => vec.push(008),
                b'f' => vec.push(012),
                b'n' => vec.push(b'\n'),
                b's' => vec.push(b' '),
                b't' => vec.push(b'\t'),
                b'r' => vec.push(b'\r'),
                b'0' => vec.push(b'\0'),

                b'u' => {
                    if self.then(b'{')? {
                        let mut buffer: Vec<u8> = Vec::with_capacity(8);

                        loop {
                            match self.read_byte()? {
                                Some(n) if is_valid_radix(n, 16) => {
                                    if buffer.len() >= 8 {
                                        return Err(DukaLexerError::InvalidUnicodeEscaped(
                                            "invalid code point".to_owned(),
                                        ));
                                    } else {
                                        buffer.push(n)
                                    }
                                }
                                Some(b'}') => break,
                                Some(_) => {
                                    return Err(DukaLexerError::InvalidUnicodeEscaped(
                                        "unexpected character in unicode escaped".to_owned(),
                                    ));
                                }
                                None => {
                                    return Err(DukaLexerError::UnfinishedString(
                                        (match buffer.len() {
                                            8 => "expected }",
                                            0 => "expected unicode value",
                                            _ => "expected unicode value or }",
                                        })
                                        .to_owned(),
                                    ));
                                }
                            }
                        }

                        assert!(buffer.len() <= 8);
                        let string =
                            str::from_utf8(&buffer).map_err(|_| DukaLexerError::InvalidUtf8)?;
                        let code = u32::from_str_radix(string, 16).map_err(|e| {
                            DukaLexerError::InvalidEscaped(format!("invalid unicode value {}", e))
                        })?;
                        if !is_valid_unicode(code) {
                            return Err(DukaLexerError::InvalidUnicodeEscaped(format!(
                                "{:x} is invalid unicode value",
                                code
                            )));
                        }
                        encode_utf8_bytes(code, &mut vec);
                    } else {
                        return Err(DukaLexerError::InvalidEscaped(
                            "expected \\u{...}".to_owned(),
                        ));
                    }
                }

                _ => {
                    return Err(DukaLexerError::InvalidEscaped(format!(
                        "unknown escaped character {}",
                        (b as char)
                    )));
                }
            },
            None => return Err(DukaLexerError::UnexpectedEnd("\\, a, b, f...".to_owned())),
        }

        Ok(vec)
    }

    fn do_number(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.begin_buffer();
        let mut float = false;
        let mut radix = 10;

        if self.current_byte == b'0'
            && let Some(&b) = self.peek_byte()?
        {
            if let Some(r) = get_radix(b) {
                self.read_byte()?;
                radix = r;
            } else if (b == b'f' || b == b'F')
                && self
                    .peek_byte_nth(1)?
                    .is_some_and(|x| !x.is_ascii_alphanumeric())
            {
                self.read_byte()?;
                return Ok(TokenKind::Float(0f64));
            } else if b == b'e' || b == b'E' || b == b'.' {
                // 0e2 0E3 0.123
                self.buffer.push(b'0')
                // the 'e' or '.' will be processed by following loop
            } else if b.is_ascii_digit() {
                return Err(DukaLexerError::InvalidInteger(
                    "integer cannot start with zero".to_owned(),
                ));
            } else if !b.is_ascii_alphabetic() {
                return Ok(TokenKind::Int(0));
            } else {
                // 0a 0b ... unsupported radix
                return Err(DukaLexerError::InvalidInteger(
                    "unsupported radix".to_owned(),
                ));
            }
        } else {
            self.buffer.push(self.current_byte);
        }

        loop {
            let Some(nb) = self.peek_byte()? else { break };
            match nb {
                b'e' | b'E' if radix == 10 => {
                    float = true;
                    self.buffer.push(b'e');
                    self.read_byte()?;
                }
                b'f' | b'F' if radix == 10 => {
                    if matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_whitespace()) {
                        float = true;
                        self.read_byte()?;
                        break;
                    } else {
                        return Err(DukaLexerError::InvalidFloat("unknown suffix".to_owned()));
                    }
                }
                b'.' if radix == 10 => {
                    if !float && matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_digit()) {
                        float = true;
                        self.buffer.push(b'.');
                        self.read_byte()?;
                    } else {
                        break;
                    }
                }
                b'_' => {
                    self.read_byte()?;
                } // skip _
                &n if is_valid_radix(n, radix) => {
                    self.buffer.push(n);
                    self.read_byte()?;
                }
                // Some(b) if b.is_ascii_whitespace() => break,
                _ => break,
                //_ => return Err(DukaLexerError::UnexpectedCharacter),
            }
        }

        let buf = self.end_buffer();
        let string =
            str::from_utf8(&buf).map_err(|e| DukaLexerError::ReaderError(e.to_string()))?;

        Ok(if float {
            assert_eq!(radix, 10);
            string
                .parse::<DukaFloat>()
                .map_err(|e| DukaLexerError::InvalidFloat(e.to_string()))
                .map(TokenKind::Float)?
        } else {
            DukaInt::from_str_radix(&string, radix)
                .map_err(|e| DukaLexerError::InvalidInteger(e.to_string()))
                .map(TokenKind::Int)?
        })
    }

    fn do_ident_or_keyword(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.begin_buffer();
        self.buffer.push(self.current_byte);

        loop {
            if let Some(&b) = self.peek_byte()?
                && is_valid_ident(b, false)
            {
                self.read_byte()?;
                self.buffer.push(b);
            } else {
                break;
            }
        }

        let buf = self.end_buffer();
        let string = str::from_utf8(&buf).map_err(|_| DukaLexerError::InvalidUtf8)?;
        Ok(match string {
            "do" => TokenKind::Do,
            "then" => TokenKind::Then,
            "nil" => TokenKind::Nil,
            "in" => TokenKind::In,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "elseif" => TokenKind::Elseif,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "and" => TokenKind::And,
            "not" => TokenKind::Not,
            "or" => TokenKind::Or,
            "xor" => TokenKind::Xor,
            "global" => TokenKind::Global,
            "local" => TokenKind::Local,
            "function" => TokenKind::Function,
            "return" => TokenKind::Return,
            "end" => TokenKind::End,
            "goto" => TokenKind::Goto,
            "match" => TokenKind::Match,
            "object" => TokenKind::Object,
            _ => {
                if let Err(c) = check_identifier(string) {
                    return Err(DukaLexerError::UnexpectedCharacter(c));
                } else {
                    TokenKind::Ident(string.to_owned())
                }
            }
        })
    }

    fn read_byte(&mut self) -> Result<Option<u8>, DukaLexerError> {
        let byte = self
            .input
            .next()
            .transpose()
            .map_err(|e| DukaLexerError::ReaderError(e.to_string()))?;

        match byte {
            Some(b) => {
                // utf8的首字节
                if !b.is_ascii()
                    && let ReaderStatus::Default = self.status
                {
                    check_utf8_head(b).or_else_error(|| DukaLexerError::InvalidUtf8)?;

                    self.status = ReaderStatus::UTF8(len_utf8_by_head(b) - 1);
                    self.current_position.column += 1;
                } else if b == b'\n' {
                    matches!(self.status, ReaderStatus::UTF8(..))
                        .then_error(|| DukaLexerError::InvalidUtf8)?;

                    self.current_position.new_line();
                } else {
                    if let ReaderStatus::UTF8(count) = self.status {
                        // 还在一个utf8中
                        check_utf8_body(b).or_else_error(|| DukaLexerError::InvalidUtf8)?;

                        self.status = (count == 1)
                            .then_some(ReaderStatus::Default)
                            .unwrap_or(ReaderStatus::UTF8(count - 1))
                    } else {
                        // 普通ascii
                        self.current_position.column += 1;
                    }
                }

                self.current_byte = b;
                self.cursor += 1;

                Ok(Some(b))
            }
            None => {
                self.current_byte = DEFAULT_BYTE;

                matches!(self.status, ReaderStatus::UTF8(..))
                    .then_some(Err(DukaLexerError::InvalidUtf8))
                    .unwrap_or(Ok(None))
            }
        }
    }

    fn peek_byte(&mut self) -> Result<Option<&u8>, DukaLexerError> {
        self.peek_byte_nth(0)
    }
    /// ## `n` must be less than `MAX_DEPTH`
    fn peek_byte_nth(&mut self, n: usize) -> Result<Option<&u8>, DukaLexerError> {
        self.input
            .peek_nth(n)
            .map(|r| r.as_ref())
            .transpose()
            .map_err(|e| DukaLexerError::ReaderError(e.to_string()))
    }

    /// call it first when buffer is needed
    #[inline(always)]
    fn begin_buffer(&mut self) {
        self.buffer.clear();
    }
    /// this will keep the buffer with capacity of the original one
    /// and return original buffer
    ///
    /// *could that help optimize? or over-designed?*
    #[inline]
    fn end_buffer(&mut self) -> Vec<u8> {
        let new_buffer: Vec<u8> =
            Vec::with_capacity(self.buffer.capacity().min(INIT_CAPACITY_LIMIT));
        mem::replace(&mut self.buffer, new_buffer)
    }
}

impl<Source: Read> DukaLexer<Token> for Lexer<Source> {
    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        self.next_kind()
            .map(|kind| (kind, self.span()))
            .map_err(|kind| DukaSpannedError {
                kind: kind.into(),
                span: self.span(),
            })
    }
    fn span(&self) -> Span {
        Span {
            start: self.start_position.clone(),
            end: self.current_position.clone(),
        }
    }
}

impl<Source: Read> Iterator for Lexer<Source> {
    type Item = Result<Token, DukaSpannedError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.next_token();
        (!matches!(item, Ok((ref tk, _)) if tk.is_terminator())).then_some(item)
    }
}

use crate::macros::*;

#[derive(Debug)]
enum CacheToken {
    Token(Token),
    ExpandEnd,
}

pub const MAX_EXPANDING_DEPTH: u16 = 256;

pub struct LexerWithMacro<Source>
where
    Source: Read,
{
    inner: Lexer<Source>,
    macros: HashMap<MacroName, MacroBody>,
    expanding: Vec<MacroExpanding>,
    cache: Vec<CacheToken>,
}

impl<Source: Read> LexerWithMacro<Source> {
    pub fn new(source: Source) -> Self {
        Self {
            inner: Lexer::new(source),
            macros: HashMap::new(),
            expanding: vec![],
            cache: vec![],
        }
    }

    fn do_macro(&mut self) -> Result<Token, DukaSpannedError> {
        loop {
            let tk = self._next()?;
            match tk.0 {
                TokenKind::Reflex => self.do_reflex()?,
                TokenKind::LSplicer => self.do_splicer()?,
                _ => break Ok(tk),
            }
        }
    }

    fn do_reflex(&mut self) -> Result<(), DukaSpannedError> {
        let keyword = self._must_ident()?;

        match keyword.as_str() {
            "define" => {
                let name = self._must_ident()?;
                let params = if self._then(TokenKind::LParen)? && !self._then(TokenKind::RParen)? {
                    if self._then(TokenKind::Dots)? {
                        self._must(TokenKind::RParen)?;
                        vec![]
                    } else {
                        let first = self._must_ident()?;
                        let mut params = vec![first];

                        while self._then(TokenKind::Comma)? {
                            if self._then(TokenKind::Dots)? {
                                break;
                            }
                            let item = self._must_ident()?;
                            params.push(item);
                        }

                        self._must(TokenKind::RParen)?;
                        params
                    }
                } else {
                    vec![]
                };

                let single = self._then(TokenKind::Arrow)?;
                let body = self.do_macro_body(&params, single)?;
                self.macros.insert(name, (params.len(), body));
            }
            "undef" => {
                let name = self._must_ident()?;
                self.macros.remove(&name);
            }
            _ => return Err(self._expected("define, undef")),
        }
        Ok(())
    }

    fn do_var_arg_sep(&mut self) -> Result<(Token, VarArgSeparatorType), DukaSpannedError> {
        Ok(if self._then(TokenKind::LBracket)? {
            let sep = self._next()?;

            let right = self._then(TokenKind::RBracket)?;

            if !right {
                self._must(TokenKind::RParen)?;
            }

            (
                sep,
                right
                    .then_some(VarArgSeparatorType::All)
                    .unwrap_or(VarArgSeparatorType::Left),
            )
        } else {
            self._must(TokenKind::LParen)?;
            let sep = self._next()?;

            let right = self._then(TokenKind::RBracket)?;

            if !right {
                self._must(TokenKind::RParen)?;
            }

            (
                sep,
                right
                    .then_some(VarArgSeparatorType::Right)
                    .unwrap_or(VarArgSeparatorType::None),
            )
        })
    }

    fn do_macro_body(
        &mut self,
        params: &[String],
        single: bool,
    ) -> Result<Vec<MacroToken>, DukaSpannedError> {
        let mut res = vec![];
        if single {
            let mut depth = 0;
            loop {
                let tk = self._next()?;

                match tk.0 {
                    TokenKind::SemiColon if depth == 0 => break,
                    TokenKind::Dollar => {
                        if self._then(TokenKind::Dots)? {
                            let (sep, ty) = self.do_var_arg_sep()?;
                            res.push(MacroToken::VarArg(sep, ty));
                        } else {
                            let name = self._must_ident()?;
                            res.push(MacroToken::Replace(
                                params.iter().position(|i| *i == name).ok_or_else(|| {
                                    DukaSpannedError {
                                        kind: DukaMacroError::UnknownParameterDefined(name).into(),
                                        span: self.span(),
                                    }
                                })?,
                            ));
                        }
                        continue;
                    }
                    _ => (),
                }

                tk.0.is_terminator().then_error(|| DukaSpannedError {
                    kind: DukaMacroError::InvalidMacroBody.into(),
                    span: tk.1,
                })?;

                tk.0.is_left().then(|| depth += 1);
                tk.0.is_right().then(|| depth -= 1);

                (depth < 0).then_error(|| DukaSpannedError {
                    kind: DukaMacroError::InvalidMacroBody.into(),
                    span: tk.1,
                })?;

                res.push(MacroToken::Token(tk));
            }
        } else {
            loop {
                let tk = self._next()?;

                match tk.0 {
                    TokenKind::Dollar => {
                        if self._then(TokenKind::Dots)? {
                            let (sep, ty) = self.do_var_arg_sep()?;
                            res.push(MacroToken::VarArg(sep, ty));
                        } else if self._then(TokenKind::LSplicer)? {
                            self.do_splicer()?;
                            continue;
                        } else {
                            let name = self._must_ident()?;
                            res.push(MacroToken::Replace(
                                params.iter().position(|i| *i == name).ok_or_else(|| {
                                    DukaSpannedError {
                                        kind: DukaMacroError::UnknownParameterDefined(name).into(),
                                        span: self.span(),
                                    }
                                })?,
                            ));
                        }
                        continue;
                    }
                    TokenKind::Reflex => {
                        (self._must_ident()? == "enifed").or_else_error(|| DukaSpannedError {
                            kind: DukaMacroError::UnexpectedToken("enifed".to_owned()).into(),
                            span: tk.1,
                        })?;
                        break;
                    }
                    _ => (),
                }

                tk.0.is_terminator().then_error(|| DukaSpannedError {
                    kind: DukaMacroError::InvalidMacroBody.into(),
                    span: tk.1,
                })?;

                res.push(MacroToken::Token(tk));
            }
        }
        Ok(res)
    }

    fn collect_param(&mut self) -> Result<Vec<Token>, DukaSpannedError> {
        let mut tks = vec![];
        let mut depth = 0;
        loop {
            let tk = self._next()?;

            match tk.0 {
                TokenKind::LSplicer => {
                    if self._then(TokenKind::BitTilde)? {
                        let span1 = self.span();
                        let raw = self.collect_raw()?;

                        self._must(TokenKind::RSplicer)?;
                        let span2 = self.span();

                        tks.push((TokenKind::LSplicer, span1));
                        tks.extend(raw);
                        tks.push((TokenKind::RSplicer, span2));

                        continue;
                    }

                    self.do_splicer()?;
                }
                TokenKind::Comma | TokenKind::RParen if depth == 0 => {
                    self.cache.push(CacheToken::Token(tk));
                    break;
                }
                ref token => {
                    token.is_terminator().then_error(|| DukaSpannedError {
                        kind: DukaLexerError::UnexpectedEnd("macro parameter".to_owned()).into(),
                        span: tk.1,
                    })?;

                    token.is_left().then(|| depth += 1);
                    token.is_right().then(|| depth -= 1);

                    tks.push(tk);
                }
            }
        }

        Ok(tks)
    }

    fn collect_raw(&mut self) -> Result<Vec<Token>, DukaSpannedError> {
        let mut tks = vec![];
        let mut depth: usize = 0;

        loop {
            let tk = self._next()?;

            match tk.0 {
                TokenKind::RSplicer if depth == 0 => {
                    self.cache.push(CacheToken::Token(tk));
                    break;
                }
                ref token => {
                    token.is_terminator().then_error(|| DukaSpannedError {
                        kind: DukaLexerError::UnexpectedEnd("tokens".to_owned()).into(),
                        span: tk.1,
                    })?;

                    token.is_left().then(|| depth += 1);
                    token.is_right().then(|| depth -= 1);

                    tks.push(tk);
                }
            }
        }

        Ok(tks)
    }

    fn do_splicer(&mut self) -> Result<(), DukaSpannedError> {
        let name = self._must_ident()?;
        let call_site = self.span();
        let builtin = self._then(TokenKind::Bang)?;

        if !builtin
            && self
                .expanding
                .iter()
                .find(|i| &i.0 == &name && i.1 >= MAX_EXPANDING_DEPTH)
                .is_some()
        {
            return Err(DukaSpannedError {
                kind: DukaMacroError::ReachMaxDepth(name).into(),
                span: self.span(),
            });
        }

        if let Some(i) = self.expanding.iter_mut().find(|i| &i.0 == &name) {
            i.1 += 1;
        } else {
            self.expanding.push((name.clone(), 1));
        }

        let params = if self._then(TokenKind::LParen)? && !self._then(TokenKind::RParen)? {
            let mut params = vec![];

            loop {
                let item = self.collect_param()?;

                params.push(item);

                if self._then(TokenKind::RParen)? {
                    break;
                }

                self._then(TokenKind::Comma)?;
            }

            params
        } else {
            vec![]
        };
        self._must(TokenKind::RSplicer)?;

        let res = self.expand_macro(name, params, builtin, call_site)?;
        self.cache.extend(res);

        Ok(())
    }

    fn expand_macro(
        &self,
        name: MacroName,
        params: Vec<MacroParam>,
        builtin: bool,
        call_site: Span,
    ) -> Result<Vec<CacheToken>, DukaSpannedError> {
        let builtins = MACRO_BUILTINS.read().unwrap();
        Ok(
            if builtin && let Some(func) = builtins.get(&&name.as_str()) {
                func(call_site, &self.expanding, params)
                    .into_iter()
                    .map(CacheToken::Token)
                    .rev()
                    .collect()
            } else {
                let Some((params_count, tokens)) = self.macros.get(&name) else {
                    return Err(DukaSpannedError {
                        kind: DukaMacroError::UnknownMacro(name).into(),
                        span: self.span(),
                    });
                };

                let expanded = tokens
                    .into_iter()
                    .flat_map(|tk| match tk {
                        MacroToken::Replace(index) => {
                            params.get(*index).map(|p| p.clone()).unwrap_or_default()
                        }
                        MacroToken::VarArg(seps, ty) => {
                            let input_len = params.len();
                            if input_len < *params_count {
                                return vec![];
                            }
                            let len = input_len - params_count;

                            params[*params_count..].iter().enumerate().fold(
                                vec![],
                                |mut vec: Vec<(TokenKind, Span)>, (i, tks)| {
                                    (i == 0
                                        && matches!(
                                            ty,
                                            VarArgSeparatorType::Left | VarArgSeparatorType::All
                                        ))
                                    .then(|| vec.push(seps.clone()));

                                    vec.extend(tks.clone());

                                    (i < len - 1).then(|| vec.push(seps.clone()));

                                    (i == len - 1
                                        && matches!(
                                            ty,
                                            VarArgSeparatorType::Right | VarArgSeparatorType::All
                                        ))
                                    .then(|| vec.push(seps.clone()));

                                    vec
                                },
                            )
                        }
                        MacroToken::Token(tk) => vec![tk.clone()],
                    })
                    .map(CacheToken::Token)
                    .rev();

                let mut res = vec![];
                res.push(CacheToken::ExpandEnd);
                res.extend(expanded);
                res
            },
        )
    }

    fn _must(&mut self, tk: TokenKind) -> Result<(), DukaSpannedError> {
        let name = tk.name();
        self._then(tk)?.or_else_error(|| self._expected(name))
    }

    fn _then(&mut self, tk: TokenKind) -> Result<bool, DukaSpannedError> {
        let n = self._next()?;

        n.0.is_terminator().then_error(|| DukaSpannedError {
            kind: DukaLexerError::UnexpectedEnd(n.0.name().to_owned()).into(),
            span: n.1,
        })?;

        let res = n.0 == tk;
        if !res {
            self.cache.push(CacheToken::Token(n));
        }
        Ok(res)
    }

    fn _expected(&mut self, expected: &str) -> DukaSpannedError {
        DukaSpannedError {
            kind: DukaMacroError::UnexpectedToken(expected.to_owned()).into(),
            span: self.span(),
        }
    }

    fn _must_ident(&mut self) -> Result<String, DukaSpannedError> {
        let tk = self._next()?;
        if let TokenKind::Ident(id) = tk.0 {
            Ok(id)
        } else {
            Err(self._expected(clex::ID))
        }
    }

    fn _next(&mut self) -> Result<Token, DukaSpannedError> {
        loop {
            match self.cache.pop() {
                Some(CacheToken::ExpandEnd) => {
                    if let Some(last) = self.expanding.last_mut() {
                        if last.1 == 1 {
                            self.expanding.pop();
                        } else {
                            last.1 -= 1;
                        }
                    }
                }
                Some(CacheToken::Token(t)) => break Ok(t),

                None => break self.inner.next_token(),
            }
        }
    }
}

impl<Source: Read> DukaLexer<Token> for LexerWithMacro<Source> {
    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        self.do_macro()
    }
    fn span(&self) -> Span {
        self.inner.span()
    }
}

impl<Source: Read> Iterator for LexerWithMacro<Source> {
    type Item = Result<Token, DukaSpannedError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.next_token();
        (!matches!(item, Ok((ref tk, _)) if tk.is_terminator())).then_some(item)
    }
}
