use duka_macros::ThatError;
use std::{
    fmt::{Debug, Display},
    iter::Peekable,
};
pub mod wrapper;

#[derive(Debug)]
enum Token {
    Char(char),
    CharGroup(LiteralGroup),
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
    #[error("Got invalid character")]
    InvalidCharacter,
    #[error("Got invalid escaped character")]
    InvalidEscape,
    #[error("Got invalid `()`")]
    InvalidGroup,
    #[error("Got invalid `{{}}`")]
    InvalidTimes,
    #[error("Got unexpected character, expected EOF")]
    ExpectedEOF,
}

fn tokenize(input: &str) -> Result<Vec<Token>, RegexError> {
    let mut chars = input.chars();
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
                let nch = chars.next().ok_or(RegexError::InvalidEscape)?;
                match nch {
                    'd' => tokens.push(Token::CharGroup(LiteralGroup::Numbers)),
                    'w' => tokens.push(Token::CharGroup(LiteralGroup::Words)),
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
                )
            }
            Some(Token::Plus) => {
                iter.next();
                Node::OneMore(Box::new(atom))
            }
            Some(Token::Star) => {
                iter.next();
                Node::ZeroMore(Box::new(atom))
            }
            Some(Token::Question) => {
                iter.next();
                Node::Optional(Box::new(atom))
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
            Some(Token::CharGroup(cg)) => Ok(Node::LiteralGroup(cg)),
            Some(Token::Caret) => Ok(Node::Marker(Marker::Begin)),
            Some(Token::Dollar) => Ok(Node::Marker(Marker::End)),
            Some(Token::LBracket) => {
                let mut chars = vec![];
                let mut ranges = vec![];
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
                        Token::CharGroup(cg) => ranges.extend(cg.as_range()),
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
                        _ => return Err(RegexError::InvalidCharacter),
                    }
                }
                Ok(Node::CharClass(CharClass {
                    negative,
                    chars,
                    ranges,
                }))
            }
            Some(Token::LParen) => {
                let inner = parse_union(iter)?;
                if !matches!(iter.next(), Some(Token::RParen)) {
                    return Err(RegexError::InvalidGroup);
                }
                Ok(Node::Group(Box::new(inner)))
            }
            None => Ok(Node::Empty),
            _ => Err(RegexError::InvalidCharacter),
        }
    }

    let node = parse_union(&mut stream)?;
    if stream.peek().is_some() {
        return Err(RegexError::ExpectedEOF);
    }
    Ok(node)
}

pub fn compile(regex: &str) -> Result<NFAContext, RegexError> {
    let toks = tokenize(regex)?;
    let node = parse(toks)?;
    let compiler = NFACompiler::new();
    Ok(compiler.compile(&node))
}

type NFAStateID = usize;

#[derive(Debug)]
pub struct NFAContext {
    states: Vec<NFAState>,
    snippet: NFASnippet,
    group_count: usize,
    counter_count: usize,
}
impl Display for NFAContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "digraph NFA {{")?;
        writeln!(f, "   rankdir=LR;")?;
        writeln!(f, "   node [shape=circle,fontname=\"sans-serif\"];")?;

        writeln!(f, "   start [shape=point,style=filled,fillcolor=black];")?;
        writeln!(f, "   start -> {};", self.snippet.start)?;

        writeln!(
            f,
            "   {} [shape=doublecircle,peripheries=2];",
            self.snippet.end
        )?;
        for (i, state) in self.states.iter().enumerate() {
            if state.transitions.is_empty() && i != self.snippet.end {
                writeln!(f, "  {};", i)?;
            }
            for trans in &state.transitions {
                if let Some(a) = &trans.2 {
                    writeln!(
                        f,
                        "  {} -> {} [label=\"{:?} [{:?}]\"];",
                        i, trans.0, trans.1, a
                    )?;
                } else {
                    writeln!(f, "  {} -> {} [label=\"{:?}\"];", i, trans.0, trans.1)?;
                }
            }
        }

        writeln!(f, "}}")
    }
}

