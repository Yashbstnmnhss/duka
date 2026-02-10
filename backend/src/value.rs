use duka_macros::Info;
use duka_shared::constants::{MetaMethod, ctype};
use duka_shared::value::ConstValue;
use duka_shared::value::{DukaFloat, DukaInt};
use gc::{Finalize, Gc, GcCell, Heap, Trace, Tracer};
use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::ops::{Add, Sub};

use crate::DebugInfo;
use crate::codegen::logic::LogicProto;
use crate::error::DukaRuntimeError;
use crate::instructions::Instruction;
use crate::vm::coroutine::{CoState, CoroutineID};

/// `instack`: `true`则在parent的栈中, `false`则也是parent的upvalue
#[derive(Debug, Clone, PartialEq)]
pub struct UpIndex {
    /// For debug
    pub name: Option<String>,
    /// Whether this is a local variable or another upvalue in parent closure
    pub local: bool,
    pub index: usize,
    pub kind: UpValueKind,
}
#[derive(Debug, Clone, PartialEq, Default, Info)]
#[idcard(u8)]
pub enum UpValueKind {
    #[default]
    Regular,
    ToBeClosed,
}

/// 捕获值
#[derive(Debug, Clone, PartialEq)]
pub enum UpValue {
    Open(usize),
    Closed(RuntimeValue),
}

/// 函数原型
#[derive(Debug, Clone, PartialEq)]
pub struct DukaProto {
    pub up_indexes: Vec<UpIndex>,
    pub constants: Vec<duka_shared::value::ConstValue>,

    pub instructions: Vec<Instruction>,
    pub reg_count: usize,
    pub nested_protos: Vec<DukaProto>,

    pub param_count: usize,
    pub has_var_arg: bool,

    pub debug_info: DebugInfo,

    pub logic: Option<LogicProto>,
}
impl Display for DukaProto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}([{}]{}, using {} upvalues) with {} constants, {} instructions, {} nested prototypes, {} registers used",
            self.debug_info
                .debug_name
                .as_ref()
                .map(|v| v.as_str())
                .unwrap_or("<Prototype>"),
            self.param_count,
            if self.has_var_arg { ", ..." } else { "" },
            self.up_indexes.len(),
            self.constants.len(),
            self.instructions.len(),
            self.nested_protos.len(),
            self.reg_count
        )
    }
}

pub const SHORT_STR_LEN: usize = 14;
pub const MID_STR_LEN: usize = 47;

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeDukaTable {
    pub inner: HashMap<RuntimeValue, RuntimeValue>,
    pub metatable: Option<Gc<GcCell<Self>>>,
}
impl RuntimeDukaTable {
    #[inline]
    pub fn new(n: usize) -> Self {
        Self {
            inner: HashMap::with_capacity(n),
            metatable: None,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn set(&mut self, key: RuntimeValue, val: RuntimeValue) {
        self.inner.insert(key, val);
    }
    pub fn get(&self, key: &RuntimeValue) -> Option<&RuntimeValue> {
        self.inner.get(key)
    }
    pub fn array_push(&mut self, at: usize, item: RuntimeValue) {
        self.set(RuntimeValue::Int(at as DukaInt), item);
    }
    pub fn array_get(&self, at: usize) -> Option<&RuntimeValue> {
        self.get(&RuntimeValue::Int(at as DukaInt))
    }
}
impl Finalize for RuntimeDukaTable {
    fn finalize(&self) {
        // todo
    }
}

impl Trace for RuntimeDukaTable {
    fn trace(&self, tracer: &mut Tracer) {
        // for v in &self.array {
        //     v.trace(tracer);
        // }
        for (k, v) in &self.inner {
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
impl PartialEq<usize> for ValueCount {
    fn eq(&self, other: &usize) -> bool {
        match self {
            Self::Exact(n) => n.eq(other),
            _ => false,
        }
    }
}
impl PartialOrd<usize> for ValueCount {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        match self {
            Self::Exact(n) => Some(n.cmp(other)),
            _ => Some(std::cmp::Ordering::Greater),
        }
    }
}
impl Add<usize> for ValueCount {
    type Output = Self;
    fn add(self, rhs: usize) -> Self::Output {
        match self {
            ValueCount::VarArg => ValueCount::VarArg,
            ValueCount::Exact(n) => ValueCount::Exact(n + rhs),
        }
    }
}
impl Sub<usize> for ValueCount {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self::Output {
        match self {
            ValueCount::VarArg => ValueCount::VarArg,
            ValueCount::Exact(n) => ValueCount::Exact(n.saturating_sub(rhs)),
        }
    }
}
impl Display for ValueCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueCount::Exact(n) => write!(f, "{n}"),
            ValueCount::VarArg => write!(f, "..."),
        }
    }
}
impl ValueCount {
    pub fn format_register(&self, from: usize) -> String {
        match self {
            Self::Exact(0) => "empty".to_owned(),
            Self::Exact(n) => format!("R[{from}] to R[{}]", from + n - 1),
            Self::VarArg => format!("R[{from}] to ..."),
        }
    }
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Exact(0))
    }
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
    pub fn from_proto(func: Gc<DukaProto>) -> Self {
        Self {
            func,
            upvalues: vec![],
        }
    }
    pub fn up_value(mut self, heap: &mut Heap, upval: UpValue) -> Self {
        self.upvalues.push(heap.alloc(GcCell::new(upval)));
        self
    }
}

