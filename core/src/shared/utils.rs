use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    iter::Fuse,
};

use unicode_ident::{is_xid_continue, is_xid_start};

/// When returning value does not depent on whether it was success
#[derive(Debug)]
pub enum Action<T> {
    Success(T),
    Failure(T),
}

pub type TryDo<T, E> = Result<Option<T>, E>;

type Scope<K, V> = (HashMap<K, V>, ScopeType);
/// A common manager of scopes
#[derive(Debug)]
pub struct Scopes<K, V>
where
    K: Eq + Hash,
{
    global: Scope<K, V>,
    children: Vec<Scope<K, V>>,
}

#[derive(Debug, Default, PartialEq)]
#[allow(unused)]
pub enum ScopeType {
    Function,
    Do,
    ControlFlow,
    #[default]
    Global,
}

#[allow(unused)]
impl<V> Scopes<String, V> {
    pub fn new() -> Self {
        Self {
            global: (HashMap::new(), ScopeType::default()),
            children: vec![],
        }
    }
    pub fn enter(&mut self, ty: ScopeType) {
        self.children.push((HashMap::new(), ty));
    }
    pub fn exit(&mut self) {
        self.children.pop();
    }
    pub fn push(&mut self, key: String, val: V) -> Result<(), ()> {
        let cur = self.get_mut();
        cur.0
            .contains_key(&key)
            .then_some(Err(()))
            .unwrap_or_else(|| {
                cur.0.insert(key, val);
                Ok(())
            })
    }
    pub fn get(&mut self, key: &str) -> Option<&V> {
        self.children
            .iter()
            .rfind(|s| s.0.contains_key(key))
            .map(|s| {
                s.0.get(key)
                    .expect("no way, i have already found it in vector")
            })
            .or_else(|| self.global.0.get(key))
    }
    pub fn find_within(&mut self, key: &str, within: ScopeType) -> bool {
        self.children
            .iter()
            .rposition(|s| s.0.contains_key(key) || s.1 == within)
            .map(|i| self.children[i].0.contains_key(key))
            .unwrap_or_else(|| self.global.0.contains_key(key))
    }
    pub fn get_mut(&mut self) -> &mut Scope<String, V> {
        self.children.last_mut().unwrap_or(&mut self.global)
    }
}

pub trait OrError {
    fn then_error<F, E>(&self, ef: F) -> Result<(), E>
    where
        F: FnOnce() -> E;
    fn or_else_error<F, E>(&self, ef: F) -> Result<(), E>
    where
        F: FnOnce() -> E;
}
impl OrError for bool {
    #[inline]
    fn then_error<F, E>(&self, ef: F) -> Result<(), E>
    where
        F: FnOnce() -> E,
    {
        if *self { Err(ef()) } else { Ok(()) }
    }
    #[inline]
    fn or_else_error<F, E>(&self, ef: F) -> Result<(), E>
    where
        F: FnOnce() -> E,
    {
        if *self { Ok(()) } else { Err(ef()) }
    }
}

/// # PLEASE ONLY USE `next` `count` AND `peek_nth`
#[derive(Debug, Clone)]
pub struct MultiPeekable<I>
where
    I: Iterator,
{
    iter: Fuse<I>,
    buf: VecDeque<I::Item>,
}

impl<I: Iterator> MultiPeekable<I> {
    pub const MAX_DEPTH: usize = 3;

    #[inline]
    fn new(iter: I) -> Self {
        Self {
            iter: iter.fuse(),
            buf: VecDeque::new(),
        }
    }

    /// ## `n` must be less than `MAX_DEPTH`
    pub fn peek_nth(&mut self, n: usize) -> Option<&<I as Iterator>::Item> {
        if n > Self::MAX_DEPTH {
            return None;
        }

        while self.buf.len() <= n {
            match self.iter.next() {
                Some(item) => self.buf.push_back(item),
                None => break,
            }
        }

        self.buf.get(n)
    }
}
impl<I: Iterator> Iterator for MultiPeekable<I> {
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.buf.pop_front().or_else(|| self.iter.next())
    }

    #[inline]
    fn count(self) -> usize {
        // 我为什么要判断不是零然后再相加?
        self.buf.len() + self.iter.count()
    }
}

