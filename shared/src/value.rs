use duka_macros::Info;
use serde::{Deserialize, Serialize};

use core::str;
use std::{borrow::Cow, collections::HashMap, fmt::Display, hash::Hash};

use crate::{dtype::Type, utils::Successor};

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

/// accepting mutable state of running vm, returning count of result
// pub type DukaFunc = fn(&mut Box<dyn DukaRuntime>) -> i32; moved to backend
/// integer type
pub type DukaInt = i64;
/// float type
pub type DukaFloat = f64;

impl Successor for DukaInt {
    fn succ(self) -> Option<Self> {
        self.checked_add(1)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd)]
pub struct DukaFloatWrapper(pub DukaFloat);

impl PartialEq<DukaFloatWrapper> for DukaInt {
    fn eq(&self, other: &DukaFloatWrapper) -> bool {
        let f = (*self) as DukaFloat;
        f.eq(&other.0)
    }
}
impl PartialOrd<DukaFloatWrapper> for DukaInt {
    fn partial_cmp(&self, other: &DukaFloatWrapper) -> Option<std::cmp::Ordering> {
        other.partial_cmp(self).map(std::cmp::Ordering::reverse)
    }
}
impl PartialEq<DukaInt> for DukaFloatWrapper {
    fn eq(&self, other: &DukaInt) -> bool {
        self.0.eq(&(*other as DukaFloat))
    }
}
impl PartialOrd<DukaInt> for DukaFloatWrapper {
    fn partial_cmp(&self, other: &DukaInt) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&(*other as DukaFloat))
    }
}

impl From<DukaFloatWrapper> for DukaFloat {
    fn from(value: DukaFloatWrapper) -> Self {
        value.0
    }
}
impl From<DukaFloat> for DukaFloatWrapper {
    fn from(value: DukaFloat) -> Self {
        Self(value)
    }
}

impl Display for DukaFloatWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Eq for DukaFloatWrapper {}
impl Hash for DukaFloatWrapper {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        if self.0 == 0f64 {
            0
        } else if self.0.is_nan() {
            f64::NAN.to_bits()
        } else {
            self.0.to_bits()
        }
        .hash(state)
    }
}

impl<'de> Deserialize<'de> for DukaFloatWrapper {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self(f64::deserialize(deserializer)?))
    }
}
impl Serialize for DukaFloatWrapper {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// Duka's table type
#[derive(Debug, PartialEq, Clone, Default, Serialize, Deserialize)]
pub struct ArrayMap<T>
where
    T: Hash + Eq + Clone,
{
    pub inner: HashMap<T, T>,
}

impl<T: Hash + Eq + Clone> Display for ArrayMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Table<const>[len={}]", self.len())
    }
}

impl ArrayMap<ConstValue> {
    #[inline]
    pub const fn is_const(&self) -> bool {
        true
    }
}

impl<T> Hash for ArrayMap<T>
where
    T: Hash + Eq + Clone,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.iter().for_each(|(k, v)| {
            k.hash(state);
            v.hash(state);
        });
    }
}

impl<T> ArrayMap<T>
where
    T: Hash + Eq + Clone,
{
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttrConstValue {
    StaticString(Cow<'static, str>),
    Value(ConstValue),
}
impl Display for AttrConstValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaticString(s) => write!(f, "\"{s}\""),
            Self::Value(v) => write!(f, "{v}"),
        }
    }
}
impl Default for AttrConstValue {
    fn default() -> Self {
        Self::Value(Default::default())
    }
}
impl AttrConstValue {
    pub fn type_of(&self) -> Type {
        match self {
            Self::StaticString(_) => Type::String,
            Self::Value(v) => v.type_of(),
        }
    }
    pub fn eval_to_str(&self) -> Option<&str> {
        match self {
            Self::StaticString(s) => Some(s),
            Self::Value(ConstValue::String(s)) => std::str::from_utf8(s).ok(),
            _ => None,
        }
    }
    pub const fn eval_to_bool(&self) -> bool {
        match self {
            Self::StaticString(_) => true,
            Self::Value(v) => v.eval_to_bool(),
        }
    }
    #[inline(always)]
    pub fn to_const_value(&self) -> ConstValue {
        self.clone().into_const_value()
    }
    #[inline(always)]
    pub fn into_const_value(self) -> ConstValue {
        self.into()
    }
}
impl From<AttrConstValue> for ConstValue {
    fn from(value: AttrConstValue) -> Self {
        match value {
            AttrConstValue::StaticString(s) => {
                Self::String(s.to_string().into_bytes().into_boxed_slice())
            }
            AttrConstValue::Value(v) => v,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Info, Serialize, Deserialize)]
#[idcard(u8)]
pub enum AttrValue {
    Single(AttrConstValue),
    Many(Vec<AttrConstValue>),
}

impl Default for AttrValue {
    fn default() -> Self {
        Self::Single(Default::default())
    }
}

impl std::ops::Add for AttrValue {
    type Output = AttrValue;
    fn add(self, rhs: Self) -> Self::Output {
        self.merge_into(rhs)
    }
}

impl AttrValue {
    pub fn merge_into(self, other: AttrValue) -> Self {
        AttrValue::Many(match (self, other) {
            (AttrValue::Single(a), AttrValue::Single(b)) => vec![a, b],
            (AttrValue::Single(a), AttrValue::Many(bs)) => [vec![a], bs].concat(),
            (AttrValue::Many(mut sa), AttrValue::Single(b)) => {
                sa.push(b);
                sa
            }
            (AttrValue::Many(mut sa), AttrValue::Many(mut bs)) => {
                sa.append(&mut bs);
                sa
            }
        })
    }
    #[inline]
    pub const fn eval_to_bool(&self) -> bool {
        match self {
            Self::Single(v) => v.eval_to_bool(),
            _ => true,
        }
    }

