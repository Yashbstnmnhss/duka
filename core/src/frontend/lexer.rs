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
            Action, MultiPeekable, MultiPeekableExtension, check_utf8_body, check_utf8_head,
            encode_utf8_bytes, get_radix, is_newline, is_valid_radix, len_utf8_by_head,
        },
    },
};

const DEFAULT_BYTE: u8 = b'\0';
const START_LINE: usize = 1;
const START_COLUMN: usize = 1;
const INIT_CAPACITY_LIMIT: usize = 64;

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
    nonascii_remaining_count: u8,
    /// 集中复用缓冲 (除escaped外)
    buffer: Vec<u8>,
}

impl<Source: Read> Lexer<Source> {
    pub fn new(source: Source) -> Self {
        Self {
            input: source.bytes().multi_peekable(),
            current_position: Position {
                line: START_LINE,
                column: START_COLUMN,
            },
            start_position: Position {
                line: START_LINE,
                column: START_COLUMN,
            },
            cursor: 0,
            nonascii_remaining_count: 0,
            current_byte: DEFAULT_BYTE,
            buffer: vec![],
        }
    }

    pub fn next_kind(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.start_position = self.current_position.clone();

        // #[cfg(target_family = "unix")]
        if self.current_position.line == START_LINE
            && self.current_position.column == START_COLUMN
            && self.try_skip_shebang()?
        {
            return self.next_kind();
        }

        match self.read_byte()? {
            Some(c) => self.do_match(c),
            None => Ok(TokenKind::EOF),
        }
    }

    fn then_if<F: FnOnce(u8) -> bool>(&mut self, condition: F) -> Result<bool, DukaLexerError> {
        match self.peek_byte()? {
            Some(b) if condition(b) => {
                self.read_byte()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn then(&mut self, target: u8) -> Result<bool, DukaLexerError> {
        self.then_if(|b| b == target)
    }

    fn try_skip_shebang(&mut self) -> Result<bool, DukaLexerError> {
        if let Some(b'#') = self.peek_byte()?
            && let Some(b'!') = self.peek_byte_nth(1)?
        {
            self.read_byte()?;
            self.read_byte()?;
            while let Some(b) = self.read_byte()?
                && !is_newline(b)
            {}
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn do_match(&mut self, ch: u8) -> Result<TokenKind, DukaLexerError> {
        match ch {
            b if b.is_ascii_whitespace() => self.next_kind(),
            b'+' => Ok(TokenKind::Plus),
            b'-' => {
                if self.then(b'-')? {
                    if self.then(b'[')?
                        && let Action::Succeed(depth) =
                            self.try_count_until_terminator(b'=', b'[')?
                    {
                        self.do_ml_comment(depth)?;
                    } else {
                        self.do_sl_comment()?;
                    }
                    self.next_kind()
                } else if self.then_if(|b| b.is_ascii_digit())? {
                    self.do_number(true)
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
            b'l' => Ok(TokenKind::SemiColon),
            b'(' => Ok(TokenKind::LParen),
            b')' => Ok(TokenKind::RParen),
            b'[' => {
                if let Action::Succeed(depth) = self.try_count_until_terminator(b'=', b'[')? {
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
            } else if self.then_if(|b| b.is_ascii_alphabetic())? {
                self.do_attr()?
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
            b'|' => Ok(TokenKind::BitOr),
            b'&' => Ok(TokenKind::BitAnd),
            b'0'..=b'9' => self.do_number(false),
            b'\'' => self.do_sl_string(b'\''),
            b'"' => self.do_sl_string(b'"'),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | 127.. => self.do_ident_or_keyword(),
            // maybe unreachable
            _ => Err(DukaLexerError::UnknownCharacter),
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
                Some(b) if b == target => {
                    count += 1;
                    self.read_byte()?;
                }
                Some(b) if b == terminator => break Ok(Action::Succeed(count)),
                _ => break Ok(Action::Fail(count)),
            }
        }
    }

    fn do_sl_comment(&mut self) -> Result<(), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b'\n') => break Ok(()),
                Some(_) => continue,
                None => break Ok(()),
            }
        }
    }
    fn do_ml_comment(&mut self, depth: usize) -> Result<(), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b']') => {
                    if let Action::Succeed(depth2) = self.try_count_until_terminator(b'=', b']')? {
                        if depth == depth2 {
                            // only when the counts are equal then we will consume the ]
                            // in order to prevent situation like ]==]====]
                            self.read_byte()?;
                            break Ok(());
                        }
                    }
                }
                Some(_) => continue,
                None => break Err(DukaLexerError::UnfinishedComment),
            }
        }
    }