#[derive(Debug, Clone)]
struct Thread {
    pc: usize,
    captures: Vec<Option<(usize, usize)>>,
    counts: Vec<usize>,
    start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<(usize, usize)>,
}

impl NFAContext {
    fn add_thread(
        &self,
        list: &mut Vec<Thread>,
        seen: &mut [bool],
        pc: usize,
        captures: &[Option<(usize, usize)>],
        counts: &[usize],
        start: usize,
        pos: usize,
        total: usize,
        cur: usize,
    ) {
        if seen[pc] {
            return;
        }
        seen[pc] = true;
        for trans in &self.states[pc].transitions {
            let zero_width = match &trans.1 {
                Predication::Epsilon => true,
                Predication::Marker(Marker::Begin) => pos == 0,
                Predication::Marker(Marker::End) => pos == total,
                Predication::Guard(f) => f(counts),
                _ => false,
            };
            if zero_width {
                let mut caps = captures.to_vec();
                let mut cnts = counts.to_vec();
                if let Some(action) = &trans.2 {
                    self.apply_action(action, &mut caps, &mut cnts, cur);
                }
                self.add_thread(list, seen, trans.0, &caps, &cnts, start, pos, total, cur);
            }
        }
        list.push(Thread {
            pc,
            captures: captures.to_vec(),
            counts: counts.to_vec(),
            start,
        });
    }
    fn apply_action(
        &self,
        action: &NFAAction,
        captures: &mut [Option<(usize, usize)>],
        counts: &mut [usize],
        cur: usize,
    ) {
        match action {
            NFAAction::Capture(group) => captures[*group] = Some((cur, cur)),
            NFAAction::EndCapture(group) => {
                if let Some((start, _)) = captures[*group] {
                    captures[*group] = Some((start, cur));
                }
            }
            NFAAction::IncCounter(cid) => counts[*cid] += 1,
        }
    }
    fn record(&self, best: &mut Option<Match>, start: usize, end: usize, captures: &[Option<(usize, usize)>]) {
        if let Some(m) = best {
            if m.start < start || (m.start == start && m.end >= end) {
                return;
            }
        }
        *best = Some(Match {
            start,
            end,
            captures: captures.iter().flatten().copied().collect(),
        });
    }
    fn step(
        &self,
        nlist: &mut Vec<Thread>,
        nseen: &mut [bool],
        threads: &[Thread],
        c: char,
        pos: usize,
        total: usize,
        cur: usize,
    ) {
        for thread in threads {
            for trans in &self.states[thread.pc].transitions {
                let matched = match &trans.1 {
                    Predication::Always => true,
                    Predication::Char(cc) => c == *cc,
                    Predication::Closure(f) => f(c),
                    Predication::Func(f) => f(c),
                    _ => false,
                };
                if matched {
                    let mut caps = thread.captures.clone();
                    let mut cnts = thread.counts.clone();
                    if let Some(action) = &trans.2 {
                        self.apply_action(action, &mut caps, &mut cnts, cur);
                    }
                    self.add_thread(
                        nlist,
                        nseen,
                        trans.0,
                        &caps,
                        &cnts,
                        thread.start,
                        pos + 1,
                        total,
                        cur + c.len_utf8(),
                    );
                }
            }
        }
    }
    fn search_from(&self, text: &str, from: usize) -> Option<Match> {
        let total = text.chars().count();
        let from_chars = text[..from].chars().count();
        let initial = vec![None; self.group_count];
        let zero_counts = vec![0; self.counter_count];

        let mut active: Vec<Thread> = vec![];
        let mut best: Option<Match> = None;
        let mut cur = from;

        for (i, c) in text[from..].chars().enumerate() {
            let pos = from_chars + i;
            let mut nlist: Vec<Thread> = vec![];
            let mut nseen = vec![false; self.states.len()];
            self.step(&mut nlist, &mut nseen, &active, c, pos, total, cur);
            let mut snlist: Vec<Thread> = vec![];
            let mut snseen = vec![false; self.states.len()];
            self.add_thread(
                &mut snlist,
                &mut snseen,
                self.snippet.start,
                &initial,
                &zero_counts,
                cur,
                pos,
                total,
                cur,
            );
            for t in &snlist {
                if t.pc == self.snippet.end {
                    self.record(&mut best, t.start, cur, &t.captures);
                }
            }
            self.step(&mut nlist, &mut nseen, &snlist, c, pos, total, cur);
            cur += c.len_utf8();
            for t in &nlist {
                if t.pc == self.snippet.end {
                    self.record(&mut best, t.start, cur, &t.captures);
                }
            }
            active = nlist;
        }

        let mut nlist: Vec<Thread> = vec![];
        let mut nseen = vec![false; self.states.len()];
        self.add_thread(
            &mut nlist,
            &mut nseen,
            self.snippet.start,
            &initial,
            &zero_counts,
            cur,
            total,
            total,
            cur,
        );
        for t in &nlist {
            if t.pc == self.snippet.end {
                self.record(&mut best, t.start, cur, &t.captures);
            }
        }
        best
    }
    pub fn search(&self, text: &str) -> Option<Match> {
        self.search_from(text, 0)
    }
    pub fn find_all(&self, text: &str) -> Vec<Match> {
        let mut result = vec![];
        let mut from = 0;
        loop {
            let Some(m) = self.search_from(text, from) else { break };
            let (start, end) = (m.start, m.end);
            result.push(m);
            if end > start {
                from = end;
            } else if let Some(c) = text[end..].chars().next() {
                from = end + c.len_utf8();
            } else {
                break;
            }
        }
        result
    }
}