    pub fn eval_to_str(&self) -> Option<&str> {
        match self {
            Self::Single(v) => v.eval_to_str(),
            Self::Many(vs) => vs.first()?.eval_to_str(),
        }
    }
}

/// ### Compile time
/// Value type of duka language
#[derive(Debug, Clone, PartialEq, Info, Default, serde::Serialize, serde::Deserialize)]
#[shy]
#[idcard(u8)]
pub enum ConstValue {
    #[default]
    Nil,
    #[tag(number)]
    Int(DukaInt),
    #[tag(number)]
    Float(DukaFloat),
    Bool(bool),
    String(Box<[u8]>),
}

impl From<String> for ConstValue {
    fn from(value: String) -> Self {
        ConstValue::String(value.as_bytes().into())
    }
}

impl ConstValue {
    #[inline]
    pub const fn into_attr(self) -> AttrConstValue {
        AttrConstValue::Value(self)
    }

    pub const fn type_of(&self) -> Type {
        match self {
            ConstValue::Nil => Type::Nil,
            ConstValue::Int(_) => Type::Int,
            ConstValue::Float(_) => Type::Float,
            ConstValue::Bool(_) => Type::Bool,
            ConstValue::String(_) => Type::String,
        }
    }

    #[inline]
    pub const fn is_const(&self) -> bool {
        true
    }

    /// Duka truthiness, only `nil` and `false` are falsy
    ///
    /// ```
    /// use duka_shared::value::ConstValue;
    ///
    /// assert!(!ConstValue::Nil.eval_to_bool());
    /// assert!(!ConstValue::Bool(false).eval_to_bool());
    /// assert!(ConstValue::Bool(true).eval_to_bool());
    /// assert!(ConstValue::Int(0).eval_to_bool()); // 0 stays truthy
    /// assert!(ConstValue::String(Vec::new().into()).eval_to_bool()); // and so does ""
    /// ```
    #[inline]
    pub const fn eval_to_bool(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Nil => false,
            _ => true,
        }
    }

    #[inline]
    pub fn get_string(&self) -> Option<&str> {
        if let ConstValue::String(s) = self {
            str::from_utf8(s).ok()
        } else {
            None
        }
    }
}

// we are sure that NaN == NaN
impl Eq for ConstValue {}
impl Hash for ConstValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ConstValue::Nil => (),
            ConstValue::Bool(b) => b.hash(state),

            ConstValue::Int(i) => i.hash(state),
            ConstValue::Float(f) => if *f == 0f64 {
                0
            } else if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            }
            .hash(state),

            ConstValue::String(s) => s.hash(state),
        }
    }
}

impl Display for ConstValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstValue::Nil => write!(f, "nil"),
            ConstValue::Int(i) => write!(f, "{}", i),
            ConstValue::Float(fl) => write!(f, "{}", fl),
            ConstValue::String(s) => {
                let c = str::from_utf8(s).map_err(|_| std::fmt::Error)?;
                write!(f, "{c}")
            }
            ConstValue::Bool(b) => write!(f, "{}", b),
        }
    }
}

impl<T: Into<ConstValue>> From<Option<T>> for ConstValue {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(v) => v.into(),
            None => Self::Nil,
        }
    }
}
impl From<bool> for ConstValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<DukaInt> for ConstValue {
    fn from(value: DukaInt) -> Self {
        Self::Int(value)
    }
}
impl From<DukaFloat> for ConstValue {
    fn from(value: DukaFloat) -> Self {
        Self::Float(value)
    }
}
