use duka_macros::Info;
use duka_shared::value::ConstValue;
use duka_shared::value::{DukaFloat, DukaInt};
use gc::{Finalize, Gc, GcCell};
use gc_derive::{Finalize, Trace};
use std::collections::HashMap;
use std::hash::Hash;

use crate::error::DukaRuntimeError;
use crate::instructions::Instruction;
use crate::vm::coroutine::CoState;

/// 闭包
#[derive(Debug, Clone)]
pub struct Closure {
    pub proto: DukaProto,
    pub upvalues: Vec<Upvalue>,
}

/// 捕获值
#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub enum Upvalue {
    Open(usize),
    Closed(RuntimeValue),
}

/// 函数原型
#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub struct DukaProto {
    pub upvalues: Vec<Upvalue>,
    #[unsafe_ignore_trace]
    pub constants: Vec<RuntimeValue>,
    #[unsafe_ignore_trace]
    pub instructions: Vec<Instruction>,
    pub nested_protos: Vec<DukaProto>,

    pub param_count: usize,
    pub has_vararg: bool,

    pub debug_name: Option<String>,
}

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

#[derive(Debug, Clone, PartialEq, Trace)]
pub struct RuntimeDukaTable {
    pub array: Vec<RuntimeValue>,
    pub map: HashMap<RuntimeValue, RuntimeValue>,
    pub metatable: Option<Gc<GcCell<Self>>>,
}
impl Finalize for RuntimeDukaTable {
    fn finalize(&self) {
        // todo
    }
}

/// # 值的数量
pub enum ValueCount {
    /// *`0` in number representing*
    VarArg,
    /// *`n + 1` in number representing*
    Exact(usize),
}
// only used for instruction
impl Into<ValueCount> for u32 {
    #[inline]
    fn into(self) -> ValueCount {
        if self == 0 {
            ValueCount::VarArg
        } else {
            ValueCount::Exact(self as usize - 1)
        }
    }
}
impl Into<ValueCount> for usize {
    #[inline]
    fn into(self) -> ValueCount {
        if self == 0 {
            ValueCount::VarArg
        } else {
            ValueCount::Exact(self - 1)
        }
    }
}
// only used for API function or coroutine returning
impl Into<usize> for ValueCount {
    #[inline]
    fn into(self) -> usize {
        match self {
            ValueCount::VarArg => 0,
            ValueCount::Exact(n) => n + 1,
        }
    }
}
impl Into<u8> for ValueCount {
    #[inline]
    fn into(self) -> u8 {
        match self {
            ValueCount::VarArg => 0,
            ValueCount::Exact(n) => (n + 1) as u8,
        }
    }
}

pub type RustFunction = fn(&mut CoState) -> Result<ValueCount, DukaRuntimeError>;

#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub struct RustClosure {
    #[unsafe_ignore_trace]
    pub func: RustFunction,
}
impl RustClosure {
    pub fn from_func(func: RustFunction) -> Self {
        Self { func }
    }
}

/// ### Runtime
/// Value type of duka language
#[derive(Debug, Clone, PartialEq, Info, Trace, Finalize)]
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
    MediumString(Gc<(u8, [u8; MID_STR_LEN])>),
    #[tag(string)]
    #[tag(collectable)]
    LongString(Gc<String>),
    #[tag(collectable)]
    Table(Gc<GcCell<RuntimeDukaTable>>),
    #[tag(collectable)]
    #[tag(user)]
    UserData(),

    // Pointer:
    #[tag(user)]
    LightUserData(),

    // Function:
    #[tag(function)]
    #[tag(collectable)]
    UserFunc(Gc<DukaProto>),
    #[tag(function)]
    NativeFunc(RustClosure),
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
            Self::UserFunc(..) => todo!(),
            Self::NativeFunc(..) => todo!(),
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
                        RuntimeValue::MediumString(Gc::new((len as u8, buffer)))
                    }
                    // it is safe because we have checked it when parsing
                    _ => RuntimeValue::LongString(Gc::new(
                        String::from_utf8(s).expect("INVALID UTF8"),
                    )),
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
