use std::{fmt::Display, rc::Rc};

use crate::backend::vm::ExeState;

const SHORT_STR_LEN: usize = 14;
const MID_STR_LEN: usize = 47;

/// accpeting mutable state of running vm, returning count of result
pub type DukaFunc = fn(&mut ExeState) -> i32;

/// Value type of duka language
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Int(i64),
    Float(f64),
    Bool(bool),
    Func(DukaFunc),

    // String们是不可变的 所以用这些效率高于String
    // Strings分三类 通过长度自动分配
    ShortStr(u8, [u8; SHORT_STR_LEN]),
    MidStr(Rc<(u8, [u8; MID_STR_LEN])>),
    LongStr(Rc<str>),
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Nil => write!(f, "none"),
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::ShortStr(..) | Value::MidStr(..) | Value::LongStr(..) => {
                let c: &str = self.into();
                write!(f, "{}", c.to_string())
            }
            Value::Bool(b) => write!(f, "{}", b),
            _ => write!(f, "unknown"),
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

impl<'a> From<&'a Value> for &'a str {
    /// ## we must ensure that val is valid string value
    fn from(val: &'a Value) -> Self {
        match val {
            Value::ShortStr(len, buf) => str::from_utf8(&buf[..*len as usize]).unwrap(),
            Value::MidStr(rc) => str::from_utf8(&rc.1[..rc.0 as usize]).unwrap(),
            Value::LongStr(rc) => rc,
            _ => panic!("Invalid string"),
        }
    }
}
