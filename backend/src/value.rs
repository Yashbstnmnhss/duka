use duka_macros::Info;
use duka_shared::value::ConstValue;
use duka_shared::value::{DukaFloat, DukaInt};
use gc::{Finalize, Gc, GcCell};
use gc_derive::{Finalize, Trace};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;

use crate::error::DukaRuntimeError;
use crate::instructions::Instruction;
use crate::vm::coroutine::{CoState, CoroutineID};
use crate::vm::frame::Stack;

/// 捕获值
#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub enum UpValue {
    Open(usize),
    Closed(RuntimeValue),
}

impl UpValue {
    pub fn get<'a>(&'a self, stack: &'a Stack) -> &'a RuntimeValue {
        match self {
            Self::Open(i) => &stack.get(*i).expect("NO UPVAL"),
            Self::Closed(rv) => rv,
        }
    }
    pub fn set(&mut self, stack: &mut Stack, v: RuntimeValue) {
        match self {
            Self::Open(i) => stack[*i] = v,
            Self::Closed(rv) => *rv = v,
        }
    }
}

/// 函数原型
#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub struct DukaProto {
    pub upvalues: Vec<UpValue>,
    pub constants: Vec<RuntimeValue>,
    #[unsafe_ignore_trace]
    pub instructions: Vec<Instruction>,
    pub nested_protos: Vec<DukaProto>,

    pub param_count: usize,
    pub has_var_arg: bool,

    pub debug_name: Option<String>,
}
impl Display for DukaProto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}([{}]{}, using {} upvalues) with {} constants, {} instructions, {} nested prototypes",
            self.debug_name
                .as_ref()
                .map(|v| v.as_str())
                .unwrap_or("<Prototype>"),
            self.param_count,
            if self.has_var_arg { ", ..." } else { "" },
            self.upvalues.len(),
            self.constants.len(),
            self.instructions.len(),
            self.nested_protos.len(),
        )
    }
}

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

#[derive(Debug, Clone, PartialEq, Trace)]
pub struct RuntimeDukaTable {
    pub array: Vec<RuntimeValue>,
    pub map: HashMap<RuntimeValue, RuntimeValue>,
    pub metatable: Option<Gc<GcCell<Self>>>,
}
impl RuntimeDukaTable {
    #[inline]
    pub fn new(narray: usize, nmap: usize) -> Self {
        Self {
            array: Vec::with_capacity(narray),
            map: HashMap::with_capacity(nmap),
            metatable: None,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.array.len() + self.map.len()
    }
    pub fn array_push(&mut self, at: usize, item: RuntimeValue) {
        match self.array.len().cmp(&at) {
            std::cmp::Ordering::Less => panic!(),
            std::cmp::Ordering::Equal => self.array.push(item),
            std::cmp::Ordering::Greater => self.array[at] = item,
        }
    }
}
impl Finalize for RuntimeDukaTable {
    fn finalize(&self) {
        // todo
    }
}

#[derive(Debug, Clone, PartialEq)]
/// # 值的数量
pub enum ValueCount {
    /// *`0` in number representing*
    VarArg,
    /// *`n + 1` in number representing*
    Exact(usize),
}
impl ValueCount {
    pub fn to_index(&self, stack_len: usize) -> usize {
        match self {
            ValueCount::VarArg => stack_len,
            ValueCount::Exact(n) => *n,
        }
    }
}
// only used for instruction
impl From<u32> for ValueCount {
    #[inline]
    fn from(val: u32) -> Self {
        if val == 0 {
            ValueCount::VarArg
        } else {
            ValueCount::Exact(val as usize - 1)
        }
    }
}
// only used for API function or coroutine returning
impl From<ValueCount> for usize {
    #[inline]
    fn from(val: ValueCount) -> Self {
        match val {
            ValueCount::VarArg => 0,
            ValueCount::Exact(n) => n + 1,
        }
    }
}
impl From<u8> for ValueCount {
    #[inline]
    fn from(val: u8) -> Self {
        match val {
            0 => ValueCount::VarArg,
            n => ValueCount::Exact((n - 1) as usize),
        }
    }
}

/// ### Closure of duka function
/// with prototype and references to upvalues it has captured
#[derive(Debug, Clone, PartialEq, Trace, Finalize)]
pub struct DukaClosure {
    pub func: Gc<DukaProto>,
    pub upvalues: Vec<Gc<GcCell<UpValue>>>,
}
impl DukaClosure {
    pub fn new(func: Gc<DukaProto>) -> Self {
        Self {
            func,
            upvalues: vec![],
        }
    }
}

/// ### Closure for Rust function
/// with function pointer itself
#[derive(Finalize, Trace)]
pub struct RustClosure {
    #[unsafe_ignore_trace]
    pub func: Box<dyn FnMut(&mut CoState) -> Result<ValueCount, DukaRuntimeError>>,
}
impl RustClosure {
    #[inline(always)]
    pub fn returning<const C: usize, F>(mut f: F) -> Self
    where
        F: FnMut(&mut CoState) -> Result<(), DukaRuntimeError> + 'static,
    {
        Self::with_count(move |c| {
            f(c)?;
            Ok(ValueCount::Exact(C))
        })
    }
    #[inline(always)]
    pub fn nonreturn<F>(f: F) -> Self
    where
        F: FnMut(&mut CoState) -> Result<(), DukaRuntimeError> + 'static,
    {
        Self::returning::<0, _>(f)
    }
    #[inline(always)]
    pub fn with_count<F>(f: F) -> Self
    where
        F: FnMut(&mut CoState) -> Result<ValueCount, DukaRuntimeError> + 'static,
    {
        Self { func: Box::new(f) }
    }
}
impl PartialEq for RustClosure {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(&*self.func, &*other.func)
    }
}
impl Debug for RustClosure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustClosure")
            .field("func", &"FnMut(...)")
            .finish()
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
    #[tag(coroutine)]
    Coroutine(CoroutineID),

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
    UserFunc(Gc<DukaClosure>),
    #[tag(function)]
    NativeFunc(Gc<GcCell<RustClosure>>),
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
            Self::Coroutine(id) => id.hash(state),
            Self::ShortString(l, b) => b[..*l as usize].hash(state),
            Self::MediumString(s) => s.1[..s.0 as usize].hash(state),
            Self::LongString(s) => s.hash(state),
            Self::Table(t) => Gc::as_ptr(t).hash(state),
            Self::UserData() => todo!(),
            Self::LightUserData() => todo!(),
            Self::UserFunc(proto) => Gc::as_ptr(proto).hash(state),
            Self::NativeFunc(rust) => Gc::as_ptr(rust).hash(state),
            //Self::UserClosure(cl) => Gc::as_ptr(cl).hash(state),
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
impl From<RustClosure> for RuntimeValue {
    fn from(value: RustClosure) -> Self {
        RuntimeValue::NativeFunc(Gc::new(GcCell::new(value)))
    }
}
impl From<DukaClosure> for RuntimeValue {
    fn from(value: DukaClosure) -> Self {
        RuntimeValue::UserFunc(Gc::new(value))
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
