use crate::error::DukaLexerError;
use core::str;
use std::{cell::RefCell, collections::HashMap, fmt::Display, hash::Hash, rc::Rc};

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

/// accpeting mutable state of running vm, returning count of result
pub type DukaFunc = fn(&mut Vec<i32>) -> i32;
pub type DukaInt = i64;
pub type DukaFloat = f64;

/// Duka's table type
#[derive(Debug, PartialEq)]
pub struct DukaTable {
    pub array: Vec<Value>,
    pub map: HashMap<Value, Value>,
}

impl DukaTable {
    pub fn new() -> Self {
        Self {
            array: vec![],
            map: HashMap::new(),
        }
    }
    #[inline]
    pub fn is_const(&self) -> bool {
        self.array.iter().all(|v| v.is_const())
            && self.map.iter().all(|(k, v)| k.is_const() && v.is_const())
    }
}

/// Value type of duka language
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Int(DukaInt),
    Float(DukaFloat),
    Bool(bool),

    Table(Rc<RefCell<DukaTable>>),

    Func(DukaFunc),

    // String们是不可变的 所以用这些效率高于String
    // Strings分三类 通过长度自动分配
    ShortStr(u8, [u8; SHORT_STR_LEN]),
    MidStr(Rc<(u8, [u8; MID_STR_LEN])>),
    LongStr(Rc<str>),
}

impl Value {
    #[inline(always)]
    pub fn new_table() -> Self {
        Self::Table(Rc::new(RefCell::new(DukaTable::new())))
    }

    #[inline]
    pub fn is_const(&self) -> bool {
        match self {
            Value::Table(t) => {
                let b = t.borrow();
                !(b.array.iter().any(|i| !i.is_const())
                    || b.map.iter().any(|(k, v)| !k.is_const() || !v.is_const()))
            }
            Value::Func(_) => false,
            _ => true,
        }
    }
    #[inline]
    pub const fn is_string(&self) -> bool {
        matches!(
            self,
            Self::ShortStr(..) | Self::MidStr(..) | Self::LongStr(..)
        )
    }
}

// we are sure that NaN == NaN
impl Eq for Value {}
impl Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Value::Nil => (),
            Value::Bool(b) => b.hash(state),

            Value::Int(i) => i.hash(state),
            Value::Float(f) => if *f == 0f64 {
                0
            } else if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            }
            .hash(state),

            Value::ShortStr(l, b) => b[..*l as usize].hash(state),
            Value::MidStr(s) => s.1[..s.0 as usize].hash(state),
            Value::LongStr(s) => s.hash(state),

            Value::Table(t) => Rc::as_ptr(t).hash(state),
            // cast to function pointer then get hash
            Value::Func(f) => (*f as *const usize).hash(state),
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::ShortStr(..) | Value::MidStr(..) | Value::LongStr(..) => {
                let c: &str = self.into();
                write!(f, "{}", c.to_string())
            }
            Value::Bool(b) => write!(f, "{}", b),
            Value::Table(t) => write!(f, "table {:?}", t.as_ptr()),
            Value::Func(_) => write!(f, "function"),
        }
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        let len = value.len();
        match len {
            ..=SHORT_STR_LEN => {
                let mut buffer = [0; SHORT_STR_LEN];
                buffer[..len].copy_from_slice(value.as_bytes());
                Value::ShortStr(len as u8, buffer)
            }
            ..=MID_STR_LEN => {
                let mut buffer = [0; MID_STR_LEN];
                buffer[..len].copy_from_slice(value.as_bytes());
                Value::MidStr(Rc::new((len as u8, buffer)))
            }
            _ => Value::LongStr(Rc::from(value.as_str())),
        }
    }
}

impl From<&Value> for String {
    fn from(value: &Value) -> Self {
        let str: &str = value.into();
        str.to_owned()
    }
}

impl<'a> From<&'a Value> for &'a str {
    /// ## we must ensure that val is valid utf8 string value
    fn from(val: &'a Value) -> Self {
        // checked when call this method
        // when i cannot ensure i wont call it
        assert!(val.is_string());
        match val {
            Value::ShortStr(len, buf) => {
                str::from_utf8(&buf[..*len as usize]).expect("not valid utf8")
            }
            Value::MidStr(rc) => str::from_utf8(&rc.1[..rc.0 as usize]).expect("not valid utf8"),
            Value::LongStr(rc) => rc,
            _ => panic!("Invalid string"),
        }
    }
}

impl TryFrom<&Vec<u8>> for Value {
    type Error = DukaLexerError;

    fn try_from(value: &Vec<u8>) -> Result<Value, Self::Error> {
        let len = value.len();
        match len {
            ..=SHORT_STR_LEN => {
                let mut buffer = [0; SHORT_STR_LEN];
                buffer[..len].copy_from_slice(value);
                Ok(Value::ShortStr(len as u8, buffer))
            }
            ..=MID_STR_LEN => {
                let mut buffer = [0; MID_STR_LEN];
                buffer[..len].copy_from_slice(value);
                Ok(Value::MidStr(Rc::new((len as u8, buffer))))
            }
            _ => Ok(Value::LongStr(Rc::from(
                str::from_utf8(value).map_err(|_| DukaLexerError::InvalidUtf8)?,
            ))),
        }
    }
}
