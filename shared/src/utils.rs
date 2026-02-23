use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    hash::Hash,
    iter::Fuse,
};
use unicode_ident::{is_xid_continue, is_xid_start};

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

/// When returning value does not depent on whether it was success
#[derive(Debug)]
pub enum Action<T> {
    Success(T),
    Failure(T),
}

pub type TryDo<T, E> = Result<Option<T>, E>;

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum SymbolType {
    #[default]
    Variable,
    Function,
}

#[derive(Debug)]
pub struct Symbol<V> {
    pub symobl_type: SymbolType,
    pub val: V,
}

#[derive(Debug)]
pub struct Scope<K, V> {
    pub parent: Option<usize>,
    pub parent_function: usize,
    pub children: Vec<usize>,
    pub symbols: HashMap<K, Vec<Symbol<V>>>,
    pub labels: Option<HashMap<K, V>>,
    pub scope_type: ScopeType,
}
/// A common manager of scopes
#[derive(Debug)]
pub struct Scopes<K, V>
where
    K: Eq + Hash,
{
    scopes: Vec<Scope<K, V>>,
    current: usize,
    global: usize,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum ScopeType {
    Function,
    #[default]
    Normal,
    Loop,
}

impl<K, V> Default for Scopes<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::with_global()
    }
}

impl<K, V> Scopes<K, V>
where
    K: Eq + Hash,
{
    pub fn with_global() -> Self {
        Self {
            scopes: vec![Scope {
                parent: None,
                parent_function: 0,
                symbols: HashMap::new(),
                labels: Some(HashMap::new()),
                scope_type: ScopeType::Function,
                children: vec![],
            }],
            current: 0,
            global: 0,
        }
    }
    pub fn enter(&mut self, scope_type: ScopeType) {
        let id = self.scopes.len();
        self.scopes[self.current].children.push(id);
        self.scopes.push(Scope {
            parent: Some(self.current),
            parent_function: match scope_type {
                ScopeType::Function => id,
                _ => self.scopes[self.current].parent_function,
            },
            symbols: HashMap::new(),
            labels: matches!(scope_type, ScopeType::Function).then_some(HashMap::new()),
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
    pub fn declare_global(&mut self, key: K, val: Symbol<V>) {
        self.scopes[self.global]
            .symbols
            .entry(key)
            .or_default()
            .push(val)
    }
    pub fn declare(&mut self, key: K, val: Symbol<V>) {
        self.scopes[self.current]
            .symbols
            .entry(key)
            .or_default()
            .push(val)
    }
    pub fn declare_label(&mut self, key: K, val: V) -> Result<(), V> {
        let parent_func = self.scopes[self.current].parent_function;
        self.scopes[parent_func]
            .labels
            .as_mut()
            .unwrap()
            .insert(key, val)
            .map(Err)
            .unwrap_or(Ok(()))
    }
    pub fn lookup(&self, key: &K) -> Option<&Symbol<V>> {
        self.lookup_in(key, self.current)
    }
    pub fn lookup_label(&self, key: &K) -> Option<&V> {
        self.lookup_label_in(key, self.current)
    }

    fn lookup_label_in(&self, key: &K, who: usize) -> Option<&V> {
        let in_func = self.scopes[who].parent_function;
        let func_scope = self.scopes[in_func].labels.as_ref().unwrap();
        func_scope.get(&key)
    }
    fn lookup_in(&self, key: &K, who: usize) -> Option<&Symbol<V>> {
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

    pub fn current_scope(&self) -> &Scope<K, V> {
        &self.scopes[self.current]
    }
}

pub struct ScopesViewer<'a, K, V>
where
    K: Eq + Hash,
{
    current: usize,
    child_idx: Vec<usize>,
    inner: &'a Scopes<K, V>,
}
impl<'a, K: Eq + Hash, V> ScopesViewer<'a, K, V> {
    pub fn new(scopes: &'a Scopes<K, V>) -> Self {
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
    pub fn lookup(&self, key: &K) -> Option<&Symbol<V>> {
        self.inner.lookup_in(key, self.current)
    }
    pub fn lookup_label(&self, key: &K) -> Option<&V> {
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
    els.windows(2).all(|a| a[1] == a[0] + 1)
}

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