#[derive(Debug)]
pub struct NFACompiler {
    states: Vec<NFAState>,
    group_count: usize,
    counter_count: usize,
}
impl NFACompiler {
    fn connect_with_action(
        &mut self,
        from: NFAStateID,
        to: NFAStateID,
        pred: Predication,
        action: NFAAction,
    ) {
        self.states[from].transitions.push((to, pred, Some(action)));
    }
    fn connect(&mut self, from: NFAStateID, to: NFAStateID, pred: Predication) {
        self.states[from].transitions.push((to, pred, None));
    }
    fn new_states(&mut self) -> (NFAStateID, NFAStateID) {
        (self.new_state(), self.new_state())
    }
    fn new_state(&mut self) -> NFAStateID {
        self.states.push(NFAState {
            transitions: vec![],
        });
        self.states.len() - 1
    }
    pub fn new() -> Self {
        Self {
            states: vec![],
            group_count: 0,
            counter_count: 0,
        }
    }
    pub fn compile(mut self, node: &Node) -> NFAContext {
        let snippet = self.generate(node);
        self.optimize();
        NFAContext {
            states: self.states,
            snippet,
            group_count: self.group_count,
            counter_count: self.counter_count,
        }
    }
    fn optimize(&mut self) {
        loop {
            let mut changed = false;

            for i in 0..self.states.len() {
                let mut trans = std::mem::take(&mut self.states[i].transitions);
                for (target, pred, act) in &mut trans {
                    if matches!(pred, Predication::Epsilon) && act.is_none() {
                        let trans2 = self.states[*target].transitions.as_slice();
                        if trans2.len() == 1
                            && let Some((next, Predication::Epsilon, None)) = trans2.first()
                            && *next != *target
                        {
                            *target = *next;
                            changed = true;
                        }
                    }
                }
                self.states[i].transitions = trans;
            }

            if !changed {
                break;
            }
        }
    }
    fn generate(&mut self, node: &Node) -> NFASnippet {
        match node {
            Node::Counter(inner, times) => {
                let inner = self.generate(inner);
                let (start, end) = self.new_states();
                let cid = self.counter_count;
                self.counter_count += 1;
                let (min, max) = match times {
                    Times::Exact(n) => (*n, *n),
                    Times::Min(n) => (*n, usize::MAX),
                    Times::Max(m) => (0, *m),
                    Times::Range(n, m) => (*n, *m),
                };

                self.connect_with_action(
                    start,
                    inner.start,
                    Predication::Epsilon,
                    NFAAction::IncCounter(cid),
                );
                self.connect_with_action(
                    inner.end,
                    inner.start,
                    Predication::Guard(Box::new(move |counts| counts[cid] < max)),
                    NFAAction::IncCounter(cid),
                );
                self.connect(
                    inner.end,
                    end,
                    Predication::Guard(Box::new(move |counts| counts[cid] >= min)),
                );
                if min == 0 {
                    self.connect(start, end, Predication::Epsilon);
                }

                NFASnippet { start, end }
            }
            Node::Empty => NFASnippet {
                start: self.new_state(),
                end: self.new_state(),
            },
            Node::Concat(nodes) => {
                let mut first = self.generate(&nodes[0]);
                for node in nodes.iter().skip(1) {
                    let next = self.generate(node);
                    self.connect(first.end, next.start, Predication::Epsilon);
                    first = NFASnippet {
                        start: first.start,
                        end: next.end,
                    };
                }
                first
            }
            Node::Union(nodes) => {
                let (start, end) = self.new_states();
                for node in nodes {
                    let s = self.generate(node);
                    self.connect(start, s.start, Predication::Epsilon);
                    self.connect(s.end, end, Predication::Epsilon);
                }
                NFASnippet { start, end }
            }
            Node::Literal(ch) => {
                let (start, end) = self.new_states();
                self.connect(start, end, Predication::Char(*ch));
                NFASnippet { start, end }
            }
            Node::LiteralGroup(lg) => {
                let (start, end) = self.new_states();
                self.connect(start, end, Predication::Func(lg.as_fn()));
                NFASnippet { start, end }
            }
            Node::Marker(marker) => {
                let (start, end) = self.new_states();
                self.connect(start, end, Predication::Marker(marker.clone()));
                NFASnippet { start, end }
            }
            Node::Group(node) => {
                let (start, end) = self.new_states();
                let inner = self.generate(node);
                let group = self.group_count;
                self.group_count += 1;
                self.connect_with_action(
                    start,
                    inner.start,
                    Predication::Epsilon,
                    NFAAction::Capture(group),
                );
                self.connect_with_action(
                    inner.end,
                    end,
                    Predication::Epsilon,
                    NFAAction::EndCapture(group),
                );
                NFASnippet { start, end }
            }
            Node::CharClass(cc) => {
                let (start, end) = self.new_states();
                let cc = cc.clone();
                self.connect(
                    start,
                    end,
                    Predication::Closure(Box::new(move |c| {
                        let r = cc.chars.contains(&c)
                            || cc.ranges.iter().any(|(s, e)| *s <= c && c <= *e);
                        if cc.negative { !r } else { r }
                    })),
                );
                NFASnippet { start, end }
            }
            Node::OneMore(node) => {
                let inner = self.generate(node);
                let (start, end) = self.new_states();

                //self.connect(start, end, Predication::Epsilon); //Zero
                self.connect(start, inner.start, Predication::Epsilon); //One
                self.connect(inner.end, inner.start, Predication::Epsilon); //More
                self.connect(inner.end, end, Predication::Epsilon); //Exit

                NFASnippet { start, end }
            }
            Node::Optional(node) => {
                let inner = self.generate(node);
                let (start, end) = self.new_states();

                self.connect(start, inner.start, Predication::Epsilon); //One
                self.connect(start, end, Predication::Epsilon); //Zero
                self.connect(inner.end, end, Predication::Epsilon); //Exit
                //self.connect(inner.end, inner.start, Predication::Epsilon); //More

                NFASnippet { start, end }
            }
            Node::ZeroMore(node) => {
                let inner = self.generate(node);
                let (start, end) = self.new_states();

                self.connect(start, inner.start, Predication::Epsilon); //One
                self.connect(start, end, Predication::Epsilon); //Zero
                self.connect(inner.end, inner.start, Predication::Epsilon); //More
                self.connect(inner.end, end, Predication::Epsilon); //Exit

                NFASnippet { start, end }
            }
            Node::Any => {
                let (start, end) = self.new_states();
                self.connect(start, end, Predication::Always);
                NFASnippet { start, end }
            }
        }
    }
}

