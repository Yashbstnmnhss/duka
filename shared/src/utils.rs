use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    hash::Hash,
    iter::Fuse,
};
use unicode_ident::{is_xid_continue, is_xid_start};

use crate::{errors::Span, value::ConstValue};

#[derive(Debug, Clone, PartialEq)]
pub struct UniqueVec<T: Hash + Eq + Clone>(Vec<T>, HashMap<T, usize>);
impl<T: Hash + Eq + Clone> Default for UniqueVec<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Hash + Eq + Clone + Serialize> Serialize for UniqueVec<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}
impl<'de, T: Hash + Eq + Clone + Deserialize<'de>> Deserialize<'de> for UniqueVec<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let vec = Vec::<T>::deserialize(deserializer)?;
        let map = vec
            .iter()
            .cloned()
            .enumerate()
            .map(|(a, b)| (b, a))
            .collect::<HashMap<_, _>>();
        Ok(Self(vec, map))
    }
}
impl<T: Hash + Eq + Clone> UniqueVec<T> {
    pub fn new() -> Self {
        Self(vec![], HashMap::new())
    }
    pub const fn len(&self) -> usize {
        self.0.len()
    }
    pub fn get(&self, key: usize) -> Option<&T> {
        self.0.get(key)
    }
    pub fn push(&mut self, val: T) -> usize {
        self.1.get(&val).copied().unwrap_or_else(|| {
            let i = self.0.len();
            self.0.push(val.clone());
            self.1.insert(val, i);
            i
        })
    }
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}
impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}
impl Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        Ok(())
    }
}
impl SemVer {
    pub const fn describe(self, _description: &str) -> Self {
        self
    }
    pub const fn breaking_change(mut self) -> Self {
        self.major += 1;
        self.minor = 0;
        self.patch = 0;
        self
    }
    pub const fn patch_update(mut self) -> Self {
        self.patch += 1;
        self
    }
    pub const fn feature_update(mut self) -> Self {
        self.minor += 1;
        self.patch = 0;
        self
    }

    pub const fn record() -> Self {
        Self::new(0, 1, 0)
    }
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}
impl From<SemVer> for String {
    fn from(value: SemVer) -> Self {
        value.to_string()
    }
}

/// When returning value does not depend on whether it was success
#[derive(Debug)]
pub enum Action<T> {
    Success(T),
    Failure(T),
}

pub type TryDo<T, E> = Result<Option<T>, E>;

#[derive(Debug, Default, Clone, PartialEq)]
pub enum SymbolType {
    #[default]
    Variable,
    Function,
    Constant(ConstValue),
}

#[derive(Debug)]
pub struct Symbol {
    pub id: usize,
    pub symbol_type: SymbolType,
    pub span: Span,
}

#[derive(Debug)]
pub struct Symbols {
    pub parent: Option<usize>,
    pub parent_function: usize,
    pub children: Vec<usize>,
    pub symbols: HashMap<Box<str>, Vec<Symbol>>,
    pub consts: HashMap<Box<str>, ConstValue>,
    pub labels: HashMap<Box<str>, Span>,
    pub scope_type: ScopeType,
}
/// A common manager of scopes
#[derive(Debug)]
pub struct SymbolTable {
    scopes: Vec<Symbols>,
    current: usize,
    global: usize,
    symbol_id_sp: usize,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum ScopeType {
    Function,
    #[default]
    Normal,
    Loop,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::with_global()
    }
}

impl SymbolTable {
    pub fn with_global() -> Self {
        Self {
            scopes: vec![Symbols {
                consts: HashMap::new(),
                parent: None,
                parent_function: 0,
                symbols: HashMap::new(),
                labels: HashMap::new(),
                scope_type: ScopeType::Function,
                children: vec![],
            }],
            current: 0,
            global: 0,
            symbol_id_sp: 0,
        }
    }
    pub fn enter(&mut self, scope_type: ScopeType) {
        let id = self.scopes.len();
        self.scopes[self.current].children.push(id);
        self.scopes.push(Symbols {
            consts: HashMap::new(),
            parent: Some(self.current),
            parent_function: match scope_type {
                ScopeType::Function => id,
                _ => self.scopes[self.current].parent_function,
            },
            symbols: HashMap::new(),
            labels: HashMap::new(),
            scope_type,
            children: vec![],
        });
        self.current = id
    }
    pub fn exit(&mut self) -> Option<usize> {
        let parent = self.scopes[self.current].parent?;
        self.current = parent;
        Some(parent)
    }

    fn create_symbol(&mut self, symbol_type: SymbolType, span: Span) -> Symbol {
        let sy = Symbol {
            id: self.symbol_id_sp,
            symbol_type,
            span,
        };
        self.symbol_id_sp += 1;
        sy
    }
    pub fn declare_constant(
        &mut self,
        key: impl Into<Box<str>>,
        span: Span,
        val: ConstValue,
    ) -> usize {
        let val = self.create_symbol(SymbolType::Constant(val), span);
        self.scopes[self.current]
            .symbols
            .entry(key.into())
            .or_default()
            .push(val);
        self.symbol_id_sp - 1
    }
    pub fn declare_variable(
        &mut self,
        key: impl Into<Box<str>>,
        span: Span,
        global: bool,
    ) -> usize {
        let val = self.create_symbol(SymbolType::Variable, span);
        self.scopes[global.then_some(self.global).unwrap_or(self.current)]
            .symbols
            .entry(key.into())
            .or_default()
            .push(val);
        self.symbol_id_sp - 1
    }
    pub fn declare_function(
        &mut self,
        key: impl Into<Box<str>>,
        span: Span,
        global: bool,
    ) -> usize {
        let val = self.create_symbol(SymbolType::Function, span);
        self.scopes[global.then_some(self.global).unwrap_or(self.current)]
            .symbols
            .entry(key.into())
            .or_default()
            .push(val);
        self.symbol_id_sp - 1
    }
    pub fn declare_label(&mut self, key: impl Into<Box<str>>, span: Span) -> Result<(), Span> {
        let parent_func = self.scopes[self.current].parent_function;
        self.scopes[parent_func]
            .labels
            .insert(key.into(), span)
            .map(Err)
            .unwrap_or(Ok(()))
    }
    pub fn lookup(&self, key: &str) -> Option<&Symbol> {
        self.lookup_in(key, self.current)
    }
    pub fn lookup_label(&self, key: &str) -> Option<Span> {
        self.lookup_label_in(key, self.current)
    }

