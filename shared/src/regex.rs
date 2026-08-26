use crate::utils::{FixedRestore, OrError, UniqueVec};
use duka_macros::{Info, ThatError};
use std::{fmt::Debug, iter::Peekable};

#[derive(Debug, PartialEq, Info)]
enum Token {
    Char(char),
    CharGroup(LiteralGroup, bool),
    Boundary(bool),
    Ref(usize),
    Hyphen,
    Plus,
    Star,
    Question,
    Dollar,
    Caret,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Or,
    Dot,
}

#[derive(Debug, ThatError)]
pub enum RegexError {
    #[error("Got unknown group named {}")]
    UnknownGroup(String),
    #[error("Got invalid character, expected {}")]
    InvalidCharacter(String),
    #[error("Got invalid escaped character")]
    InvalidEscape,
    #[error("Got invalid `{{}}`")]
    InvalidTimes,
    #[error("Got unexpected character, expected EOF")]
    ExpectedEOF,
}

pub fn escape(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '?' | '+' | '*' | '[' | ']' | '{' | '}' | '(' | ')' | '^' | '$' | '|' | '-' | '.'
            | '\\' => {
                output.push('\\');
                output.push(ch);
            }
            _ => output.push(ch),
        }
    }
    output
}

enum ReplacementNode {
    Literal(String),
    Ref(usize),
    RefNamed(String),
}

fn parse_replacement(replacement: &str) -> Result<Vec<ReplacementNode>, RegexError> {
    let mut chars = replacement.chars().peekable();
    let mut buffer = String::new();
    let mut res = vec![];
    while let Some(ch) = chars.next() {
        match ch {
            '$' => match chars.next().ok_or(RegexError::InvalidEscape)? {
                '$' => buffer.push('$'),
                '{' => {
                    if !buffer.is_empty() {
                        res.push(ReplacementNode::Literal(std::mem::take(&mut buffer)));
                    }
                    while let Some(ch) = chars.peek()
                        && ch.is_ascii_alphanumeric()
                    {
                        buffer.push(*ch);
                        chars.next();

                        if matches!(chars.peek(), Some('}')) {
                            break;
                        }
                    }
                    (!matches!(chars.next(), Some('}')))
                        .then_error(|| RegexError::InvalidEscape)?;

                    if !buffer.is_empty() {
                        let st = std::mem::take(&mut buffer);
                        if let Ok(i) = st.parse::<usize>() {
                            res.push(ReplacementNode::Ref(i));
                        } else {
                            res.push(ReplacementNode::RefNamed(st));
                        }
                    }
                }
                c if c.is_ascii_digit() => {
                    if !buffer.is_empty() {
                        res.push(ReplacementNode::Literal(std::mem::take(&mut buffer)));
                    }
                    res.push(ReplacementNode::Ref(
                        c.to_digit(10).unwrap_or_default() as usize
                    ))
                }
                c => buffer.push(c),
            },
            _ => buffer.push(ch),
        }
    }
    if !buffer.is_empty() {
        res.push(ReplacementNode::Literal(std::mem::take(&mut buffer)));
    }
    Ok(res)
}

fn tokenize(input: &str) -> Result<Vec<Token>, RegexError> {
    let mut chars = input.chars().peekable();
    let mut tokens = vec![];
    while let Some(ch) = chars.next() {
        match ch {
            '?' => tokens.push(Token::Question),
            '+' => tokens.push(Token::Plus),
            '*' => tokens.push(Token::Star),
            '[' => tokens.push(Token::LBracket),
            ']' => tokens.push(Token::RBracket),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '^' => tokens.push(Token::Caret),
            '$' => tokens.push(Token::Dollar),
            '|' => tokens.push(Token::Or),
            '-' => tokens.push(Token::Hyphen),
            '.' => tokens.push(Token::Dot),
            '\\' => {
                if chars.peek().is_some_and(|c| c == &'{') {
                    chars.next();
                    let mut buffer = String::new();
                    while let Some(ch) = chars.peek()
                        && ch.is_ascii_digit()
                    {
                        buffer.push(*ch);
                        chars.next();
                        if matches!(chars.peek(), Some('}')) {
                            break;
                        }
                    }
                    (!matches!(chars.next(), Some('}')))
                        .then_error(|| RegexError::InvalidEscape)?;
                    let idx = buffer
                        .parse::<usize>()
                        .map_err(|_| RegexError::InvalidEscape)?;
                    tokens.push(Token::Ref(idx));
                    continue;
                }

                let nch = chars.next().ok_or(RegexError::InvalidEscape)?;
                match nch {
                    'd' => tokens.push(Token::CharGroup(LiteralGroup::Numbers, false)),
                    'w' => tokens.push(Token::CharGroup(LiteralGroup::Words, false)),
                    's' => tokens.push(Token::CharGroup(LiteralGroup::Spaces, false)),
                    'D' => tokens.push(Token::CharGroup(LiteralGroup::Numbers, true)),
                    'W' => tokens.push(Token::CharGroup(LiteralGroup::Words, true)),
                    'S' => tokens.push(Token::CharGroup(LiteralGroup::Spaces, true)),
                    'b' => tokens.push(Token::Boundary(false)),
                    'B' => tokens.push(Token::Boundary(true)),
                    n if n.is_ascii_digit() => {
                        tokens.push(Token::Ref(n.to_digit(10).unwrap_or_default() as usize))
                    }
                    _ => tokens.push(Token::Char(nch)),
                }
            }
            _ => tokens.push(Token::Char(ch)),
        }
    }
    Ok(tokens)
}

