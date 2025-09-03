use duka_macros::Info;

use crate::error::DukaLexerError;
use core::str;
use std::{cell::RefCell, collections::HashMap, fmt::Display, hash::Hash, rc::Rc};

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

/// accpeting mutable state of running vm, returning count of result
// pub type DukaFunc = fn(&mut Box<dyn DukaRuntime>) -> i32; moved to backend
pub type DukaInt = i64;
pub type DukaFloat = f64;

/// Duka's table type
#[derive(Debug, PartialEq, Clone)]
pub struct ArrayMap<T>
where
    T: Hash + Eq + Clone,
{
    pub array: Vec<T>,
    pub map: HashMap<T, T>,
}

impl ArrayMap<ConstValue> {
    #[inline]
    pub const fn is_const(&self) -> bool {
        true
    }
}

impl<T> ArrayMap<T>
where
    T: Hash + Eq + Clone,
{
    pub fn new() -> Self {
        Self {
            array: vec![],
            map: HashMap::new(),
        }
    }
}

/// ### Compile time
/// Value type of duka language
#[derive(Debug, Clone, PartialEq, Info)]
#[shy]
pub enum ConstValue {
    Nil,
    Int(DukaInt),
    Float(DukaFloat),
    Bool(bool),
    // this should have a better way to handle it
    ConstTable(Rc<RefCell<ArrayMap<Self>>>),
    String(Vec<u8>),
}

impl ConstValue {
    #[inline(always)]
    pub fn new_table() -> Self {
        Self::ConstTable(Rc::new(RefCell::new(ArrayMap::new())))
    }

    #[inline]
    pub fn is_const(&self) -> bool {
        true
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

            ConstValue::ConstTable(t) => Rc::as_ptr(t).hash(state),
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
            ConstValue::ConstTable(t) => write!(f, "table {:?}", t.as_ptr()),
            // Value::Func(_) => write!(f, "function"),
        }
    }
}