    fn lookup_label_in(&self, key: &str, who: usize) -> Option<Span> {
        let mut id = who;
        while let Some(scope) = self.scopes.get(id) {
            if let Some(span) = scope.labels.get(key) {
                return Some(*span);
            }

            if matches!(scope.scope_type, ScopeType::Function) {
                break;
            }
            match scope.parent {
                Some(pid) => id = pid,
                _ => break,
            }
        }
        None
    }
    fn lookup_in(&self, key: &str, who: usize) -> Option<&Symbol> {
        let mut id = who;
        while let Some(scope) = self.scopes.get(id) {
            if let Some(symbols) = scope.symbols.get(key) {
                return symbols.last();
            }

            match scope.parent {
                Some(pid) => id = pid,
                _ => break,
            }
        }
        None
    }

    pub fn current_scope(&self) -> &Symbols {
        &self.scopes[self.current]
    }
}

pub struct SymbolTableViewer<'a> {
    current: usize,
    child_idx: Vec<usize>,
    inner: &'a SymbolTable,
}
impl<'a> SymbolTableViewer<'a> {
    pub fn new(scopes: &'a SymbolTable) -> Self {
        Self {
            current: scopes.global,
            child_idx: vec![],
            inner: scopes,
        }
    }
    pub fn enter(&mut self) {
        let children = &self.inner.scopes[self.inner.current].children;
        if let Some(child_idx) = self.child_idx.last_mut()
            && *child_idx < children.len() - 1
        {
            *child_idx += 1;
            self.current = children[*child_idx];
            self.child_idx.push(0);
        }
    }
    pub fn exit(&mut self) {
        if let Some(parent) = self.inner.scopes[self.inner.current].parent {
            self.current = parent;
            self.child_idx.pop();
        }
    }
    pub fn lookup(&self, key: &str) -> Option<&Symbol> {
        self.inner.lookup_in(key, self.current)
    }
    pub fn lookup_label(&self, key: &str) -> Option<Span> {
        self.inner.lookup_label_in(key, self.current)
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
    #[inline]
    fn new(iter: I) -> Self {
        Self {
            iter: iter.fuse(),
            buf: VecDeque::new(),
        }
    }

    pub fn rollback(&mut self, el: I::Item) {
        self.buf.push_front(el);
    }

    /// ## `n` must be less than `MAX_DEPTH`
    pub fn peek_nth(&mut self, n: usize) -> Option<&<I as Iterator>::Item> {
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
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.iter.size_hint();
        let extra = self.buf.len();
        (low + extra, high.map(|h| h + extra))
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

/// Check if elements are consecutive
/// # Example
/// NO
pub fn is_consecutive(els: &[usize]) -> bool {
    els.len() <= 1 || els.windows(2).all(|a| a[1] == a[0] + 1)
}

/// we must ensure that all the input are valid utf8
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
/// we must ensure that code are valid Unicode
#[inline(always)]
pub fn encode_utf8_bytes(code: u32, target: &mut Vec<u8>) {
    debug_assert!(code <= MAX_UNICODE);
    match code {
        // 一字节
        // 原样放入
        0x00..=0x7F => target.push(code as u8),
        // 两字节
        // code的二进制数字有8~11位
        0x80..=0x7FF => {
            // >> 6 先取前2~5位
            // 0xC0 | ~ 加上前缀11
            target.push(0xC0 | (code >> 6) as u8);
            // & 0x3F 再取出后6位 位掩码用于提取特定位
            // 0x80 | ~ 加上前缀10
            target.push(0x80 | (code & 0x3F) as u8);
        }
        // 三字节
        0x800..=0xFFFF => {
            target.push(0xE0 | (code >> 12) as u8);
            target.push(0x80 | ((code >> 6) & 0x3F) as u8);
            target.push(0x80 | (code & 0x3F) as u8);
        }
        // 四字节
        0x10000..=MAX_UNICODE => {
            target.push(0xF0 | (code >> 18) as u8);
            target.push(0x80 | ((code >> 12) & 0x3F) as u8);
            target.push(0x80 | ((code >> 6) & 0x3F) as u8);
            target.push(0x80 | (code & 0x3F) as u8);
        }
        _ => {}
    }
}

#[inline(always)]
pub const fn is_valid_ident(b: u8, head: bool) -> bool {
    b.is_ascii_alphabetic() || (b.is_ascii_digit() && !head) || b > 127 || b == b'_'
}

/// ensure that ident is not empty, return `Ok` or `Err` with the invalid character
#[inline(always)]
pub fn check_identifier(ident: &str) -> Result<(), char> {
    let mut chars = ident.chars();
    assert!(!ident.is_empty());
    let head = chars.next().unwrap();

    // ATTENTION: XID_START DOESNT CONTAIN "_"
    (is_xid_start(head) || head == '_')
        .then_some(chars.find(|c| !is_xid_continue(*c)).map_or(Ok(()), Err))
        .unwrap_or(Err(head))
}
