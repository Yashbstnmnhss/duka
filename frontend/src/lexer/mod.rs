use std::{
    collections::HashMap,
    io::{Bytes, Read},
    mem,
    sync::Arc,
    time::Instant,
    vec,
};
use std::io::BufReader;

pub mod macros;
pub mod token;

use duka_shared::{
    constants::{MAX_EXPANDING_DEPTH, clex},
    errors::{DukaLexerError, DukaMacroError, DukaSpannedError, Position, Span},
    types::{Complete, DukaLexer, DukaResult, DukaResumable, Incomplete, SourceInfo, TokenStream},
    utils::{
        Action, MultiPeekable, MultiPeekableExtension, OrError, check_identifier, check_utf8_body,
        check_utf8_head, encode_utf8_bytes, get_radix, is_newline, is_valid_ident, is_valid_radix,
        is_valid_unicode, len_utf8_by_head,
    },
    value::{DukaFloat, DukaInt},
};

const DEFAULT_BYTE: u8 = b'\0';
const INIT_CAPACITY_LIMIT: usize = 64;

#[derive(Debug, Clone)]
pub enum ReaderStatus {
    UTF8(u8),
    Default,
}

#[derive(Debug, Default, Clone)]
pub enum LexerMode {
    #[default]
    Normal,

    String(u8),
    StringEnd(usize, bool),
    MLString(usize),

    CommentEnd(usize, bool),
    Comment,
    MLComment(usize),

    ID,
    Number,
    Symbol(TokenKind),
}

#[derive(Debug, Clone)]
pub struct LexerState {
    current_byte: u8,
    current_position: Position,
    start_position: Position,
    cursor: usize,
    /// 处理utf8用
    status: ReaderStatus,
    /// 集中复用缓冲 (除escaped外)
    buffer: Vec<u8>,
    source: Vec<u8>,
    mode: LexerMode,
    source_name: Option<Arc<str>>,
    time: Instant,
}

/// Duka's basic lexer
#[derive(Debug)]
pub struct Lexer<Source>
where
    Source: Read,
{
    input: MultiPeekable<Bytes<BufReader<Source>>>,
    state: LexerState,
}

enum Command {
    Switch(LexerMode),
}

impl<Source: Read> Lexer<Source> {
    pub fn new(source: Source, name: Option<String>) -> Self {
        Self {
            input: BufReader::new(source).bytes().multi_peekable(),
            state: LexerState {
                current_byte: DEFAULT_BYTE,
                current_position: Position::START,
                start_position: Position::START,
                cursor: 0,
                status: ReaderStatus::Default,
                buffer: vec![],
                source: vec![],
                mode: LexerMode::default(),
                source_name: name.map(|s| s.into()),
                time: Instant::now(),
            },
        }
    }

    pub fn resume(&mut self, source: Source) {
        self.input = BufReader::new(source).bytes().multi_peekable();
    }