fn parse(tokens: Vec<Token>) -> Result<Node, RegexError> {
    let mut stream = tokens.into_iter().peekable();

    fn then<I>(iter: &mut Peekable<I>, c: Token) -> bool
    where
        I: Iterator<Item = Token>,
    {
        if matches!(iter.peek(), Some(ch) if c == *ch) {
            iter.next();
            true
        } else {
            false
        }
    }
    fn expect<I>(iter: &mut Peekable<I>, c: Token) -> Result<(), RegexError>
    where
        I: Iterator<Item = Token>,
    {
        matches!(iter.next(), Some(ch) if ch == c)
            .then_some(())
            .ok_or(RegexError::InvalidCharacter(c.to_string()))
    }
    fn expect_char<I>(iter: &mut Peekable<I>, c: char) -> Result<(), RegexError>
    where
        I: Iterator<Item = Token>,
    {
        expect(iter, Token::Char(c))
    }

    fn parse_union<I>(iter: &mut Peekable<I>) -> Result<Node, RegexError>
    where
        I: Iterator<Item = Token>,
    {
        let left = parse_concat(iter)?;
        if iter.peek().is_none_or(|o| !matches!(o, Token::Or)) {
            return Ok(left);
        }

        let mut vec = vec![left];
        while let Some(Token::Or) = iter.peek() {
            iter.next();
            let right = parse_concat(iter)?;
            vec.push(right);
        }
        Ok(Node::Union(vec))
    }

    fn parse_concat<I>(iter: &mut Peekable<I>) -> Result<Node, RegexError>
    where
        I: Iterator<Item = Token>,
    {
        let left = parse_term(iter)?;
        if matches!(iter.peek(), Some(Token::Or | Token::RParen) | None) {
            return Ok(left);
        }

        let mut vec = vec![left];
        while let Some(next) = iter.peek() {
            if matches!(next, Token::Or | Token::RParen) {
                break;
            }

            let right = parse_term(iter)?;
            vec.push(right);
        }
        Ok(Node::Concat(vec))
    }

    fn parse_term<I>(iter: &mut Peekable<I>) -> Result<Node, RegexError>
    where
        I: Iterator<Item = Token>,
    {
        let atom = parse_atom(iter)?;
        Ok(match iter.peek() {
            Some(Token::LBrace) => {
                iter.next();

                let mut buffer = String::new();
                let mut comma = false;
                let mut num1: Option<usize> = None;
                let mut num2: Option<usize> = None;
                let mut closed = false;

                while let Some(t) = iter.next() {
                    match t {
                        Token::Char(ch) if ch.is_numeric() => buffer.push(ch),
                        Token::Char(',') if !comma => {
                            comma = true;
                            num1 = if buffer.is_empty() {
                                None
                            } else {
                                Some(buffer.parse().map_err(|_| RegexError::InvalidTimes)?)
                            };
                            buffer.clear();
                        }
                        Token::RBrace => {
                            num2 = if buffer.is_empty() {
                                None
                            } else {
                                Some(buffer.parse().map_err(|_| RegexError::InvalidTimes)?)
                            };
                            closed = true;
                            break;
                        }
                        _ => return Err(RegexError::InvalidTimes),
                    }
                }

                if !closed {
                    return Err(RegexError::InvalidTimes);
                }

                Node::Counter(
                    Box::new(atom),
                    match (num1, num2) {
                        (None, None) => return Err(RegexError::InvalidTimes),
                        (Some(n1), Some(n2)) if n1 <= n2 => Times::Range(n1, n2),
                        (Some(_), Some(_)) => return Err(RegexError::InvalidTimes),
                        (Some(n1), None) if comma => Times::Min(n1),
                        (None, Some(n2)) if comma => Times::Max(n2),
                        (_, Some(n)) => Times::Exact(n),
                        _ => unreachable!(),
                    },
                    then(iter, Token::Question),
                )
            }
            Some(Token::Plus) => {
                iter.next();
                Node::OneMore(Box::new(atom), then(iter, Token::Question))
            }
            Some(Token::Star) => {
                iter.next();
                Node::ZeroMore(Box::new(atom), then(iter, Token::Question))
            }
            Some(Token::Question) => {
                iter.next();
                Node::ZeroOne(Box::new(atom), then(iter, Token::Question))
            }
            _ => atom,
        })
    }

    fn parse_atom<I>(iter: &mut Peekable<I>) -> Result<Node, RegexError>
    where
        I: Iterator<Item = Token>,
    {
        match iter.next() {
            Some(Token::Dot) => Ok(Node::Any),
            Some(Token::Char(c)) => Ok(Node::Literal(c)),
            Some(Token::CharGroup(cg, neg)) => Ok(Node::LiteralGroup(cg, neg)),
            Some(Token::Caret) => Ok(Node::Marker(Marker::Begin)),
            Some(Token::Dollar) => Ok(Node::Marker(Marker::End)),
            Some(Token::Boundary(neg)) => Ok(Node::Marker(Marker::Boundary(neg))),
            Some(Token::LBracket) => {
                let mut chars = vec![];
                let mut ranges = vec![];
                let mut neg_ranges = vec![];
                let negative = if matches!(iter.peek(), Some(Token::Caret)) {
                    iter.next();
                    true
                } else {
                    false
                };
                while let Some(token) = iter.next() {
                    match token {
                        Token::RBracket => {
                            break;
                        }
                        Token::Hyphen => {
                            let Some(start) = chars.pop() else {
                                chars.push('-');
                                continue;
                            };
                            let Some(end) = iter.next().and_then(|c| {
                                let Token::Char(c) = c else { return None };
                                Some(c)
                            }) else {
                                chars.push('-');
                                continue;
                            };
                            ranges.push((start, end))
                        }
                        Token::Char(ch) => chars.push(ch),
                        Token::CharGroup(cg, neg) => {
                            if neg {
                                neg_ranges.extend(cg.as_range())
                            } else {
                                ranges.extend(cg.as_range())
                            }
                        }
                        Token::Boundary(neg) => {
                            chars.push('\\');
                            chars.push(if neg { 'B' } else { 'b' });
                        }
                        Token::Plus => chars.push('+'),
                        Token::Star => chars.push('*'),
                        Token::Question => chars.push('?'),
                        Token::Dollar => chars.push('$'),
                        Token::LParen => chars.push('('),
                        Token::RParen => chars.push(')'),
                        Token::LBrace => chars.push('{'),
                        Token::RBrace => chars.push('}'),
                        Token::Or => chars.push('|'),
                        Token::Caret => chars.push('^'),
                        Token::Dot => chars.push('.'),
                        Token::Ref(num) => {
                            for c in num.to_string().chars() {
                                chars.push(c)
                            }
                        }
                        _ => return Err(RegexError::InvalidCharacter("<characters>".to_owned())),
                    }
                }
                Ok(Node::CharClass(CharClass {
                    negative,
                    chars,
                    ranges,
                    neg_ranges,
                }))
            }
            Some(Token::Ref(idx)) => Ok(Node::Group(Group::Ref(idx))),
            Some(Token::LParen) => {
                if then(iter, Token::Question) {
                    if then(iter, Token::Char('=')) {
                        let inner = parse_union(iter)?;
                        expect(iter, Token::RParen)?;
                        Ok(Node::Assertion(Assertion::Lookahead(
                            Box::new(inner),
                            false,
                        )))
                    } else if then(iter, Token::Char('!')) {
                        let inner = parse_union(iter)?;
                        expect(iter, Token::RParen)?;
                        Ok(Node::Assertion(Assertion::Lookahead(Box::new(inner), true)))
                    } else if then(iter, Token::Char(':')) {
                        let inner = parse_union(iter)?;
                        expect(iter, Token::RParen)?;
                        Ok(Node::Group(Group::NonCapturing(Box::new(inner))))
                    } else {
                        expect_char(iter, '<')?;

                        if then(iter, Token::Char('=')) {
                            let inner = parse_union(iter)?;
                            expect(iter, Token::RParen)?;
                            return Ok(Node::Assertion(Assertion::Lookbehind(
                                Box::new(inner),
                                false,
                            )));
                        } else if then(iter, Token::Char('!')) {
                            let inner = parse_union(iter)?;
                            expect(iter, Token::RParen)?;
                            return Ok(Node::Assertion(Assertion::Lookbehind(
                                Box::new(inner),
                                true,
                            )));
                        }

                        let is_ref = then(iter, Token::Char('&'));
                        let mut name = String::new();
                        while let Some(Token::Char(ch)) = iter.peek() {
                            name.push(*ch);
                            iter.next();
                        }
                        expect_char(iter, '>')?;

                        if is_ref {
                            expect(iter, Token::RParen)?;
                            return Ok(Node::Group(Group::RefNamed(name.into_boxed_str())));
                        }

                        let inner = parse_union(iter)?;
                        expect(iter, Token::RParen)?;
                        Ok(Node::Group(Group::Named(
                            name.into_boxed_str(),
                            Box::new(inner),
                        )))
                    }
                } else {
                    let inner = parse_union(iter)?;
                    expect(iter, Token::RParen)?;
                    Ok(Node::Group(Group::Default(Box::new(inner))))
                }
            }
            None => Ok(Node::Empty),
            _ => Err(RegexError::InvalidCharacter("<patterns>".to_owned())),
        }
    }

    let node = parse_union(&mut stream)?;
    if stream.peek().is_some() {
        return Err(RegexError::ExpectedEOF);
    }
    Ok(node)
}