/// ### Closure for Rust function
/// with function pointer itself
#[derive()]
pub struct RustClosure {
    pub func: Box<dyn FnMut(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>>,
}
impl RustClosure {
    #[inline(always)]
    pub fn returning<const C: usize, F>(mut f: F) -> Self
    where
        F: FnMut(&mut CoState, &mut Heap) -> Result<(), DukaRuntimeError> + 'static,
    {
        Self::returns(move |c, h| {
            f(c, h)?;
            Ok(ValueCount::Exact(C))
        })
    }
    #[inline(always)]
    pub fn nonreturn<F>(f: F) -> Self
    where
        F: FnMut(&mut CoState, &mut Heap) -> Result<(), DukaRuntimeError> + 'static,
    {
        Self::returning::<0, _>(f)
    }
    #[inline(always)]
    pub fn returns<F>(f: F) -> Self
    where
        F: FnMut(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError> + 'static,
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
    #[tag(table)]
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
            Self::UserData(ud) => Gc::as_ptr(ud).hash(state),
            Self::UserFunc(proto) => Gc::as_ptr(proto).hash(state),
            Self::NativeFunc(rust) => Gc::as_ptr(rust).hash(state),
        }
    }
}

impl RuntimeValue {
    pub(crate) fn from_short_str_unsafe(str: &'static str) -> RuntimeValue {
        let len = str.len();
        assert!(len <= SHORT_STR_LEN);
        let mut buffer = [0; SHORT_STR_LEN];
        buffer[..len].copy_from_slice(str.as_bytes());
        RuntimeValue::ShortString(len as u8, buffer)
    }
    pub fn from_string(heap: &mut gc::Heap, string: String) -> Self {
        let s = string.into_bytes();
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
                let mut rt = RuntimeDukaTable::new(t.inner.len());
                // for v in &t.array {
                //     rt.array.push(RuntimeValue::from_const(heap, v.clone()));
                // }
                for (k, v) in &t.inner {
                    let rk = RuntimeValue::from_const(heap, k.clone());
                    let rv = RuntimeValue::from_const(heap, v.clone());
                    rt.inner.insert(rk, rv);
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
    pub(crate) fn const2runtime(heap: &mut gc::Heap, cv: &ConstValue) -> Self {
        RuntimeValue::from_const(heap, cv.clone())
    }

    pub(crate) fn metamethod_key(heap: &mut gc::Heap, method: &MetaMethod) -> Self {
        Self::from_const(heap, ConstValue::String(method.name().as_bytes().to_vec()))
    }
}

impl RuntimeValue {
    pub fn eval_to_string(&self) -> Cow<'_, str> {
        use RuntimeValue::*;
        match self {
            ShortString(_, bytes) => Cow::Borrowed(str::from_utf8(bytes).unwrap()),
            MediumString(inner) => Cow::Borrowed(str::from_utf8(&inner.1).unwrap()),
            LongString(string) => Cow::Borrowed(&string.0),

            Int(i) => Cow::Owned(i.to_string()),
            Float(f) => Cow::Owned(f.to_string()),

            _ => Cow::Borrowed(self.name()),
        }
    }
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
                _ => {
                    debug_assert!(true, "This is unreachable, checking types of RuntimeValue");
                    unreachable!()
                }
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
        for p in &self.nested_protos {
            p.trace(tracer);
        }
    }
}