    fn do_sl_string(&mut self, terminator: u8) -> Result<TokenKind, DukaLexerError> {
        // " has already been consumed
        self.init_buffer();

        loop {
            match self.read_byte()? {
                Some(b) => match b {
                    b'\\' => {
                        let mut escaped = self.do_escaped(terminator)?;
                        self.buffer.append(&mut escaped)
                    }
                    _ if is_newline(b) => break Err(DukaLexerError::UnfinishedString),
                    _ if b == terminator => {
                        break Ok(TokenKind::String(
                            String::from_utf8(self.consume_buffer())
                                .map_err(|e| DukaLexerError::ReaderError(e.to_string()))?,
                        ));
                    }
                    _ => self.buffer.push(b),
                },
                None => break Err(DukaLexerError::UnfinishedString),
            }
        }
    }
    fn do_ml_string(&mut self, depth: usize) -> Result<TokenKind, DukaLexerError> {
        self.init_buffer();
        self.then_if(is_newline)?;

        loop {
            match self.read_byte()? {
                Some(b']') => {
                    match self.try_count_until_terminator(b'=', b']')? {
                        Action::Succeed(depth2) if depth == depth2 => {
                            // only when the counts are equal then we will consume the ]
                            // in order to prevent situation like ]==]====]
                            self.read_byte()?;
                            break;
                        }
                        Action::Succeed(depth2) | Action::Fail(depth2) => {
                            self.buffer.push(b']'); // restore it
                            for _ in 0..depth2 {
                                self.buffer.push(b'=')
                            }
                        }
                    }
                }
                Some(b) => self.buffer.push(b),
                None => return Err(DukaLexerError::UnfinishedString),
            }
        }
        Ok(TokenKind::String(
            String::from_utf8(self.consume_buffer())
                .map_err(|e| DukaLexerError::ReaderError(e.to_string()))?,
        ))
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
                                            "Invalid code point".to_string(),
                                        ));
                                    } else {
                                        buffer.push(n)
                                    }
                                }
                                Some(b'}') => break,
                                Some(_) => {
                                    return Err(DukaLexerError::InvalidUnicodeEscaped(
                                        "Unexpected character in unicode escaped".to_string(),
                                    ));
                                }
                                None => return Err(DukaLexerError::UnfinishedString),
                            }
                        }

                        assert!(buffer.len() <= 8);
                        let string =
                            str::from_utf8(&buffer).map_err(|_| DukaLexerError::InvalidEscaped)?;
                        let code = u32::from_str_radix(string, 16)
                            .map_err(|_| DukaLexerError::InvalidEscaped)?;
                        encode_utf8_bytes(code, &mut vec);
                    } else {
                        return Err(DukaLexerError::InvalidEscaped);
                    }
                }

                _ => return Err(DukaLexerError::InvalidEscaped),
            },
            None => return Err(DukaLexerError::UnexpectedEnd),
        }

        Ok(vec)
    }

    fn do_number(&mut self, neg: bool) -> Result<TokenKind, DukaLexerError> {
        self.init_buffer();
        let mut float = false;
        let mut radix = 10;

        if self.current_byte == b'0'
            && let Some(b) = self.peek_byte()?
        {
            if let Some(r) = get_radix(b) {
                self.read_byte()?;
                radix = r;
            } else if b == b'f'
                && matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_whitespace())
            {
                self.read_byte()?;
                return Ok(TokenKind::Float(0f64));
            } else if b == b'e' || b == b'E' {
                // 0e2 0E3 ...
                self.buffer.push(b'0')
                // the 'e' will be processed by following loop
            } else {
                // 0a 0b ... unsupported radix
                return Err(DukaLexerError::UnexpectedCharacter);
            }
        } else {
            self.buffer.push(b'0');
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
                        return Err(DukaLexerError::UnexpectedCharacter);
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
                Some(n) if is_valid_radix(n, radix) => {
                    self.buffer.push(n);
                    self.read_byte()?;
                }
                Some(b) if b.is_ascii_whitespace() => break,
                None => break,
                _ => return Err(DukaLexerError::UnexpectedCharacter),
            }
        }

        let buf = self.consume_buffer();
        let string =
            str::from_utf8(&buf).map_err(|e| DukaLexerError::ReaderError(e.to_string()))?;

        Ok(if float {
            assert_eq!(radix, 10);
            string
                .parse::<f64>()
                .map_err(|e| DukaLexerError::InvalidFloat(e))
                .map(|f| if neg { -f } else { f })
                .map(TokenKind::Float)?
        } else {
            i64::from_str_radix(&string, radix)
                .map_err(|e| DukaLexerError::InvalidInteger(e))
                .map(|i| if neg { -i } else { i })
                .map(TokenKind::Int)?
        })
    }

    fn do_attr(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.init_buffer();
        self.buffer.push(self.current_byte);

        loop {
            match self.peek_byte()? {
                Some(b) if b.is_ascii_alphabetic() => {
                    self.read_byte()?;
                    self.buffer.push(b);
                }
                Some(b'>') => {
                    self.read_byte()?;
                    break;
                }
                _ => return Err(DukaLexerError::UnexpectedCharacter),
            }
        }

        let buf = self.consume_buffer();
        let string =
            String::from_utf8(buf).map_err(|e| DukaLexerError::ReaderError(e.to_string()))?;
        Ok(TokenKind::Attr(string))
    }

    fn do_ident_or_keyword(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.init_buffer();
        self.buffer.push(self.current_byte);

        loop {
            match self.peek_byte()? {
                Some(b) if b.is_ascii_alphanumeric() || b > 127 => {
                    self.read_byte()?;
                    self.buffer.push(b);
                }
                _ => break,
            }
        }

        let buf = self.consume_buffer();
        let string =
            str::from_utf8(&buf).map_err(|e| DukaLexerError::ReaderError(e.to_string()))?;
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
            "local" => TokenKind::Local,
            "function" => TokenKind::Function,
            "return" => TokenKind::Return,
            "end" => TokenKind::End,
            "goto" => TokenKind::Goto,
            _ => TokenKind::Ident(string.to_owned()),
        })
    }

    fn read_byte(&mut self) -> Result<Option<u8>, DukaLexerError> {
        let byte = self.input.next().transpose();

        match byte {
            Ok(Some(b)) => {
                // utf8的首字节
                if !b.is_ascii() && self.nonascii_remaining_count == 0 {
                    if !check_utf8_head(b) {
                        return Err(DukaLexerError::InvalidUtf8);
                    }
                    self.nonascii_remaining_count = len_utf8_by_head(b) - 1;
                    self.current_position.column += 1;
                } else if b == b'\n' {
                    self.nonascii_remaining_count = 0;
                    self.current_position.line += 1;
                    self.current_position.column = START_COLUMN;
                } else {
                    if self.nonascii_remaining_count != 0 {
                        // 还在一个utf8中
                        if !check_utf8_body(b) {
                            return Err(DukaLexerError::InvalidUtf8);
                        }
                        self.nonascii_remaining_count -= 1;
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
                if self.nonascii_remaining_count != 0 {
                    Err(DukaLexerError::InvalidUtf8)
                } else {
                    Ok(None)
                }
            }
            Err(e) => Err(DukaLexerError::ReaderError(e.to_string())),
        }
    }

    fn peek_byte(&mut self) -> Result<Option<u8>, DukaLexerError> {
        self.peek_byte_nth(0)
    }
    /// ## `n` must be less than `MAX_DEPTH`
    fn peek_byte_nth(&mut self, n: usize) -> Result<Option<u8>, DukaLexerError> {
        match self.input.peek_nth(n) {
            Some(Ok(b)) => Ok(Some(*b)),
            Some(Err(e)) => Err(DukaLexerError::ReaderError(e.to_string())),
            None => Ok(None),
        }
    }

    /// call it first when you need to use buffer
    fn init_buffer(&mut self) {
        self.buffer.clear();
    }
    /// this will keep the buffer with capacity of the original one
    /// and return original buffer
    ///
    /// *could that help optimize? or over-designed?*
    fn consume_buffer(&mut self) -> Vec<u8> {
        let new_buffer: Vec<u8> =
            Vec::with_capacity(std::cmp::min(self.buffer.capacity(), INIT_CAPACITY_LIMIT));
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