pub fn compile(regex: &str) -> Result<Compiled, RegexError> {
    let toks = tokenize(regex)?;
    let node = parse(toks)?;
    let mut compiler = Compiler::new();
    compiler.compile(node)?;
    Ok(compiler.take())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<(usize, usize)>,
    pub named_captures: Vec<(Box<str>, (usize, usize))>,
}

#[derive(Debug)]
pub enum Instruction {
    Assert(usize, bool /* neg */),
    Match(Cond),
    Check(ZeroCond),
    Action(Action),
    Success,
    Noop,
    Split(usize),
    Jump(usize),
    Condition(ZeroCond, usize /* success */, usize /* failure */),
}

#[derive(Debug)]
pub struct Compiled {
    instructions: Vec<Instruction>,
    group_name_list: Vec<Box<str>>,
    counter_count: usize,
    group_count: usize,
    subs: Vec<Compiled>,
}

#[derive(Debug)]
pub struct Compiler {
    instructions: Vec<Instruction>,
    counter_count: usize,
    group_count: usize,
    group_name_list: UniqueVec<Box<str>>,
    subs: Vec<Compiled>,
}
impl Compiler {
    pub fn new() -> Self {
        Compiler {
            instructions: vec![],
            counter_count: 0,
            group_name_list: UniqueVec::new(),
            group_count: 0,
            subs: vec![],
        }
    }
    fn fillback(&mut self, at: usize, i: Instruction) {
        self.instructions[at] = i
    }
    fn emit(&mut self, i: Instruction) -> usize {
        self.instructions.push(i);
        self.instructions.len() - 1
    }
    fn new_group(&mut self) -> usize {
        let c = self.group_count;
        self.group_count += 1;
        c
    }
    fn new_counter(&mut self) -> usize {
        let c = self.counter_count;
        self.counter_count += 1;
        c
    }
    pub fn take(mut self) -> Compiled {
        self.emit(Instruction::Success);
        Compiled {
            instructions: self.instructions,
            counter_count: self.counter_count,
            group_name_list: self.group_name_list.into_vec(),
            group_count: self.group_count,
            subs: self.subs,
        }
    }
    pub fn compile(&mut self, node: Node) -> Result<(), RegexError> {
        match node {
            Node::Any => self.emit(Instruction::Match(Cond::Any)),
            Node::Empty => self.emit(Instruction::Noop),
            Node::Concat(nodes) => {
                for node in nodes {
                    self.compile(node)?;
                }
                0
            }
            Node::Union(nodes) => {
                let len = nodes.len();
                let mut fillbacks = vec![];
                for (i, node) in nodes.into_iter().enumerate() {
                    if i != len - 1 {
                        let w = self.emit(Instruction::Split(0));
                        self.compile(node)?;
                        fillbacks.push(self.emit(Instruction::Jump(0)));
                        self.fillback(w, Instruction::Split(self.instructions.len()));
                    } else {
                        self.compile(node)?;
                    }
                }
                for at in fillbacks {
                    self.fillback(at, Instruction::Jump(self.instructions.len()));
                }
                0
            }
            Node::Literal(ch) => self.emit(Instruction::Match(Cond::Char(ch))),
            Node::LiteralGroup(lg, neg) => self.emit(Instruction::Match(Cond::Fn(lg.as_fn(), neg))),
            Node::Marker(marker) => self.emit(Instruction::Check(ZeroCond::Marker(marker))),
            Node::Group(group) => match group {
                Group::Ref(idx) => self.emit(Instruction::Check(ZeroCond::Ref(idx))),
                Group::RefNamed(name) => {
                    let idx = self.group_name_list.has(&name);
                    if let Some(idx) = idx {
                        self.emit(Instruction::Check(ZeroCond::RefNamed(idx)))
                    } else {
                        return Err(RegexError::UnknownGroup(name.into_string()));
                    }
                }
                Group::Default(node) => {
                    let g = self.new_group();
                    self.emit(Instruction::Action(Action::Capture(g)));
                    self.compile(*node)?;
                    self.emit(Instruction::Action(Action::EndCapture(g)));
                    0
                }
                Group::Named(n, node) => {
                    let idx = self.group_name_list.push(n);
                    self.emit(Instruction::Action(Action::NamedCapture(idx)));
                    self.compile(*node)?;
                    self.emit(Instruction::Action(Action::EndNamedCapture(idx)));
                    0
                }
                Group::NonCapturing(node) => {
                    self.compile(*node)?;
                    0
                }
            },
            Node::Assertion(assertion) => match assertion {
                Assertion::Lookahead(l, neg) => {
                    let mut compiler = Compiler::new();
                    compiler.compile(*l)?;
                    self.subs.push(compiler.take());
                    self.emit(Instruction::Assert(self.subs.len() - 1, neg));
                    0
                }
                Assertion::Lookbehind(l, neg) => {
                    let mut compiler = Compiler::new();
                    compiler.compile(*l)?;
                    self.subs.push(compiler.take());
                    self.emit(Instruction::Assert(self.subs.len() - 1, neg));
                    0
                }
            },
            Node::CharClass(cc) => {
                self.emit(Instruction::Match(Cond::Closure(Box::new(move |c| {
                    let r = cc.chars.contains(&c)
                        || cc.ranges.iter().any(|(s, e)| *s <= c && c <= *e)
                        || !cc.neg_ranges.iter().any(|(s, e)| *s <= c && c <= *e);
                    if cc.negative { !r } else { r }
                }))))
            }
            Node::Counter(node, times, lazy) => {
                let (min, max) = match times {
                    Times::Exact(n) => (n, n),
                    Times::Min(n) => (n, 10_000_000),
                    Times::Max(m) => (0, m),
                    Times::Range(n, m) => (n, m),
                };
                if lazy {
                    for _ in 0..min {
                        self.compile(*node.clone())?;
                    }
                    let remain = max - min;
                    let c = self.new_counter();
                    let s = self.emit(Instruction::Split(0));
                    let j = self.emit(Instruction::Jump(0));
                    self.fillback(s, Instruction::Split(self.instructions.len()));
                    let cond = self.emit(Instruction::Noop);
                    self.compile(*node)?;
                    self.emit(Instruction::Action(Action::IncCounter(c)));
                    self.emit(Instruction::Jump(s));
                    self.fillback(j, Instruction::Jump(self.instructions.len()));
                    self.fillback(
                        cond,
                        Instruction::Condition(
                            ZeroCond::Guard(Box::new(move |counts| counts[c] < remain)),
                            cond + 1,
                            self.instructions.len(),
                        ),
                    );
                } else {
                    let c = self.new_counter();
                    let l = self.emit(Instruction::Action(Action::IncCounter(c)));
                    let s = self.emit(Instruction::Split(0));
                    self.compile(*node)?;
                    self.emit(Instruction::Condition(
                        ZeroCond::Guard(Box::new(move |counts| counts[c] < max)),
                        l,
                        self.instructions.len() + 1,
                    ));
                    self.fillback(s, Instruction::Split(self.instructions.len()));
                    self.emit(Instruction::Check(ZeroCond::Guard(Box::new(
                        move |counts| counts[c] >= min,
                    ))));
                }
                0
            }
            Node::OneMore(node, lazy) => {
                if lazy {
                    let f = self.instructions.len();
                    self.compile(*node)?;
                    self.emit(Instruction::Split(f));
                } else {
                    let f = self.instructions.len();
                    self.compile(*node)?;
                    let e = self.emit(Instruction::Split(0));
                    self.emit(Instruction::Jump(f));
                    self.fillback(e, Instruction::Split(self.instructions.len()));
                }
                0
            }
            Node::ZeroOne(node, lazy) => {
                if lazy {
                    let f = self.emit(Instruction::Split(0));
                    let j = self.emit(Instruction::Jump(0));
                    self.fillback(f, Instruction::Split(self.instructions.len()));
                    self.compile(*node)?;
                    self.fillback(j, Instruction::Jump(self.instructions.len()));
                } else {
                    let f = self.emit(Instruction::Split(0));
                    self.compile(*node)?;
                    self.fillback(f, Instruction::Split(self.instructions.len()));
                }
                0
            }
            Node::ZeroMore(node, lazy) => {
                if lazy {
                    let f = self.emit(Instruction::Split(0));
                    let j = self.emit(Instruction::Jump(0));
                    self.fillback(f, Instruction::Split(self.instructions.len()));
                    self.compile(*node)?;
                    self.fillback(j, Instruction::Jump(self.instructions.len()));
                } else {
                    let f = self.emit(Instruction::Split(0));
                    self.compile(*node)?;
                    self.emit(Instruction::Jump(f));
                    self.fillback(f, Instruction::Split(self.instructions.len()));
                }
                0
            }
        };
        Ok(())
    }
}