#[derive(Debug)]
pub struct NFASnippet {
    pub start: NFAStateID,
    pub end: NFAStateID,
}

#[derive(Debug)]
pub enum NFAAction {
    Capture(usize),
    EndCapture(usize),
    IncCounter(usize),
}

pub type Transition = (NFAStateID, Predication, Option<NFAAction>);
#[derive(Debug)]
pub struct NFAState {
    pub transitions: Vec<Transition>,
}

pub enum Predication {
    Epsilon,
    Always,
    Char(char),
    Func(fn(char) -> bool),
    Closure(Box<dyn Fn(char) -> bool + Send + Sync>),
    Guard(Box<dyn Fn(&[usize]) -> bool + Send + Sync>),
    Marker(Marker),
}
impl Debug for Predication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Predication::Epsilon => write!(f, "ε"),
            Predication::Char(ch) => write!(f, "'{ch}'"),
            Predication::Marker(m) => write!(f, "{m:?}"),
            Predication::Always => write!(f, "any"),
            Predication::Func(_) | Predication::Closure(_) => write!(f, "fn()"),
            Predication::Guard(_) => write!(f, "guard"),
        }
    }
}

#[derive(Debug)]
pub enum Node {
    Empty,
    Concat(Vec<Node>),
    Union(Vec<Node>),
    Literal(char),
    LiteralGroup(LiteralGroup),
    Marker(Marker),
    Group(Box<Node>),
    CharClass(CharClass),
    Counter(Box<Node>, Times),
    OneMore(Box<Node>),
    Optional(Box<Node>),
    ZeroMore(Box<Node>),
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
}

