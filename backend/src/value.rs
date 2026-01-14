use duka_macros::Info;
use duka_shared::constants::ctype;
use duka_shared::value::ConstValue;
use duka_shared::value::{DukaFloat, DukaInt};
use gc::{Finalize, Gc, GcCell, Trace, Tracer};
use std::any::Any;
// gc_derive removed during migration; Trace/Finalize will be implemented by hand where needed.
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;

use crate::error::DukaRuntimeError;
use crate::instructions::Instruction;
use crate::vm::coroutine::{CoState, CoroutineID};
use crate::vm::frame::Stack;

/// 捕获值
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct DukaProto {
    pub upvalues: Vec<UpValue>,
    pub constants: Vec<duka_shared::value::ConstValue>,
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

#[derive(Debug, Clone, PartialEq)]
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
            std::cmp::Ordering::Greater => self.array[at] = item,
            // std::cmp::Ordering::Less,
            // std::cmp::Ordering::Equal
            _ => self.array.push(item),
        }
    }
}
impl Finalize for RuntimeDukaTable {
    fn finalize(&self) {
        // todo
    }
}

impl Trace for RuntimeDukaTable {
    fn trace(&self, tracer: &mut Tracer) {
        for v in &self.array {
            v.trace(tracer);
        }
        for (k, v) in &self.map {
            k.trace(tracer);
            v.trace(tracer);
        }
        if let Some(mt) = &self.metatable {
            tracer.mark(mt);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// # 值的数量
pub enum ValueCount {
    /// `VarArg`: *`0` in number representing*
    VarArg,
    /// `Exact(n)`: *`n + 1` in number representing*
    Exact(usize),
}
impl ValueCount {
    /// Convert `ValueCount` to its index in given stack
    pub const fn to_index(&self, stack_len: usize) -> usize {
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

#[derive(Debug)]
pub struct UserData {
    pub payload: Box<dyn Any + Send + Sync>,
    pub finalizer: Option<Gc<DukaClosure>>,
}

impl Trace for UserData {
    fn trace(&self, tracer: &mut Tracer) {
        if let Some(inner) = self.finalizer {
            inner.trace(tracer);
        }
    }
}
impl Finalize for UserData {
    fn finalize(&self) {
        // We dont run finalizer here, because if so it is so complex
    }
}

/// ### Closure of duka function
/// with prototype and references to upvalues it has captured
#[derive(Debug, Clone, PartialEq)]
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
#[derive()]
pub struct RustClosure {
    pub func: Box<dyn FnMut(&mut CoState) -> Result<ValueCount, DukaRuntimeError>>,
}
impl RustClosure {
    #[inline(always)]
    pub fn returning<const C: usize, F>(mut f: F) -> Self
    where
        F: FnMut(&mut CoState) -> Result<(), DukaRuntimeError> + 'static,
    {
        Self::returns(move |c| {
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
    pub fn returns<F>(f: F) -> Self
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

impl Finalize for RustClosure {
    fn finalize(&self) {}
}

impl Trace for RustClosure {
    fn trace(&self, _tracer: &mut Tracer) {
        // RustClosure No Need
    }
}

/// Wrapper for MediumString
#[derive(Debug, Clone, PartialEq)]
pub struct MediumStringInner(pub u8, pub [u8; MID_STR_LEN]);

impl Finalize for MediumStringInner {
    fn finalize(&self) {}
}

impl Trace for MediumStringInner {
    fn trace(&self, _tracer: &mut Tracer) {}
}

/// Wrapper for String, used to implement GC
#[derive(Debug, Clone, PartialEq)]
pub struct HeapString(pub String);

impl Finalize for HeapString {
    fn finalize(&self) {}
}

impl Trace for HeapString {
    fn trace(&self, _tracer: &mut Tracer) {}
}

impl std::hash::Hash for HeapString {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

/// ### Runtime
/// Value type of duka language
#[derive(Debug, Clone, PartialEq, Info, Default)]
#[shy]
#[idcard(u8)]
pub enum RuntimeValue {
    // Primitive:
    #[default]
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
    MediumString(Gc<MediumStringInner>),
    #[tag(string)]
    #[tag(collectable)]
    LongString(Gc<HeapString>),
    #[tag(collectable)]
    Table(Gc<GcCell<RuntimeDukaTable>>),
    #[tag(collectable)]
    #[tag(user)]
    UserData(Gc<UserData>),

    // // Pointer:
    // #[tag(user)]
    // LightUserData(),

    // Function:
    #[tag(function)]
    #[tag(collectable)]
    UserFunc(Gc<DukaClosure>),
    #[tag(function)]
    #[tag(collectable)]
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
            Self::UserData(ud) => todo!(),
            //Self::LightUserData() => todo!(),
            Self::UserFunc(proto) => Gc::as_ptr(proto).hash(state),
            Self::NativeFunc(rust) => Gc::as_ptr(rust).hash(state),
            //Self::UserClosure(cl) => Gc::as_ptr(cl).hash(state),
            // cast to function pointer then get hash
            // Value::Func(f) => (*f as *const usize).hash(state),
        }
    }
}

impl RuntimeValue {
    pub(crate) fn const_str_2_runtime(str: &'static str) -> RuntimeValue {
        let len = str.len();
        assert!(len <= SHORT_STR_LEN);
        let mut buffer = [0; SHORT_STR_LEN];
        buffer[..len].copy_from_slice(str.as_bytes());
        RuntimeValue::ShortString(len as u8, buffer)
    }
    /// Convert a compile-time `ConstValue` into a runtime `RuntimeValue` using
    /// the provided `heap` for any GC allocations
    pub fn from_const(heap: &mut gc::Heap, value: ConstValue) -> Self {
        match value {
            ConstValue::Nil => RuntimeValue::Nil,
            ConstValue::Bool(b) => RuntimeValue::Bool(b),
            ConstValue::Int(i) => RuntimeValue::Int(i),
            ConstValue::Float(f) => RuntimeValue::Float(f),
            ConstValue::ConstTable(t) => {
                // convert compile-time table into a runtime table
                let borrowed = t.borrow();
                let mut rt = RuntimeDukaTable::new(borrowed.array.len(), borrowed.map.len());
                for v in &borrowed.array {
                    rt.array.push(RuntimeValue::from_const(heap, v.clone()));
                }
                for (k, v) in &borrowed.map {
                    let rk = RuntimeValue::from_const(heap, k.clone());
                    let rv = RuntimeValue::from_const(heap, v.clone());
                    rt.map.insert(rk, rv);
                }
                RuntimeValue::Table(heap.alloc(GcCell::new(rt)))
            }
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
                        RuntimeValue::MediumString(heap.alloc(MediumStringInner(len as u8, buffer)))
                    }
                    _ => RuntimeValue::LongString(
                        heap.alloc(HeapString(String::from_utf8(s).expect("INVALID UTF8"))),
                    ),
                }
            }
        }
    }
}

impl RuntimeValue {
    pub fn from_rust_closure(heap: &mut gc::Heap, value: RustClosure) -> Self {
        RuntimeValue::NativeFunc(heap.alloc(GcCell::new(value)))
    }

    pub fn from_duka_closure(heap: &mut gc::Heap, value: DukaClosure) -> Self {
        RuntimeValue::UserFunc(heap.alloc(value))
    }
}

impl RuntimeValue {
    pub fn const2runtime(heap: &mut gc::Heap, cv: &ConstValue) -> Self {
        RuntimeValue::from_const(heap, cv.clone())
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
            ctype::STR
        } else if self.is_function() {
            ctype::FUN
        } else {
            match self {
                Self::Bool(..) => ctype::BOO,
                Self::Float(..) => ctype::FLO,
                Self::Int(..) => ctype::INT,
                Self::Nil => ctype::NIL,
                Self::Table(..) => ctype::TAB,
                Self::UserData(..) => "userdata",
                //Self::LightUserData() => "lightuserdata",
                _ => unreachable!(),
            }
        }
    }
}

impl Trace for RuntimeValue {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            RuntimeValue::MediumString(s) => tracer.mark(s),
            RuntimeValue::LongString(s) => tracer.mark(s),
            RuntimeValue::Table(t) => tracer.mark(t),
            RuntimeValue::UserFunc(c) => tracer.mark(c),
            RuntimeValue::NativeFunc(r) => tracer.mark(r),
            _ => {} //基本类型无需GC
        }
    }
}

impl Finalize for DukaClosure {
    fn finalize(&self) {}
}

impl Trace for DukaClosure {
    fn trace(&self, tracer: &mut Tracer) {
        tracer.mark(&self.func);
        for uv in &self.upvalues {
            tracer.mark(uv);
        }
    }
}

impl Finalize for UpValue {
    fn finalize(&self) {}
}

impl Trace for UpValue {
    fn trace(&self, tracer: &mut Tracer) {
        match self {
            UpValue::Open(_) => {}
            UpValue::Closed(rv) => rv.trace(tracer),
        }
    }
}

impl Finalize for DukaProto {
    fn finalize(&self) {}
}

impl Trace for DukaProto {
    fn trace(&self, tracer: &mut Tracer) {
        for uv in &self.upvalues {
            uv.trace(tracer);
        }
        for p in &self.nested_protos {
            p.trace(tracer);
        }
    }
}
