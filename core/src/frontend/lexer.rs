use std::{
    io::{Bytes, Read},
    mem, vec,
};

use crate::{
    frontend::token::{Token, TokenKind},
    shared::{
        error::{DukaError, DukaLexerError, Position, Span},
        types::DukaLexer,
        utils::{
            Action, MultiPeekable, MultiPeekableExtension, check_identifier, check_utf8_body,
            check_utf8_head, encode_utf8_bytes, get_radix, is_newline, is_valid_ident,
            is_valid_radix, is_valid_unicode, len_utf8_by_head,
        },
        value::{DukaFloat, DukaInt},
    },
};

const DEFAULT_BYTE: u8 = b'\0';
const INIT_CAPACITY_LIMIT: usize = 64;

#[derive(Debug)]
enum ReaderStatus {
    UTF8(u8),
    Default,
}

/// Duka's lexer
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
            .map_or(Ok(TokenKind::EOF), |c| self.do_match(c))
    }

    #[inline]
    fn then_if<F: FnOnce(u8) -> bool>(&mut self, condition: F) -> Result<bool, DukaLexerError> {
        match self.peek_byte()? {
            Some(&b) if condition(b) => {
                self.read_byte()?;
                Ok(true)
            }
            _ => Ok(false),
        }
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
            while self.read_byte()?.is_some_and(is_newline) {}
        }
        Ok(())
    }

    fn do_match(&mut self, ch: u8) -> Result<TokenKind, DukaLexerError> {
        match ch {
            b if b.is_ascii_whitespace() => self.next_kind(),
            b'+' => Ok(TokenKind::Plus),
            b'-' => {
                if self.then(b'-')? {
                    if self.then(b'[')?
                        && let Action::Success(depth) =
                            self.try_count_until_terminator(b'=', b'[')?
                    {
                        self.do_ml_comment(depth)?;
                    } else {
                        self.do_sl_comment()?;
                    }
                    self.next_kind()
                }
                // 不能弄成负数 防止i-10出错
                // else if self.then_if(|b| b.is_ascii_digit())? {
                //     self.do_number(true)
                // }
                else {
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
            b'^' => Ok(TokenKind::Pow),
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
            } else {
                TokenKind::Colon
            }),
            // wtf "l"? typo难绷
            b';' => Ok(TokenKind::SemiColon),
            b'(' => Ok(TokenKind::LParen),
            b')' => Ok(TokenKind::RParen),
            b'[' => {
                if let Action::Success(depth) = self.try_count_until_terminator(b'=', b'[')? {
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
            b'0'..=b'9' => self.do_number(),
            b'\'' => self.do_sl_string(b'\''),
            b'"' => self.do_sl_string(b'"'),
            b if is_valid_ident(b, true) => self.do_ident_or_keyword(),
            // maybe unreachable
            _ => Err(DukaLexerError::UnknownCharacter(ch.to_string())),
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
                                            "invalid code point".to_string(),
                                        ));
                                    } else {
                                        buffer.push(n)
                                    }
                                }
                                Some(b'}') => break,
                                Some(_s) => {
                                    dbg!(_s as char);
                                    return Err(DukaLexerError::InvalidUnicodeEscaped(
                                        "unexpected character in unicode escaped".to_string(),
                                    ));
                                }
                                None => {
                                    return Err(DukaLexerError::UnfinishedString(
                                        (match buffer.len() {
                                            8 => "expected }",
                                            0 => "expected unicode value",
                                            _ => "expected unicode value or }",
                                        })
                                        .to_string(),
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
                            "expected \\u{...}".to_string(),
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
            None => return Err(DukaLexerError::UnexpectedEnd("\\, a, b, f...".to_string())),
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
            } else if b == b'f'
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
                    "an integer shouldn't starts with zero".to_string(),
                ));
            } else if !b.is_ascii_alphabetic() {
                return Ok(TokenKind::Int(0));
            } else {
                // 0a 0b ... unsupported radix
                return Err(DukaLexerError::InvalidInteger(
                    "unsupported radix".to_string(),
                ));
            }
        } else {
            self.buffer.push(self.current_byte);
        }

        loop {
            match self.peek_byte()? {
                Some(b'e' | b'E') if radix == 10 => {
                    float = true;
                    self.buffer.push(b'e');
                    self.read_byte()?;
                }
                Some(b'f') if radix == 10 => {
                    if matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_whitespace()) {
                        float = true;
                        self.read_byte()?;
                        break;
                    } else {
                        return Err(DukaLexerError::InvalidFloat("unknown suffix".to_string()));
                    }
                }
                Some(b'.') if radix == 10 => {
                    if !float && matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_digit()) {
                        float = true;
                        self.buffer.push(b'.');
                        self.read_byte()?;
                    } else {
                        break;
                    }
                }
                Some(b'_') => {
                    self.read_byte()?;
                } // skip _
                Some(&n) if is_valid_radix(n, radix) => {
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
            match self.peek_byte()? {
                Some(&b) if is_valid_ident(b, false) => {
                    self.read_byte()?;
                    self.buffer.push(b);
                }
                _ => break,
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
            "implement" => TokenKind::Implement,
            //"logic" if self.then(b'!')? => TokenKind::Bang,
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
        let byte = self.input.next().transpose();

        match byte {
            Ok(Some(b)) => {
                // utf8的首字节
                if !b.is_ascii()
                    && let ReaderStatus::Default = self.status
                {
                    if !check_utf8_head(b) {
                        return Err(DukaLexerError::InvalidUtf8);
                    }

                    self.status = ReaderStatus::UTF8(len_utf8_by_head(b) - 1);
                    self.current_position.column += 1;
                } else if b == b'\n' {
                    if let ReaderStatus::UTF8(..) = self.status {
                        return Err(DukaLexerError::InvalidUtf8);
                    }

                    // self.status = ReaderStatus::Default;
                    self.current_position.new_line();
                } else {
                    if let ReaderStatus::UTF8(count) = self.status {
                        // 还在一个utf8中
                        if !check_utf8_body(b) {
                            return Err(DukaLexerError::InvalidUtf8);
                        }

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
            Ok(None) => {
                self.current_byte = DEFAULT_BYTE;

                matches!(self.status, ReaderStatus::UTF8(..))
                    .then_some(Err(DukaLexerError::InvalidUtf8))
                    .unwrap_or(Ok(None))
            }
            Err(e) => Err(DukaLexerError::ReaderError(e.to_string())),
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
    fn next(&mut self) -> Result<Token, DukaError> {
        self.next_kind()
            .map(|kind| (kind, self.span()))
            .map_err(|kind| DukaError {
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