    pub(crate) fn next_kind(&mut self) -> DukaResult<TokenKind, (), DukaLexerError> {
        self.state.start_position = self.state.current_position;

        if self.state.current_position.is_start() {
            self.try_skip_bom()?;
            self.try_skip_shebang()?;
        }

        loop {
            match mem::take(&mut self.state.mode) {
                LexerMode::Normal => {
                    let Some(ch) = self.read_byte()? else {
                        self.state.start_position = self.state.current_position;
                        break Ok(DukaResumable::Complete(TokenKind::terminator()));
                    };
                    break match ch {
                        b if b.is_ascii_whitespace() => self.next_kind(),

                        b'@' => Complete(TokenKind::At),
                        b'$' => Complete(TokenKind::Dollar),
                        b'+' => Complete(TokenKind::Plus),
                        b'-' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Minus);
                            continue;
                        }
                        b'*' => Complete(TokenKind::Multiply),
                        b'/' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Divide);
                            continue;
                        }
                        b'%' => Complete(TokenKind::Mod),
                        b'^' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Pow);
                            continue;
                        }
                        b'#' => Complete(TokenKind::Length),
                        b'.' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Dot);
                            continue;
                        }
                        b',' => Complete(TokenKind::Comma),
                        b':' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Colon);
                            continue;
                        }
                        // wtf "l"? typo难绷
                        b';' => Complete(TokenKind::SemiColon),
                        b'(' => Complete(TokenKind::LParen),
                        b')' => Complete(TokenKind::RParen),
                        b'[' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::LBracket);
                            continue;
                        }

                        b']' => Complete(TokenKind::RBracket),
                        b'{' => Complete(TokenKind::LBrace),
                        b'}' => Complete(TokenKind::RBrace),
                        b'<' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Less);
                            continue;
                        }
                        b'>' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Greater);
                            continue;
                        }
                        b'=' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Assign);
                            continue;
                        }
                        b'~' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::BitTilde);
                            continue;
                        }
                        b'|' => {
                            self.state.mode = LexerMode::Symbol(TokenKind::BitOr);
                            continue;
                        }
                        b'&' => Complete(TokenKind::BitAnd),
                        b'!' => Complete(TokenKind::Bang),
                        b'0'..=b'9' => {
                            self.state.mode = LexerMode::Number;
                            continue;
                        }
                        b'\'' | b'"' => {
                            self.state.mode = LexerMode::String(ch);
                            continue;
                        }
                        b if is_valid_ident(b, true) => {
                            self.state.mode = LexerMode::ID;
                            continue;
                        }
                        // maybe unreachable
                        _ => Err(DukaLexerError::UnknownCharacter(
                            (ch as char).to_string().into_boxed_str(),
                        )),
                    };
                }
                LexerMode::String(t) => break self.do_sl_string(t),
                LexerMode::MLString(depth) => {
                    match self.do_ml_string(depth)? {
                        DukaResumable::Complete(Command::Switch(to)) => self.state.mode = to,
                        DukaResumable::Incomplete(state, si, expected, span) => {
                            break Incomplete(state, si, expected, span);
                        }
                    }
                    continue;
                }
                LexerMode::StringEnd(counted, is_head) => {
                    if is_head {
                        match self
                            .try_count_until_terminator(b'=', b'[')?
                            .map_to_result(|_, _, m, _| DukaLexerError::UnfinishedString(m))?
                        {
                            Action::Success(depth) => {
                                self.read_byte()?; // consume '['
                                self.then_if(is_newline)?;
                                self.state.mode = LexerMode::MLString(depth + counted);
                                continue;
                            }
                            Action::Failure(depth) => {
                                break Err(DukaLexerError::UnfinishedString(
                                    format!("[{}[", "=".repeat(depth + counted)).into_boxed_str(),
                                ));
                            }
                        }
                    } else {
                        match self.try_count_until_terminator(b'=', b']')? {
                            DukaResumable::Complete(Action::Success(depth)) if depth == counted => {
                                self.read_byte()?;
                                break Complete(TokenKind::String(self.take_buffer().into()));
                            }
                            DukaResumable::Incomplete(depth, ..)
                            | DukaResumable::Complete(
                                Action::Success(depth) | Action::Failure(depth),
                            ) => {
                                self.state.buffer.push(b']'); // restore it
                                for _ in 0..depth {
                                    self.state.buffer.push(b'=')
                                }
                                self.state.mode = LexerMode::MLString(counted);
                                continue;
                            }
                        }
                    }
                }
                LexerMode::CommentEnd(counted, is_head) => {
                    self.state.mode = if is_head {
                        let Some(ch) = self.read_byte()? else {
                            if is_head {
                                self.state.start_position = self.state.current_position;
                                break Complete(TokenKind::terminator());
                            } else {
                                return Err(DukaLexerError::UnfinishedComment(
                                    format!("]{}]", "=".repeat(counted)).into_boxed_str(),
                                ));
                            }
                        };

                        if ch == b'[' {
                            match self.try_count_until_terminator(b'=', b'[')? {
                                DukaResumable::Complete(Action::Success(depth)) => {
                                    self.read_byte()?; // consumed '['
                                    LexerMode::MLComment(depth + counted)
                                }
                                DukaResumable::Complete(Action::Failure(..))
                                | DukaResumable::Incomplete(..) => LexerMode::Comment,
                            }
                        } else {
                            LexerMode::Comment
                        }
                    } else {
                        match self.try_count_until_terminator(b'=', b']')? {
                            DukaResumable::Complete(Action::Success(depth)) if depth == counted => {
                                self.read_byte()?;
                                continue;
                            }
                            DukaResumable::Complete(_) | DukaResumable::Incomplete(..) => {
                                LexerMode::MLComment(counted)
                            }
                        }
                    };
                    continue;
                }
                LexerMode::Comment => {
                    self.do_sl_comment()?;
                    continue;
                }
                LexerMode::MLComment(depth) => {
                    match self.do_ml_comment(depth)? {
                        DukaResumable::Complete(Command::Switch(to)) => self.state.mode = to,
                        DukaResumable::Incomplete(state, si, expected, span) => {
                            break Incomplete(state, si, expected, span);
                        }
                    }
                    continue;
                }
                LexerMode::ID => break Complete(self.do_ident_or_keyword()?),
                LexerMode::Number => break self.do_number(),
                LexerMode::Symbol(tk) => {
                    let Some(ch) = self.peek_byte()? else {
                        break Ok(DukaResumable::Complete(tk));
                    };
                    let res = Complete(match (&tk, ch) {
                        (TokenKind::Minus, b'>') => TokenKind::Arrow,
                        (TokenKind::Minus, b'-') => {
                            self.state.mode = LexerMode::CommentEnd(0, true);
                            self.read_byte()?;
                            continue;
                        }

                        (TokenKind::Divide, b'/') => TokenKind::IDivide,

                        (TokenKind::Pow, b'#') => TokenKind::Reflex,

                        (TokenKind::Dot, b'.') => {
                            self.state.mode = LexerMode::Symbol(TokenKind::Concat);
                            self.read_byte()?;
                            continue;
                        }
                        (TokenKind::Concat, b'.') => TokenKind::Dots,

                        (TokenKind::Colon, b':') => TokenKind::DoubleColon,
                        (TokenKind::Colon, b']') => TokenKind::RSplicer,

                        (TokenKind::LBracket, b':') => TokenKind::LSplicer,
                        (TokenKind::LBracket, b'[') => {
                            self.state.mode = LexerMode::MLString(0);
                            self.read_byte()?;
                            continue;
                        }
                        (TokenKind::LBracket, b'=') => {
                            self.state.mode = LexerMode::StringEnd(1, true);
                            self.read_byte()?;
                            continue;
                        }

                        (TokenKind::Less, b'=') => TokenKind::LessEqual,
                        (TokenKind::Less, b'<') => TokenKind::ShiftL,
                        (TokenKind::Less, b'|') => TokenKind::PipelineL,

                        (TokenKind::Greater, b'=') => TokenKind::GreaterEqual,
                        (TokenKind::Greater, b'>') => TokenKind::ShiftR,

                        (TokenKind::Assign, b'=') => TokenKind::Equal,

                        (TokenKind::BitTilde, b'=') => TokenKind::NotEqual,

                        (TokenKind::BitOr, b'>') => TokenKind::Pipeline,

                        _ => break Complete(tk),
                    });
                    self.read_byte()?;
                    break res;
                }
            }
        }
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

    /// this function does **NOT** consume terminator
    fn try_count_until_terminator(
        &mut self,
        target: u8,
        terminator: u8,
    ) -> Result<DukaResumable<Action<usize>, usize>, DukaLexerError> {
        let mut count: usize = 0;
        Complete(loop {
            match self.peek_byte()? {
                Some(b) if *b == target => {
                    count += 1;
                    self.read_byte()?;
                }
                Some(b) if *b == terminator => break Action::Success(count),
                Some(_) => break Action::Failure(count),
                None => {
                    return Incomplete(
                        count,
                        self.source_info(),
                        (terminator as char).to_string().into_boxed_str(),
                        self.span(),
                    );
                }
            }
        })
    }

    fn do_sl_comment(&mut self) -> Result<(), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b'\n') | None => break Ok(()),
                Some(_) => continue,
            }
        }
    }

    fn do_ml_comment(&mut self, depth: usize) -> DukaResult<Command, (), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b']') => {
                    break Complete(Command::Switch(LexerMode::CommentEnd(depth, false)));
                }
                Some(_) => continue,
                None => {
                    self.state.mode = LexerMode::MLComment(depth);
                    return Incomplete(
                        (),
                        self.source_info(),
                        format!("]{}]", "=".repeat(depth)).into_boxed_str(),
                        self.span(),
                    );
                }
            }
        }
    }

    fn do_sl_string(&mut self, terminator: u8) -> DukaResult<TokenKind, (), DukaLexerError> {
        // " has already been consumed
        loop {
            match self.read_byte()? {
                Some(b) => match b {
                    b'\\' => {
                        let mut escaped = self.do_escaped(terminator)?;
                        self.state.buffer.append(&mut escaped)
                    }
                    _ if is_newline(b) => {
                        break Err(DukaLexerError::UnfinishedString(
                            format!("expected {}", terminator as char).into_boxed_str(),
                        ));
                    }
                    _ if b == terminator => {
                        break Complete(TokenKind::String(self.take_buffer().into()));
                    }
                    _ => self.state.buffer.push(b),
                },
                None => {
                    break Incomplete(
                        (),
                        self.source_info(),
                        format!("expected {}", terminator as char).into_boxed_str(),
                        self.span(),
                    );
                }
            }
        }
    }

    fn do_ml_string(&mut self, depth: usize) -> DukaResult<Command, (), DukaLexerError> {
        loop {
            match self.read_byte()? {
                Some(b']') => break Complete(Command::Switch(LexerMode::StringEnd(depth, false))),
                Some(b) => self.state.buffer.push(b),
                None => {
                    self.state.mode = LexerMode::MLString(depth);
                    break Incomplete(
                        (),
                        self.source_info(),
                        format!("]{}]", "=".repeat(depth)).into_boxed_str(),
                        self.span(),
                    );
                }
            }
        }
    }

    fn do_escaped(&mut self, terminator: u8) -> Result<Vec<u8>, DukaLexerError> {
        let mut vec: Vec<u8> = Vec::with_capacity(1);

        match self.read_byte()? {
            Some(b) => match b {
                b if b == terminator => vec.push(terminator),
                b'\\' => vec.push(b'\\'),

                b'a' => vec.push(7),
                b'b' => vec.push(8),
                b'f' => vec.push(12),
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
                                            "invalid code point".into(),
                                        ));
                                    } else {
                                        buffer.push(n)
                                    }
                                }
                                Some(b'}') => break,
                                Some(_) => {
                                    return Err(DukaLexerError::InvalidUnicodeEscaped(
                                        "unexpected character in unicode escaped".into(),
                                    ));
                                }
                                None => {
                                    return Err(DukaLexerError::UnfinishedString(
                                        (match buffer.len() {
                                            8 => "expected }",
                                            0 => "expected unicode value",
                                            _ => "expected unicode value or }",
                                        })
                                        .into(),
                                    ));
                                }
                            }
                        }

                        assert!(buffer.len() <= 8);
                        let string =
                            str::from_utf8(&buffer).map_err(|_| DukaLexerError::InvalidUtf8)?;
                        let code = u32::from_str_radix(string, 16).map_err(|e| {
                            DukaLexerError::InvalidEscaped(
                                format!("invalid unicode value {}", e).into_boxed_str(),
                            )
                        })?;
                        if !is_valid_unicode(code) {
                            return Err(DukaLexerError::InvalidUnicodeEscaped(
                                format!("{:x} is invalid unicode value", code).into_boxed_str(),
                            ));
                        }
                        encode_utf8_bytes(code, &mut vec);
                    } else {
                        return Err(DukaLexerError::InvalidEscaped("expected \\u{...}".into()));
                    }
                }

                _ => {
                    return Err(DukaLexerError::InvalidEscaped(
                        format!("unknown escaped character {}", b as char).into_boxed_str(),
                    ));
                }
            },
            None => return Err(DukaLexerError::UnexpectedEnd("\\, a, b, f...".into())),
        }

        Ok(vec)
    }

    fn do_number(&mut self) -> DukaResult<TokenKind, (), DukaLexerError> {
        let mut float = false;
        let mut has_exp = false;
        let mut radix = 10;

        if self.state.current_byte == b'0'
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
                return Complete(TokenKind::Float(0f64));
            } else if b == b'e' || b == b'E' || b == b'.' {
                // 0e2 0E3 0.123
                self.state.buffer.push(b'0');
                has_exp = true;
                // the 'e' or '.' will be processed by following loop
            } else if b.is_ascii_digit() {
                return Err(DukaLexerError::InvalidInteger(
                    "integer cannot start with zero".into(),
                ));
            } else if !b.is_ascii_alphabetic() {
                return Complete(TokenKind::Int(0));
            } else {
                // 0a 0b ... unsupported radix
                return Err(DukaLexerError::InvalidInteger("unsupported radix".into()));
            }
        } else {
            self.state.buffer.push(self.state.current_byte);
        }

        loop {
            let Some(nb) = self.peek_byte()? else {
                if float || has_exp || radix != 10 {
                    return Incomplete((), self.source_info(), "<number>".into(), self.span());
                } else {
                    break;
                }
            };
            match nb {
                b'e' | b'E' if radix == 10 => {
                    float = true;
                    has_exp = true;
                    self.state.buffer.push(b'e');
                    self.read_byte()?;
                }
                b'f' | b'F' if radix == 10 => {
                    if matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_whitespace()) {
                        float = true;
                        self.read_byte()?;
                        break;
                    } else {
                        return Err(DukaLexerError::InvalidFloat("unknown suffix".into()));
                    }
                }
                b'-' if has_exp && radix == 10 => {
                    self.state.buffer.push(b'-');
                    self.read_byte()?;
                }
                b'.' if radix == 10 => {
                    if !float && matches!(self.peek_byte_nth(1)?, Some(b) if b.is_ascii_digit()) {
                        float = true;
                        self.state.buffer.push(b'.');
                        self.read_byte()?;
                    } else {
                        break;
                    }
                }
                b'_' => {
                    self.read_byte()?;
                } // skip _
                &n if is_valid_radix(n, radix) => {
                    self.state.buffer.push(n);
                    self.read_byte()?;
                }
                _ => break,
            }
        }

        let buf = self.take_buffer();
        let string = str::from_utf8(&buf)
            .map_err(|e| DukaLexerError::ReaderError(e.to_string().into_boxed_str()))?;

        Complete(if float {
            assert_eq!(radix, 10);
            string
                .parse::<DukaFloat>()
                .map_err(|e| DukaLexerError::InvalidFloat(e.to_string().into_boxed_str()))
                .map(TokenKind::Float)?
        } else {
            DukaInt::from_str_radix(string, radix)
                .map_err(|e| DukaLexerError::InvalidInteger(e.to_string().into_boxed_str()))
                .map(TokenKind::Int)?
        })
    }

    fn do_ident_or_keyword(&mut self) -> Result<TokenKind, DukaLexerError> {
        self.clear_buffer();
        self.state.buffer.push(self.state.current_byte);

        loop {
            if let Some(&b) = self.peek_byte()?
                && is_valid_ident(b, false)
            {
                self.read_byte()?;
                self.state.buffer.push(b);
            } else {
                break;
            }
        }

        let buf = self.take_buffer();
        let string = str::from_utf8(&buf).map_err(|_| DukaLexerError::InvalidUtf8)?;
        Ok(match string {
            "export" => TokenKind::Export,
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
            _ => check_identifier(string)
                .map_err(DukaLexerError::UnexpectedCharacter)
                .map(|_| TokenKind::Ident(string.to_owned()))?,
        })
    }

    fn read_byte(&mut self) -> Result<Option<u8>, DukaLexerError> {
        let byte = self
            .input
            .next()
            .transpose()
            .map_err(|e| DukaLexerError::ReaderError(e.to_string().into_boxed_str()))?;

        match byte {
            Some(b) => {
                // utf8的首字节
                if !b.is_ascii()
                    && let ReaderStatus::Default = self.state.status
                {
                    check_utf8_head(b).or_else_error(|| DukaLexerError::InvalidUtf8)?;

                    self.state.status = ReaderStatus::UTF8(len_utf8_by_head(b) - 1);
                    self.state.current_position.step();
                } else if b == b'\n' {
                    matches!(self.state.status, ReaderStatus::UTF8(..))
                        .then_error(|| DukaLexerError::InvalidUtf8)?;

                    self.state.current_position.new_line();
                } else if let ReaderStatus::UTF8(count) = self.state.status {
                    // 还在一个utf8中
                    check_utf8_body(b).or_else_error(|| DukaLexerError::InvalidUtf8)?;

                    self.state.status = if count == 1 { ReaderStatus::Default } else { ReaderStatus::UTF8(count - 1) }
                } else {
                    // 普通ascii
                    self.state.current_position.step();
                }

                self.state.source.push(b);
                self.state.current_byte = b;
                self.state.cursor += 1;

                Ok(Some(b))
            }
            None => {
                self.state.current_byte = DEFAULT_BYTE;

                matches!(self.state.status, ReaderStatus::UTF8(..))
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
            .map_err(|e| DukaLexerError::ReaderError(e.to_string().into_boxed_str()))
    }

    /// call it first when buffer is needed
    #[inline(always)]
    fn clear_buffer(&mut self) {
        self.state.buffer.clear();
    }
    /// this will keep the buffer with capacity of the original one
    /// and return original buffer
    ///
    /// *could that help optimize? or over-designed?*
    #[inline]
    fn take_buffer(&mut self) -> Vec<u8> {
        let new_buffer: Vec<u8> =
            Vec::with_capacity(self.state.buffer.capacity().min(INIT_CAPACITY_LIMIT));
        mem::replace(&mut self.state.buffer, new_buffer)
    }

    #[inline]
    fn collect_source(&self) -> &str {
        // Checked in `read_byte()`
        str::from_utf8(&self.state.source).unwrap()
    }
    fn span(&self) -> Span {
        Span {
            start: self.state.start_position,
            end: self.state.current_position,
        }
    }
    pub fn source_info(&self) -> SourceInfo {
        SourceInfo {
            name: self.state.source_name.clone(),
            source: self.collect_source().as_bytes().into(),
            time: self.state.time,
        }
    }

    pub fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        self.next_token_resumable()
            .and_then(DukaResumable::into_result)
    }
    pub fn next_token_resumable(&mut self) -> Result<DukaResumable<Token, ()>, DukaSpannedError> {
        self.next_kind()
            .map(|dr| dr.map(|t| (t, self.span())))
            .map_err(|err| DukaSpannedError::new(err.into(), self.span(), self.source_info()))
    }
}