pub struct FindIter<'a> {
    re: &'a Compiled,
    text: &'a str,
    start: usize,
}
impl<'a> FindIter<'a> {
    pub fn new(re: &'a Compiled, text: &'a str) -> Self {
        Self { re, text, start: 0 }
    }
}
impl Iterator for FindIter<'_> {
    type Item = Match;
    fn next(&mut self) -> Option<Self::Item> {
        while self.start <= self.text.len() {
            if let Some(m) = Runner::new(self.re).search(self.text, self.start) {
                if m.end > m.start {
                    self.start = m.end;
                    return Some(m);
                }

                let mut chars = self.text[self.start..].chars();
                if let Some(ch) = chars.next() {
                    self.start += ch.len_utf8();
                } else {
                    self.start = self.text.len() + 1;
                }
                return Some(m);
            }

            if self.start < self.text.len() {
                let ch = self.text[self.start..].chars().next().unwrap();
                self.start += ch.len_utf8();
            } else {
                break;
            }
        }
        None
    }
}

pub fn find_all<'b>(compiled: &'b Compiled, text: &'b str) -> FindIter<'b> {
    FindIter::new(compiled, text)
}

#[derive(Debug)]
struct Frame {
    pc: usize,
    byte_pos: usize,
    capture_len: usize,
    named_capture_len: usize,
    counter_len: usize,
}
#[derive(Debug)]
pub struct Runner<'a> {
    inner: &'a Compiled,
    current: Frame,
    frames: Vec<Frame>,
    captures: FixedRestore<Option<(usize, usize)>>,
    named_captures: FixedRestore<Option<(usize, usize)>>,
    counters: FixedRestore<usize>,
}
impl<'a> Runner<'a> {
    pub fn new(inner: &'a Compiled) -> Runner<'a> {
        Self {
            inner,
            current: Frame {
                pc: 0,
                byte_pos: 0,
                capture_len: 0,
                named_capture_len: 0,
                counter_len: 0,
            },
            frames: vec![],
            captures: FixedRestore::new(inner.group_count),
            counters: FixedRestore::new(inner.counter_count),
            named_captures: FixedRestore::new(inner.group_name_list.len()),
        }
    }
    fn clear(&mut self) {
        self.current = Frame {
            pc: 0,
            byte_pos: 0,
            capture_len: 0,
            named_capture_len: 0,
            counter_len: 0,
        };
        self.frames.clear();
        self.captures = FixedRestore::new(self.inner.group_count);
        self.counters = FixedRestore::new(self.inner.counter_count);
        self.named_captures = FixedRestore::new(self.inner.group_name_list.len());
    }
    fn cur(&self) -> &Frame {
        &self.current
    }
    fn cur_mut(&mut self) -> &mut Frame {
        &mut self.current
    }
    fn split(&mut self, pc: usize) {
        self.frames.push(Frame {
            pc,
            byte_pos: self.current.byte_pos,
            capture_len: self.captures.point(),
            named_capture_len: self.named_captures.point(),
            counter_len: self.counters.point(),
        });
    }
    fn fail(&mut self) -> bool {
        if let Some(f) = self.frames.pop() {
            self.captures.restore(f.capture_len);
            self.named_captures.restore(f.named_capture_len);
            self.counters.restore(f.counter_len);
            self.current = f;
            true
        } else {
            false
        }
    }
    fn run_zero_cond(&mut self, cond: &ZeroCond, text: &str) -> bool {
        match cond {
            ZeroCond::Guard(f) => f(self.counters.as_slice()),
            ZeroCond::Ref(r) => {
                if let Some(Some((from, end))) = self.captures.get(*r)
                    && from != end
                {
                    text[self.cur().byte_pos..].starts_with(&text[*from..*end])
                } else {
                    false
                }
            }
            ZeroCond::RefNamed(n) => {
                if let Some(Some((from, end))) = self.named_captures.get(*n)
                    && from != end
                {
                    text[self.cur().byte_pos..].starts_with(&text[*from..*end])
                } else {
                    false
                }
            }
            ZeroCond::Marker(m) => match m {
                Marker::Begin => self.cur().byte_pos == 0,
                Marker::End => self.cur().byte_pos == text.len(),
                Marker::Boundary(neg) => {
                    let pos = self.cur().byte_pos;
                    let next_word = text[pos..]
                        .chars()
                        .next()
                        .map_or(false, |c| c.is_alphabetic() || c == '_');
                    let former_word = text[..pos]
                        .chars()
                        .next_back()
                        .map_or(false, |c| c.is_alphabetic() || c == '_');
                    let b = next_word ^ former_word;
                    if *neg { !b } else { b }
                }
            },
        }
    }
    fn run_frame(&mut self, text: &str) -> (bool, usize) {
        let mut succeed = false;
        while self.cur().pc < self.inner.instructions.len() {
            let inst = &self.inner.instructions[self.cur().pc]; //checked
            match inst {
                Instruction::Assert(sub, neg) => {
                    let inner = &self.inner.subs[*sub];
                    let res = Runner::new(inner).run_frame(&text[self.cur().byte_pos..]);
                    if *neg && res.0 || !*neg && !res.0 {
                        if !self.fail() {
                            return (succeed, 0);
                        }
                        continue;
                    }
                }
                Instruction::Match(cond) => {
                    let pos = self.cur().byte_pos;
                    if let Some(ch) = text[pos..].chars().next()
                        && match cond {
                            Cond::Any => true,
                            Cond::Char(c) => *c == ch,
                            Cond::Closure(f) => f(ch),
                            Cond::Fn(f, n) => {
                                if *n {
                                    !f(ch)
                                } else {
                                    f(ch)
                                }
                            }
                        }
                    {
                        self.cur_mut().byte_pos += ch.len_utf8();
                    } else {
                        if !self.fail() {
                            return (succeed, 0);
                        }
                        continue;
                    }
                }
                Instruction::Check(cond) => {
                    if !self.run_zero_cond(cond, text) {
                        if !self.fail() {
                            return (succeed, 0);
                        }
                        continue;
                    }
                }
                Instruction::Action(action) => match action {
                    Action::Capture(g) => {
                        let pos = self.cur().byte_pos;
                        self.captures.set(*g, Some((pos, pos)));
                    }
                    Action::NamedCapture(n) => {
                        let pos = self.cur().byte_pos;
                        self.named_captures.set(*n, Some((pos, pos)));
                    }
                    Action::EndNamedCapture(n) => {
                        let pos = self.cur().byte_pos;
                        if let Some(Some((start, _))) = self.named_captures.get(*n) {
                            self.named_captures.set(*n, Some((*start, pos)));
                        }
                    }
                    Action::EndCapture(g) => {
                        let pos = self.cur().byte_pos;
                        if let Some(Some((start, _))) = self.captures.get(*g) {
                            self.captures.set(*g, Some((*start, pos)));
                        }
                        println!("{:?}", &self.captures)
                    }
                    Action::IncCounter(i) => {
                        match self.counters.get(*i) {
                            None => self.counters.set(*i, 1),
                            Some(v) => self.counters.set(*i, *v + 1),
                        };
                    }
                },
                Instruction::Success => {
                    succeed = true;
                    break;
                }
                Instruction::Noop => continue,
                Instruction::Split(pc) => self.split(*pc),
                Instruction::Jump(pc) => {
                    self.cur_mut().pc = *pc;
                    continue;
                }
                Instruction::Condition(cond, yes, no) => {
                    if self.run_zero_cond(cond, text) {
                        self.cur_mut().pc = *yes;
                    } else {
                        self.cur_mut().pc = *no;
                    }
                    continue;
                }
            }
            self.cur_mut().pc += 1;
        }
        (succeed, self.current.byte_pos)
    }
    pub fn replace(
        &mut self,
        text: &str,
        replacement: &str,
        start: usize,
    ) -> Result<String, RegexError> {
        let Some(m) = self.search(text, start) else {
            return Ok(text.to_owned());
        };
        let pat = parse_replacement(replacement)?;
        let mut buffer = String::with_capacity(text.len());
        buffer.push_str(&text[0..m.start]);
        for p in pat {
            match p {
                ReplacementNode::Literal(l) => buffer.push_str(&l),
                ReplacementNode::Ref(i) => {
                    if let Some(c) = m.captures.get(i) {
                        buffer.push_str(&text[c.0..c.1]);
                    }
                }
                ReplacementNode::RefNamed(n) => {
                    if let Some((_, c)) = m.named_captures.iter().find(|i| *i.0 == *n.as_str()) {
                        buffer.push_str(&text[c.0..c.1]);
                    }
                }
            }
        }
        buffer.push_str(&text[m.end..]);
        todo!()
    }
    pub fn search(&mut self, text: &str, start: usize) -> Option<Match> {
        self.clear();
        let (succeed, rel_end) = self.run_frame(&text[start..]);
        succeed.then_some(Match {
            start: start,
            end: start + rel_end,
            captures: self
                .captures
                .clone()
                .into_vec()
                .into_iter()
                .flatten()
                .map(|v| (v.0 + start, v.1 + start))
                .collect(),
            named_captures: self
                .named_captures
                .clone()
                .into_vec()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, v)| {
                    (
                        self.inner.group_name_list[i].clone(),
                        (v.0 + start, v.1 + start),
                    )
                })
                .collect(),
        })
    }
}