#[derive(Debug, Clone)]
pub enum LiteralGroup {
    Words,
    Numbers,
}
impl LiteralGroup {
    pub fn as_range(&self) -> Vec<(char, char)> {
        match self {
            Self::Numbers => vec![('0', '9')],
            Self::Words => vec![('a', 'z'), ('A', 'Z')],
        }
    }
    pub fn as_fn(&self) -> fn(char) -> bool {
        match self {
            Self::Numbers => |c| c.is_numeric(),
            Self::Words => |c| c.is_ascii_alphabetic(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharClass {
    pub negative: bool,
    pub chars: Vec<char>,
    pub ranges: Vec<(char, char)>,
}

#[cfg(test)]
mod tests {
    use crate::builtin::regex::compile;

    #[test]
    fn test() {
        println!("{}", compile(r#"(abc)"#).unwrap());
    }

    #[test]
    fn test_anchors() {
        let re = |p: &str| compile(p).unwrap();
        assert!(re(r"^a$").search("a").is_some());
        assert!(!re(r"^a$").search("ab").is_some());
        assert!(re(r"^a").search("a").is_some());
        assert!(!re(r"^a").search("ba").is_some());
        assert!(re(r"a$").search("a").is_some());
        assert!(re(r"a$").search("ba").is_some());
        assert!(!re(r"a$").search("ab").is_some());
        assert!(re(r"$").search("").is_some());
        assert!(re(r"^").search("").is_some());
        assert!(re(r"^你好$").search("你好").is_some());
        assert!(!re(r"^你好$").search("你好啊").is_some());
        assert!(re(r"你好$").search("你好").is_some());
        assert!(!re(r"你好$").search("啊你好啊").is_some());
    }

    #[test]
    fn test_counter() {
        let re = |p: &str| compile(p).unwrap();
        assert!(re(r"a{2}").search("aa").is_some());
        assert!(!re(r"a{2}").search("a").is_some());
        assert!(re(r"a{2}").search("aaa").is_some());
        assert!(re(r"a{2,4}").search("aa").is_some());
        assert!(re(r"a{2,4}").search("aaaa").is_some());
        assert!(!re(r"a{2,4}").search("a").is_some());
        assert!(re(r"a{2,4}").search("aaaaa").is_some());
        assert!(re(r"a{2,}").search("aaaa").is_some());
        assert!(!re(r"a{2,}").search("a").is_some());
        assert!(re(r"a{,2}").search("").is_some());
        assert!(re(r"a{,2}").search("aa").is_some());
        assert!(re(r"a{,2}").search("aaa").is_some());
        assert!(re(r"(a){1,2}").search("aa").is_some());
        assert!(re(r"(a){1,2}").search("aaa").is_some());
        assert_eq!(
            re(r"(a){1,2}").search("aa").map(|m| m.captures),
            Some(vec![(1, 2)])
        );
        assert!(re(r"a{2,4}b").search("aaab").is_some());
        assert!(!re(r"a{2,4}b").search("ab").is_some());
    }

    #[test]
    fn test_basic() {
        let re = |p: &str| compile(p).unwrap();
        assert!(re(r"a").search("a").is_some());
        assert!(!re(r"a").search("b").is_some());
        assert!(re(r"abc").search("abc").is_some());
        assert!(re(r"abc").search("xxabc").is_some());
        assert!(re(r"a*").search("").is_some());
        assert!(re(r"a+").search("aaa").is_some());
        assert!(re(r"[0-9]+").search("123").is_some());
        assert!(re(r"\d").search("7").is_some());
        assert!(re(r"\w+").search("hello").is_some());
        assert!(re(r"(ab)+").search("abab").is_some());
        assert!(re(r"a|b").search("b").is_some());
        assert!(re(r"a|b").search("a").is_some());
    }

    #[test]
    fn test_capture() {
        let re = |p: &str| compile(p).unwrap();
        assert_eq!(re(r"(a)").search("a").map(|m| m.captures), Some(vec![(0, 1)]));
        assert_eq!(
            re(r"(a)(b)").search("ab").map(|m| m.captures),
            Some(vec![(0, 1), (1, 2)])
        );
        assert_eq!(
            re(r"(ab)+").search("abab").map(|m| m.captures),
            Some(vec![(2, 4)])
        );
        assert_eq!(
            re(r"(a)+").search("aaa").map(|m| m.captures),
            Some(vec![(2, 3)])
        );
        assert_eq!(re(r"a(b)c").search("abc").map(|m| m.captures), Some(vec![(1, 2)]));
        assert_eq!(re(r"(你好)").search("你好").map(|m| m.captures), Some(vec![(0, 6)]));
        assert_eq!(re(r"(a|b)").search("a").map(|m| m.captures), Some(vec![(0, 1)]));
        assert!(re(r"(a)?b").search("b").is_some());
        assert!(re(r"(a)?b").search("ab").is_some());
        assert_eq!(re(r"(a)?b").search("b").map(|m| m.captures), Some(vec![]));
        assert_eq!(re(r"(a)?b").search("ab").map(|m| m.captures), Some(vec![(0, 1)]));
        assert!(re(r"a.c").search("abc").is_some());
        assert!(!re(r"a.c").search("ac").is_some());
        assert!(re(r".*").search("anything").is_some());
        assert!(re(r"(\w+)@(\w+).com").search("bob@example.com").is_some());
        assert_eq!(
            re(r"(\w+)@(\w+).com").search("bob@example.com").map(|m| m.captures),
            Some(vec![(0, 3), (4, 11)])
        );
        assert_eq!(re(r"(\w+)").search("bob").map(|m| m.captures), Some(vec![(0, 3)]));
    }

    #[test]
    fn test_find_all() {
        let re = |p: &str| compile(p).unwrap();
        assert_eq!(
            re(r"(a){1,2}").find_all("aaa").into_iter().map(|m| (m.start, m.end)).collect::<Vec<_>>(),
            vec![(0, 2), (2, 3)]
        );
        assert_eq!(
            re(r"\w+").find_all("bob@example.com").into_iter().map(|m| (m.start, m.end)).collect::<Vec<_>>(),
            vec![(0, 3), (4, 11), (12, 15)]
        );
        assert_eq!(
            re(r"a*").find_all("bb").into_iter().map(|m| (m.start, m.end)).collect::<Vec<_>>(),
            vec![(0, 0), (1, 1), (2, 2)]
        );
    }
}
