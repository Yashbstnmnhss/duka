use duka_macros::{Info, binops};
use duka_shared::value::ConstValue;
use duka_shared::value::{DukaFloat, DukaInt};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use crate::error::DukaRuntimeError;

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDukaTable {
    pub array: Vec<RuntimeValue>,
    pub map: HashMap<RuntimeValue, RuntimeValue>,
    pub metatable: Option<Rc<RefCell<Self>>>,
}

/// ### Runtime
/// Value type of duka language
#[derive(Debug, Clone, PartialEq, Info)]
#[shy]
pub enum RuntimeValue {
    // Primitive:
    Nil,
    #[tag(number)]
    Int(DukaInt),
    #[tag(number)]
    Float(DukaFloat),
    Bool(bool),
    #[tag(string)]
    ShortString(u8, [u8; SHORT_STR_LEN]),

    // Collectable:
    #[tag(string)]
    #[tag(collectable)]
    MediumString(Rc<(u8, [u8; MID_STR_LEN])>),
    #[tag(string)]
    #[tag(collectable)]
    LongString(Rc<str>),
    #[tag(collectable)]
    Table(Rc<RefCell<RuntimeDukaTable>>),
    #[tag(collectable)]
    #[tag(user)]
    UserData(),

    // Pointer:
    #[tag(user)]
    LightUserData(),

    // Function:
    #[tag(function)]
    UserFunc(),
    #[tag(function)]
    NativeFunc(),
}
impl Eq for RuntimeValue {}
impl Hash for RuntimeValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Nil => (),
            Self::Bool(b) => b.hash(state),
            Self::Int(i) => i.hash(state),
            Self::Float(f) => if *f == 0f64 {
                0
            } else if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            }
            .hash(state),
            Self::ShortString(l, b) => b[..*l as usize].hash(state),
            Self::MediumString(s) => s.1[..s.0 as usize].hash(state),
            Self::LongString(s) => s.hash(state),
            Self::Table(t) => todo!(),
            Self::UserData() => todo!(),
            Self::LightUserData() => todo!(),
            Self::UserFunc() => todo!(),
            Self::NativeFunc() => todo!(),
            // cast to function pointer then get hash
            // Value::Func(f) => (*f as *const usize).hash(state),
        }
    }
}
impl From<ConstValue> for RuntimeValue {
    fn from(value: ConstValue) -> Self {
        match value {
            ConstValue::Nil => RuntimeValue::Nil,
            ConstValue::Bool(b) => RuntimeValue::Bool(b),
            ConstValue::Int(i) => RuntimeValue::Int(i),
            ConstValue::Float(f) => RuntimeValue::Float(f),
            ConstValue::ConstTable(t) => todo!(),
            ConstValue::String(s) => {
                let len = s.len();
                match len {
                    ..=SHORT_STR_LEN => {
                        let mut buffer = [0; SHORT_STR_LEN];
                        buffer[..len].copy_from_slice(&s);
                        RuntimeValue::ShortString(len as u8, buffer)
                    }
                    ..=MID_STR_LEN => {
                        let mut buffer = [0; MID_STR_LEN];
                        buffer[..len].copy_from_slice(&s);
                        RuntimeValue::MediumString(Rc::new((len as u8, buffer)))
                    }
                    // it is safe because we have checked it when parsing
                    _ => RuntimeValue::LongString(Rc::from(str::from_utf8(&s).unwrap())),
                }
            }
        }
    }
}

impl RuntimeValue {
    pub fn eval_to_bool(&self) -> bool {
        match self {
            Self::Nil => false,
            Self::Bool(b) => *b,
            _ => true,
        }
    }
    pub fn eval_to_float(&self) -> Result<DukaFloat, ()> {
        Ok(match self {
            Self::Int(i) => *i as DukaFloat,
            Self::Float(f) => *f,
            Self::Bool(b) => b.then_some(1).unwrap_or(0) as DukaFloat,
            _ => return Err(()),
        })
    }
    pub fn eval_to_int(&self) -> Result<DukaInt, ()> {
        Ok(match self {
            Self::Int(i) => *i,
            Self::Float(f) => *f as DukaInt,
            Self::Bool(b) => b.then_some(1).unwrap_or(0),
            _ => return Err(()),
        })
    }
    pub const fn type_of(&self) -> &'static str {
        if self.is_string() {
            "string"
        } else if self.is_function() {
            "function"
        } else {
            match self {
                Self::Bool(..) => "bool",
                Self::Float(..) => "float",
                Self::Int(..) => "int",
                Self::Nil => "nil",
                Self::Table(..) => "table",
                Self::UserData() => "userdata",
                Self::LightUserData() => "lightuserdata",
                _ => unreachable!(),
            }
        }
    }
}