#[derive(Debug)]
pub enum Action {
    Capture(usize),
    NamedCapture(usize),
    EndNamedCapture(usize),
    EndCapture(usize),
    IncCounter(usize),
}

type GuardFn = Box<dyn Fn(&[usize]) -> bool + Send + Sync + 'static>;

pub enum ZeroCond {
    Guard(GuardFn),
    Marker(Marker),
    Ref(usize),
    RefNamed(usize),
}
impl Debug for ZeroCond {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZeroCond::Marker(m) => write!(f, "{m:?}"),
            ZeroCond::Guard(_) => write!(f, "guard"),
            ZeroCond::Ref(idx) => write!(f, "\\{idx}"),
            ZeroCond::RefNamed(n) => write!(f, "(?<&Named{n}>)"),
        }
    }
}
pub enum Cond {
    Any,
    Char(char),
    Fn(fn(char) -> bool, bool),
    Closure(Box<dyn Fn(char) -> bool + Send + Sync>),
}
impl Debug for Cond {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cond::Char(ch) => write!(f, "'{ch}'"),
            Cond::Any => write!(f, "any"),
            Cond::Closure(_) => write!(f, "fn()"),
            Cond::Fn(_, neg) => write!(f, "{}fn()", neg.then_some("!").unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Group {
    Default(Box<Node>),
    Named(Box<str>, Box<Node>),
    NonCapturing(Box<Node>),
    Ref(usize),
    RefNamed(Box<str>),
}

#[derive(Debug, Clone)]
pub enum Assertion {
    Lookahead(Box<Node>, bool /* neg */),
    Lookbehind(Box<Node>, bool /* neg */),
}

#[derive(Debug, Clone)]
pub enum Node {
    Empty,
    Concat(Vec<Node>),
    Union(Vec<Node>),
    Literal(char),
    LiteralGroup(LiteralGroup, bool),
    Marker(Marker),
    Group(Group),
    Assertion(Assertion),
    CharClass(CharClass),
    Counter(Box<Node>, Times, bool),
    OneMore(Box<Node>, bool),
    ZeroOne(Box<Node>, bool),
    ZeroMore(Box<Node>, bool),
    Any,
}

#[derive(Debug, Clone)]
pub enum Times {
    Exact(usize),
    Min(usize),
    Max(usize),
    Range(usize, usize),
}

#[derive(Debug, Clone)]
pub enum Marker {
    Begin,
    End,
    Boundary(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiteralGroup {
    Words,
    Numbers,
    Spaces,
}
impl LiteralGroup {
    pub fn as_range(&self) -> Vec<(char, char)> {
        match self {
            Self::Numbers => vec![('0', '9')],
            Self::Words => vec![('a', 'z'), ('A', 'Z')],
            Self::Spaces => vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')],
        }
    }
    pub fn as_fn(&self) -> fn(char) -> bool {
        match self {
            Self::Numbers => |c| c.is_numeric(),
            Self::Words => |c| c.is_ascii_alphabetic(),
            Self::Spaces => |c| c.is_ascii_whitespace(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharClass {
    pub negative: bool,
    pub chars: Vec<char>,
    pub ranges: Vec<(char, char)>,
    pub neg_ranges: Vec<(char, char)>,
}

#[cfg(test)]
mod tests {
    use crate::regex::compile;

    #[test]
    fn test() {
        println!("{:?}", compile("(\\d+)(?=元)"))
    }
}
