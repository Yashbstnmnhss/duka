use duka_macros::Info;
use serde::{Deserialize, Serialize};

use core::str;
use std::{collections::HashMap, fmt::Display, hash::Hash};

use crate::dtype::Type;

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

/// accepting mutable state of running vm, returning count of result
// pub type DukaFunc = fn(&mut Box<dyn DukaRuntime>) -> i32; moved to backend

/// integer type
pub type DukaInt = i64;
/// float type
pub type DukaFloat = f64;

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
    pub fn len(&self) -> usize {
        self.inner.len()
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
    /// ~~this could have a better way to handle it~~
    ConstTable(Box<ArrayMap<Self>>),
    String(Box<[u8]>),
}

impl From<String> for ConstValue {
    fn from(value: String) -> Self {
        ConstValue::String(value.as_bytes().into())
    }
}

impl ConstValue {
    pub const fn type_of(&self) -> Type {
        match self {
            ConstValue::Nil => Type::Nil,
            ConstValue::Int(_) => Type::Int,
            ConstValue::Float(_) => Type::Float,
            ConstValue::Bool(_) => Type::Bool,
            ConstValue::ConstTable(_) => Type::Table,
            ConstValue::String(_) => Type::String,
        }
    }
    #[inline(always)]
    pub fn new_table() -> Self {
        Self::ConstTable(Box::new(ArrayMap::new()))
    }

    #[inline]
    pub fn is_const(&self) -> bool {
        true
    }

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

            ConstValue::ConstTable(t) => t.hash(state),
            // cast to function pointer then get hash
            // Value::Func(f) => (*f as *const usize).hash(state),
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
            ConstValue::ConstTable(t) => write!(f, "{t}"),
            // Value::Func(_) => write!(f, "function"),
        }
    }
}