impl<Source: Read> DukaLexer<Source> for Lexer<Source> {
    type TokenType = Token;

    fn from_source(source: Source, source_name: Option<String>) -> Self {
        Self::new(source, source_name)
    }
    fn tokenize(mut self) -> Result<TokenStream<Self::TokenType>, DukaSpannedError> {
        Ok(TokenStream::new(
            self.by_ref().collect::<Result<_, _>>()?,
            self.source_info(),
        ))
    }
}

impl<Source: Read> Iterator for Lexer<Source> {
    type Item = Result<Token, DukaSpannedError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.next_token();
        (!matches!(item, Ok((ref t, _)) if t.is_terminator())).then_some(item)
    }
}

use macros::*;

use crate::lexer::token::{Token, TokenKind};

#[derive(Debug)]
enum CacheToken {
    Token(Token),
    ExpandEnd,
}

pub struct LexerWithMacro<Source>
where
    Source: Read,
{
    inner: Lexer<Source>,
    macros: HashMap<MacroName, MacroBody>,
    expanding: Vec<MacroExpanding>,
    cache: Vec<CacheToken>,
}

const KW_DEFINE: &str = "define";
const KW_ENIFED: &str = "enifed";
const KW_UNDEF: &str = "undef";

impl<Source: Read> LexerWithMacro<Source> {
    pub fn new(source: Source, name: Option<String>) -> Self {
        Self {
            inner: Lexer::new(source, name),
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
            KW_DEFINE => {
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
            KW_UNDEF => {
                let name = self._must_ident()?;
                self.macros.remove(&name);
            }
            _ => return Err(self._expected(KW_DEFINE)),
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
                if right {
                    VarArgSeparatorType::All
                } else {
                    VarArgSeparatorType::Left
                },
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
                if right {
                    VarArgSeparatorType::Right
                } else {
                    VarArgSeparatorType::None
                },
            )
        })
    }

    #[inline]
    fn span(&self) -> Span {
        self.inner.span()
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
                                    DukaSpannedError::new(
                                        DukaMacroError::UnknownParameterDefined(
                                            name.into_boxed_str(),
                                        )
                                        .into(),
                                        self.span(),
                                        self.inner.source_info(),
                                    )
                                })?,
                            ));
                        }
                        continue;
                    }
                    _ => (),
                }

                tk.0.is_terminator().then_error(|| {
                    DukaSpannedError::new(
                        DukaMacroError::InvalidMacroBody.into(),
                        tk.1,
                        self.inner.source_info(),
                    )
                })?;

                tk.0.is_left().then(|| depth += 1);
                tk.0.is_right().then(|| depth -= 1);

                (depth < 0).then_error(|| {
                    DukaSpannedError::new(
                        DukaMacroError::InvalidMacroBody.into(),
                        tk.1,
                        self.inner.source_info(),
                    )
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
                                    DukaSpannedError::new(
                                        DukaMacroError::UnknownParameterDefined(
                                            name.into_boxed_str(),
                                        )
                                        .into(),
                                        self.span(),
                                        self.inner.source_info(),
                                    )
                                })?,
                            ));
                        }
                        continue;
                    }
                    TokenKind::Reflex => {
                        (self._must_ident()? == KW_ENIFED).or_else_error(|| {
                            DukaSpannedError::new(
                                DukaMacroError::UnexpectedToken(KW_ENIFED.into()).into(),
                                tk.1,
                                self.inner.source_info(),
                            )
                        })?;
                        break;
                    }
                    _ => (),
                }

                tk.0.is_terminator().then_error(|| {
                    DukaSpannedError::new(
                        DukaMacroError::InvalidMacroBody.into(),
                        tk.1,
                        self.inner.source_info(),
                    )
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
                    token.is_terminator().then_error(|| {
                        DukaSpannedError::new(
                            DukaLexerError::UnexpectedEnd("macro parameter".into()).into(),
                            tk.1,
                            self.inner.source_info(),
                        )
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
                    token.is_terminator().then_error(|| {
                        DukaSpannedError::new(
                            DukaLexerError::UnexpectedEnd("tokens".into()).into(),
                            tk.1,
                            self.inner.source_info(),
                        )
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
                .any(|i| i.0 == name && i.1 >= MAX_EXPANDING_DEPTH)
        {
            return Err(DukaSpannedError::new(
                DukaMacroError::ReachMaxDepth(name.into_boxed_str()).into(),
                self.span(),
                self.inner.source_info(),
            ));
        }

        if let Some((_, count)) = self.expanding.iter_mut().find(|i| i.0 == name) {
            *count += 1;
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
        Ok(if builtin {
            let Ok(builtins) = MACRO_BUILTINS.read() else {
                return Err(DukaSpannedError::new(
                    DukaMacroError::FailedLoadBuiltin.into(),
                    self.span(),
                    self.inner.source_info(),
                ));
            };
            let Some(func) = builtins.get(&name.as_str()) else {
                return Err(DukaSpannedError::new(
                    DukaMacroError::UnknownBuiltinMacro(name.into_boxed_str()).into(),
                    self.span(),
                    self.inner.source_info(),
                ));
            };
            func(call_site, &self.expanding, params)
                .into_iter()
                .map(CacheToken::Token)
                .rev()
                .collect()
        } else {
            let Some((params_count, tokens)) = self.macros.get(&name) else {
                return Err(DukaSpannedError::new(
                    DukaMacroError::UnknownMacro(name.into_boxed_str()).into(),
                    self.span(),
                    self.inner.source_info(),
                ));
            };

            let expanded = tokens
                .iter()
                .flat_map(|tk| match tk {
                    MacroToken::Replace(index) => params.get(*index).cloned().unwrap_or_default(),
                    MacroToken::VarArg(separator, ty) => {
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
                                .then(|| vec.push(separator.clone()));

                                vec.extend(tks.clone());

                                (i < len - 1).then(|| vec.push(separator.clone()));

                                (i == len - 1
                                    && matches!(
                                        ty,
                                        VarArgSeparatorType::Right | VarArgSeparatorType::All
                                    ))
                                .then(|| vec.push(separator.clone()));

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
        })
    }

    fn _must(&mut self, tk: TokenKind) -> Result<(), DukaSpannedError> {
        let name = tk.name();
        self._then(tk)?.or_else_error(|| self._expected(name))
    }

    fn _then(&mut self, tk: TokenKind) -> Result<bool, DukaSpannedError> {
        let n = self._next()?;

        n.0.is_terminator().then_error(|| {
            DukaSpannedError::new(
                DukaLexerError::UnexpectedEnd(n.0.name().into()).into(),
                n.1,
                self.inner.source_info(),
            )
        })?;

        let res = n.0 == tk;
        if !res {
            self.cache.push(CacheToken::Token(n));
        }
        Ok(res)
    }

    fn _expected(&mut self, expected: &str) -> DukaSpannedError {
        DukaSpannedError::new(
            DukaMacroError::UnexpectedToken(expected.into()).into(),
            self.span(),
            self.inner.source_info(),
        )
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

impl<Source: Read> DukaLexer<Source> for LexerWithMacro<Source> {
    type TokenType = Token;

    fn from_source(source: Source, source_name: Option<String>) -> Self {
        Self::new(source, source_name)
    }

    fn tokenize(mut self) -> Result<TokenStream<Self::TokenType>, DukaSpannedError> {
        Ok(TokenStream::new(
            self.by_ref().collect::<Result<_, _>>()?,
            self.inner.source_info(),
        ))
    }
}

impl<Source: Read> Iterator for LexerWithMacro<Source> {
    type Item = Result<Token, DukaSpannedError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.do_macro();
        (!matches!(item, Ok((ref tk, _)) if tk.is_terminator())).then_some(item)
    }
}