pub trait MultiPeekableExtension: Iterator + Sized {
    fn multi_peekable(self) -> MultiPeekable<Self>;
}
impl<I: Iterator> MultiPeekableExtension for I {
    fn multi_peekable(self) -> MultiPeekable<Self> {
        MultiPeekable::new(self)
    }
}

#[inline(always)]
pub const fn is_newline(input: u8) -> bool {
    input == b'\n' || input == b'\r'
}

#[inline(always)]
pub const fn is_valid_radix(input: u8, radix: u32) -> bool {
    match radix {
        2 => matches!(input, b'0'..=b'1'),
        8 => matches!(input, b'0'..=b'7'),
        10 => input.is_ascii_digit(),
        16 => input.is_ascii_hexdigit(),
        _ => false,
    }
}

#[inline(always)]
pub const fn get_radix(b: u8) -> Option<u32> {
    match b.to_ascii_lowercase() {
        b'b' => Some(2),
        b'o' => Some(8),
        b'x' => Some(16),
        _ => None,
    }
}

const MAX_UTF8: u8 = 0xF7;
const MAX_UNICODE: u32 = 0x10FFFF;
const UTF8_BODY_MASK: u8 = 0b10000000;

/// we must ensure that all of the input are valid utf8
#[inline(always)]
pub const fn len_utf8_by_head(head: u8) -> u8 {
    match head {
        // 110xxxxx
        0xC0..=0xDF => 2,
        // 1110xxxx
        0xE0..=0xEF => 3,
        // 11110xxx
        0xF0..=MAX_UTF8 => 4,
        _ => 1,
    }
}

#[inline(always)]
pub const fn check_utf8_head(head: u8) -> bool {
    head <= MAX_UTF8
}

#[inline(always)]
pub const fn check_utf8_body(body: u8) -> bool {
    body & UTF8_BODY_MASK != 0
}
#[inline(always)]
pub const fn is_valid_unicode(code: u32) -> bool {
    code <= MAX_UNICODE
}
/// convert u32 to utf8 bytes, write into vec
///
/// we must ensure that code are valid unicode
#[inline(always)]
pub fn encode_utf8_bytes(code: u32, vec: &mut Vec<u8>) {
    match code {
        // 一字节
        // 原样放入
        0x00..=0x7F => vec.push(code as u8),
        // 两字节
        // code的二进制数字有8~11位
        0x80..=0x7FF => {
            // >> 6 先取前2~5位
            // 0xC0 | ~ 加上前缀11
            vec.push(0xC0 | (code >> 6) as u8);
            // & 0x3F 再取出后6位 位掩码用于提取特定位
            // 0x80 | ~ 加上前缀10
            vec.push(0x80 | (code & 0x3F) as u8);
        }
        // 三字节
        0x800..=0xFFFF => {
            vec.push(0xE0 | (code >> 12) as u8);
            vec.push(0x80 | ((code >> 6) & 0x3F) as u8);
            vec.push(0x80 | (code & 0x3F) as u8);
        }
        // 四字节
        0x10000..=MAX_UNICODE => {
            vec.push(0xF0 | (code >> 18) as u8);
            vec.push(0x80 | ((code >> 12) & 0x3F) as u8);
            vec.push(0x80 | ((code >> 6) & 0x3F) as u8);
            vec.push(0x80 | (code & 0x3F) as u8);
        }
        _ => unreachable!(),
    }
}

#[inline(always)]
pub const fn is_valid_ident(b: u8, head: bool) -> bool {
    b.is_ascii_alphabetic() || (b.is_ascii_digit() && !head) || b > 127 || b == b'_'
}

/// ensure that ident is not empty
#[inline(always)]
pub fn check_identifier(ident: &str) -> Result<(), char> {
    let mut chars = ident.chars();
    assert!(ident.len() != 0);
    let head = chars.next().expect("assert!() will deal this first");

    // ATTENTION: XID_START DOESNT CONTAIN "_"
    (is_xid_start(head) || head == '_')
        .then_some(chars.find(|c| !is_xid_continue(*c)).map_or(Ok(()), Err))
        .unwrap_or(Err(head))
}
