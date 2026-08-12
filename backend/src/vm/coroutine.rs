use std::{
    cmp::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;

use crate::{
    errors::{DukaRuntimeError, DukaStackTrace, DukaTraceFrame},
    instructions::{Address, DecodeInstruction, Instruction},
    value::{
        DukaClosure, DukaProto, RuntimeDukaArray, RuntimeDukaTable, RuntimeValue, RustClosure,
        UpValue, make_pairs_iterator, make_values_iterator,
    },
    vm::{
        Bits25, CoAction,
        frame::{CallFrame, CallProto, Stack},
    },
};
use duka_gc::{Finalize, Trace, Tracer};
use duka_gc::{Gc, GcCell};
use duka_macros::Info;
use duka_shared::{
    constants::{MetaMethod, ctype, cvm},
    types::ValueCount,
    utils::OrError,
    value::{DukaFloat, DukaInt},
};
const INIT_CAPACITY: usize = 16;

#[inline(always)]
fn for_number_check<T: PartialOrd>(init: T, limit: T, neg_step: bool) -> bool {
    !neg_step && init <= limit || neg_step && init >= limit
}

#[inline(always)]
fn floor_div(a: DukaInt, b: DukaInt) -> DukaInt {
    let mut r = a / b;
    if (a % b) != 0 && (a < 0) != (b < 0) {
        r -= 1;
    }
    r
}

#[inline(always)]
fn check_zero(right: &RuntimeValue) -> Result<(), DukaRuntimeError> {
    (right.is_number()
        && (match right {
            RuntimeValue::Int(v) => *v == 0,
            RuntimeValue::Float(v) => *v == 0.0,
            _ => unreachable!(),
        }))
    .then_error(|| DukaRuntimeError::DividedByZero)
}

#[inline(always)]
fn ari_bit(
    a: &RuntimeValue,
    b: &RuntimeValue,
    f: fn(DukaInt, DukaInt) -> DukaInt,
) -> Option<DukaInt> {
    let (RuntimeValue::Int(a), RuntimeValue::Int(b)) = (a, b) else {
        return None;
    };
    Some(f(*a, *b))
}

#[inline(always)]
fn ari(
    a: &RuntimeValue,
    b: &RuntimeValue,
    fi: fn(DukaInt, DukaInt) -> DukaInt,
    ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
) -> Option<RuntimeValue> {
    unify_float(a, b).map(|c| match c {
        UnifiedNumber::Floats(a, b) => RuntimeValue::Float(ff(a, b)),
        UnifiedNumber::Ints(a, b) => RuntimeValue::Int(fi(a, b)),
    })
}

#[inline(always)]
fn cmp_im(
    fi: fn(DukaInt, DukaInt) -> bool,
    ff: fn(DukaFloat, DukaFloat) -> bool,
    im: DukaInt,
) -> impl Fn(&RuntimeValue) -> Option<bool> {
    move |v| -> Option<bool> {
        Some(match v {
            RuntimeValue::Int(i) => fi(*i, im),
            RuntimeValue::Float(f) => ff(*f, im as DukaFloat),
            _ => return None,
        })
    }
}

fn cmp_mi(
    fi: fn(DukaInt, DukaInt) -> bool,
    ff: fn(DukaFloat, DukaFloat) -> bool,
    im: DukaInt,
) -> impl Fn(&RuntimeValue) -> Option<bool> {
    move |v| -> Option<bool> {
        Some(match v {
            RuntimeValue::Int(i) => fi(im, *i),
            RuntimeValue::Float(f) => ff(im as DukaFloat, *f),
            _ => return None,
        })
    }
}

#[inline(always)]
fn cmp_lt(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
    Ok(match (a, b) {
        (
            RuntimeValue::ShortString(..)
            | RuntimeValue::MediumString(_)
            | RuntimeValue::LongString(_),
            RuntimeValue::ShortString(..)
            | RuntimeValue::MediumString(_)
            | RuntimeValue::LongString(_),
        ) => a.eval_to_string() < b.eval_to_string(),
        _ => {
            let c = unify_float(a, b).ok_or(DukaRuntimeError::InvalidValueType(ctype::NUM))?;
            match c {
                UnifiedNumber::Floats(a, b) => a < b,
                UnifiedNumber::Ints(a, b) => a < b,
            }
        }
    })
}
#[inline(always)]
fn cmp_le(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
    Ok(match (a, b) {
        (
            RuntimeValue::ShortString(..)
            | RuntimeValue::MediumString(_)
            | RuntimeValue::LongString(_),
            RuntimeValue::ShortString(..)
            | RuntimeValue::MediumString(_)
            | RuntimeValue::LongString(_),
        ) => a.eval_to_string() <= b.eval_to_string(),
        _ => {
            let c = unify_float(a, b).ok_or(DukaRuntimeError::InvalidValueType(ctype::NUM))?;
            match c {
                UnifiedNumber::Floats(a, b) => a <= b,
                UnifiedNumber::Ints(a, b) => a <= b,
            }
        }
    })
}
#[inline(always)]
fn cmp_eq(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
    Ok(match (a, b) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => a == b,
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => (*a as DukaFloat) == *b,
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => *a == (*b as DukaFloat),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => a == b,
        _ => a.eq(b),
    })
}

enum UnifiedNumber {
    Ints(DukaInt, DukaInt),
    Floats(DukaFloat, DukaFloat),
}
fn unify_float(a: &RuntimeValue, b: &RuntimeValue) -> Option<UnifiedNumber> {
    use UnifiedNumber::*;
    Some(match (a, b) {
        (RuntimeValue::Int(a), RuntimeValue::Int(b)) => Ints(*a, *b),
        (RuntimeValue::Int(a), RuntimeValue::Float(b)) => Floats(*a as DukaFloat, *b),
        (RuntimeValue::Float(a), RuntimeValue::Int(b)) => Floats(*a, *b as DukaFloat),
        (RuntimeValue::Float(a), RuntimeValue::Float(b)) => Floats(*a, *b),
        _ => return None,
    })
}

/// 协程运行状态
#[derive(Debug, Default)]
pub struct CoState {
    pub stack: Stack,
    pub frames: Vec<CallFrame>,
    pub open_upvalues: HashMap<usize, Gc<GcCell<UpValue>>, FxBuildHasher>,
    pub rng_state: u32,
    pub id: CoroutineID,
    pub status: CoroutineStatus,
    pub last_wanted: usize,
    pub ret_slot: u8,
    /// `yield` 表达式的值槽 再次 `go` 时装参数用
    pub resume_slot: Option<u8>,
    pub(crate) pending_action: Option<CoAction>,
}
impl CoState {
    pub fn create_trace(&self) -> DukaStackTrace {
        let mut frames = vec![];
        for frame in &self.frames {
            match &frame.proto {
                CallProto::Main { proto, .. } => {
                    frames.push(proto.func.create_trace_frame(Some(frame.pc)))
                }
                CallProto::Call { proto, .. } => {
                    let Some(val) = self.stack.get(*proto) else {
                        continue;
                    };
                    match val {
                        RuntimeValue::NativeFunc(proto) => frames.push(DukaTraceFrame {
                            debug_name: proto.borrow().debug_name.clone(),
                            source_name: None,
                            span: None,
                            is_native: true,
                        }),
                        RuntimeValue::UserFunc(proto) => {
                            frames.push(proto.func.create_trace_frame(Some(frame.pc)))
                        }
                        _ => continue,
                    }
                }
            }
        }
        DukaStackTrace { frames }
    }
    #[inline(always)]
    pub(crate) fn new_unsafe(reg_count: Option<usize>) -> Self {
        Self {
            stack: Vec::with_capacity(reg_count.unwrap_or(INIT_CAPACITY)),
            frames: vec![],
            open_upvalues: HashMap::with_capacity_and_hasher(0, FxBuildHasher),
            rng_state: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("WHY ARE YOU USING THIS BEFORE 1970")
                .as_nanos() as u32,
            id: 0,
            status: CoroutineStatus::default(),
            last_wanted: 0,
            ret_slot: 0,
            resume_slot: None,
            pending_action: None,
        }
    }
    #[inline(always)]
    pub fn with_closure(closure: Gc<DukaClosure>) -> Self {
        Self {
            stack: Vec::with_capacity(closure.func.used_reg_count),
            frames: vec![CallFrame::main(closure)],
            open_upvalues: HashMap::with_capacity_and_hasher(0, FxBuildHasher),
            rng_state: 171912,
            id: 0,
            status: CoroutineStatus::default(),
            last_wanted: 0,
            ret_slot: 0,
            resume_slot: None,
            pending_action: None,
        }
    }
    #[inline(always)]
    pub fn with_proto(proto: Gc<DukaProto>, heap: &mut duka_gc::Heap) -> Self {
        Self::with_closure(heap.alloc(DukaClosure::from_proto(proto)))
    }

    fn get_closure(&self) -> Result<&Gc<DukaClosure>, DukaRuntimeError> {
        match &self.current().proto {
            CallProto::Main { proto, .. } => Ok(proto),
            // `proto` stores the callee's absolute stack slot (its frame base
            // sits one above at `base`), so it must NOT go through the base
            // offsetting of `get_stack`.
            CallProto::Call { proto, .. } => match self.stack.get(*proto) {
                Some(RuntimeValue::UserFunc(p)) => Ok(p),
                _ => Err(DukaRuntimeError::InvalidValueType(ctype::PRO)),
            },
        }
    }
    fn fetch(&self) -> Result<&Instruction, DukaRuntimeError> {
        let pc = &self.current().pc;
        let proto = self.get_closure()?;
        Ok(&proto.func.instructions[*pc])
    }

    #[inline]
    pub fn push_frame(&mut self, frame: CallFrame) {
        if let CallProto::Main { proto, .. } = frame.proto {
            self.stack.reserve(proto.func.used_reg_count);
        }
        self.frames.push(frame);
    }

    fn get_up_value(&self, index: usize) -> Result<&Gc<GcCell<UpValue>>, DukaRuntimeError> {
        self.get_closure()?
            .up_values
            .get(index)
            .ok_or(DukaRuntimeError::OutOfRange(cvm::UPVAL))
    }

    #[inline]
    pub fn current(&self) -> &CallFrame {
        self.frames.last().expect("WHERE IS YOUR MAIN FRAME?") //bro...
    }
    #[inline]
    pub fn current_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().expect("WHERE IS YOUR MAIN FRAME?")
    }

    #[inline(always)]
    fn get_base(&self) -> usize {
        self.current().get_base()
    }
    fn set_base(&mut self, base: usize) {
        self.current_mut().set_base(base);
    }

    pub(crate) fn adjust_stack(&mut self, to_len: usize) {
        self.stack.truncate(to_len);
    }

    pub(crate) fn cut_stack(&mut self, from: usize, count: ValueCount) -> Vec<RuntimeValue> {
        self.stack
            .drain(from..count.to_index(self.stack.len()))
            .collect()
    }

    pub fn get_stack_many(&self, from: usize, count: ValueCount) -> &[RuntimeValue] {
        if count.is_empty() || from >= self.stack.len() {
            &[]
        } else {
            let els = self.stack.get(from..).unwrap_or_default();
            match count {
                ValueCount::Exact(n) => {
                    let len = els.len().min(n);
                    &els[..len]
                }
                _ => els,
            }
        }
    }

    pub fn get_stack_mut(&mut self, ad: usize) -> Result<&mut RuntimeValue, DukaRuntimeError> {
        if !self.ensure_address(ad) {
            return Err(DukaRuntimeError::OutOfRange(cvm::STACK));
        }
        let dst = ad + self.get_base();
        self.stack
            .get_mut(dst)
            .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))
    }

    pub fn take_stack(&mut self, ad: usize) -> Result<RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.get_base();
        let val = self
            .stack
            .get_mut(dst)
            .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?;
        Ok(std::mem::take(val))
    }

    pub fn take_stack_many(
        &mut self,
        from: usize,
        count: ValueCount,
    ) -> Result<Box<[RuntimeValue]>, DukaRuntimeError> {
        let from = from + self.get_base();
        let res = match count {
            ValueCount::Exact(n) => self
                .stack
                .drain(from..(from + n).min(self.stack.len()))
                .chain(std::iter::from_fn(|| Some(RuntimeValue::default())))
                .take(n)
                .collect(),
            ValueCount::VarArg => self.stack.drain(from..).collect(),
        };
        Ok(res)
    }
    /// **含偏移*
    pub fn ensure_address(&self, ad: usize) -> bool {
        self.stack.len() > ad + self.get_base()
    }
    /// 获取栈上的值**含base偏移**
    pub fn get_stack(&self, ad: usize) -> Result<&RuntimeValue, DukaRuntimeError> {
        if !self.ensure_address(ad) {
            return Err(DukaRuntimeError::OutOfRange(cvm::STACK));
        }
        let dst = ad + self.get_base();
        Ok(&self.stack[dst])
    }
    pub fn append_stack(&mut self, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
        self.stack.push(val);
        Ok(())
    }
    pub fn set_stack(&mut self, ad: usize, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
        let dst = ad + self.get_base();
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => {
                self.stack.resize_with(dst + 1, RuntimeValue::default);
                self.stack[dst] = val;
            }
        }
        Ok(())
    }
    pub fn set_stack_many(
        &mut self,
        from: usize,
        values: &[RuntimeValue],
    ) -> Result<(), DukaRuntimeError> {
        for (i, val) in values.iter().cloned().enumerate() {
            self.set_stack(from + i, val)?;
        }
        Ok(())
    }
}

impl Finalize for CoState {
    fn finalize(&self) {}
}

impl Trace for CoState {
    fn trace(&self, tracer: &mut Tracer) {
        // Trace stack values
        for v in &self.stack {
            v.trace(tracer);
        }
        // Trace call frames
        for f in &self.frames {
            f.trace(tracer);
        }
        // Trace open upvalue cells
        for uv in self.open_upvalues.values() {
            tracer.mark(uv);
        }
    }
}

pub type CoroutineID = usize;

pub type ShadowStatus = HashMap<CoroutineID, CoroutineStatus>;
pub type ShadowCell = std::rc::Rc<std::cell::RefCell<ShadowStatus>>;
pub type GcFlagCell = std::rc::Rc<std::cell::Cell<bool>>;

/// API to access whole VM
#[derive(Debug)]
pub struct NativeApi {
    pending: Option<CoAction>,
    shadow: ShadowCell,
    gc_flag: GcFlagCell,
}

impl Default for NativeApi {
    fn default() -> Self {
        Self {
            pending: None,
            shadow: Default::default(),
            gc_flag: Default::default(),
        }
    }
}

impl NativeApi {
    pub fn emit(&mut self, action: CoAction) {
        self.pending = Some(action);
    }
    pub(crate) fn take_pending(&mut self) -> Option<CoAction> {
        self.pending.take()
    }

    pub fn co_status(&self, id: CoroutineID) -> CoroutineStatus {
        self.shadow
            .borrow()
            .get(&id)
            .copied()
            .unwrap_or(CoroutineStatus::Unknown)
    }
    pub fn request_gc(&mut self) {
        self.gc_flag.set(true);
    }

    pub(crate) fn with_runtime(shadow: ShadowCell, gc_flag: GcFlagCell) -> Self {
        Self {
            pending: None,
            shadow,
            gc_flag,
        }
    }
}

/// # 协程状态
#[derive(Debug, Info, Default, Clone, Copy)]
pub enum CoroutineStatus {
    /// 准备完毕
    #[default]
    #[tag(go_able)]
    Ready,
    /// 正在运行
    Running,
    /// 已经挂起
    #[tag(go_able)]
    Suspended,
    /// 已经结束
    Dead,
    /// 未知状态
    #[name("unknown")]
    Unknown,
}

/// # 协程
#[derive(Debug)]
pub struct Coroutine {
    pub inner: CoState,
    pub parent: Option<CoroutineID>,
}

pub(crate) fn call_native_meta_sync(
    sv: &mut CoState,
    heap: &mut duka_gc::Heap,
    api: &mut NativeApi,
    closure: Gc<GcCell<RustClosure>>,
    params: &[RuntimeValue],
) -> Result<RuntimeValue, DukaRuntimeError> {
    let saved_stack = std::mem::take(&mut sv.stack);
    let saved_base = sv.get_base();
    sv.set_base(0);
    sv.stack.push(RuntimeValue::NativeFunc(closure));
    for p in params {
        sv.stack.push(p.clone());
    }
    (closure.borrow_mut().func)(sv, heap, api)?;
    let results = std::mem::take(&mut sv.stack);
    sv.stack = saved_stack;
    sv.set_base(saved_base);
    Ok(results.into_iter().next().unwrap_or(RuntimeValue::Nil))
}

impl Coroutine {
    pub fn new(id: CoroutineID, state: CoState, parent: Option<CoroutineID>) -> Self {
        let mut state = state;
        state.id = id;
        state.status = CoroutineStatus::Ready;
        Self {
            inner: state,
            parent,
        }
    }

    /// ### Push a frame of calling into this coroutine
    pub fn push_frame(&mut self, frame: CallFrame) {
        self.inner.push_frame(frame);
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    pub fn call(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        func: usize,
        narg: ValueCount,
        nwanted: ValueCount,
        tailcall: bool,
    ) -> Result<(), DukaRuntimeError> {
        self.inner.call(heap, api, func, narg, nwanted, tailcall)
    }
}
impl Finalize for Coroutine {
    fn finalize(&self) {}
}

impl Trace for Coroutine {
    fn trace(&self, tracer: &mut Tracer) {
        // Trace inner state which contains stack and frames
        self.inner.trace(tracer);
    }
}
impl CoState {
    fn unpack_up_val<'a>(
        &'a self,
        up_value: &'a UpValue,
    ) -> Result<&'a RuntimeValue, DukaRuntimeError> {
        Ok(match up_value {
            UpValue::Closed(c) => c,
            // Open upvalues store an absolute stack slot (created as
            // `base + index`), so read the stack directly without re-adding
            // the current frame's base.
            UpValue::Open(i) => self
                .stack
                .get(*i)
                .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?,
        })
    }
    fn with_up_val<F, R>(&mut self, up_val_idx: usize, f: F) -> Result<R, DukaRuntimeError>
    where
        F: FnOnce(&mut RuntimeValue) -> R,
    {
        let mut borrow = self.get_up_value(up_val_idx)?.borrow_mut();
        Ok(match *borrow {
            UpValue::Closed(ref mut v) => f(v),
            UpValue::Open(i) => {
                let val = self
                    .stack
                    .get_mut(i)
                    .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?;
                f(val)
            }
        })
    }

    pub fn reset(&mut self) {
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
        self.last_wanted = 0;
        self.ret_slot = 0;
        self.resume_slot = None;
        self.pending_action = None;

        self.status = CoroutineStatus::Ready;
    }

    /// ### Where instructions are executed exactly
    pub fn execute(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        boundary: Option<usize>,
    ) -> Result<CoAction, DukaRuntimeError> {
        use CoroutineStatus::*;
        use DecodeInstruction::*;
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        if matches!(self.status, Dead) {
            return Err(UnableRunCoroutine(self.id));
        }

        self.status = Running;

        let mut extra_arg: Option<Bits25> = None;

        macro_rules! cast {
            /* RUST */
            (for $t: ident all(from $c: expr) as usize) => {
                match $t.into() {
                    ValueCount::VarArg => vm!(@top).saturating_sub($c),
                    ValueCount::Exact(c) => c
                }
            };
            (as $($target: ident : $type: ty),+) => {
                $(let $target = $target as $type);+;
            };
            (deref $($target: ident),+) => {
                $(let $target = *$target);+;
            };

            /* DUKA casting type */
            ($ty: ident use $func:ident for $target: expr) => {
                $target . $func ().ok_or_else(|| InvalidValueType(stringify!($ty)))
            };
            ($ty: ident(deref $id: ident) = $target: expr) => {
                let $ty($id) = $target else {
                    return Err(InvalidValueType(stringify!($ty)));
                };
                let $id = *$id;
            };
            ($ty: ident($($id: ident),*) = $target: expr) => {
                let $ty($($id),*) = $target else {
                    return Err(InvalidValueType(stringify!($ty)));
                };
            };
        }
        macro_rules! vm {
            /* stack (registers) *NO BASE */
            // cut a range of items
            (@stack:remove [$start: expr]..[$end: expr]) => {
                self.stack.drain($start as usize..$end as usize)
            };
            // drop the tail
            (@stack:remove [$end: expr]..) => {
                self.stack.drain($end as usize..)
            };

            /* getter */
            (@frame) => {
                self.current()
            };
            (@frame mut) => {
                self.current_mut()
            };
            (@top) => {
                self.stack.len()
            };
            (@base) => {
                vm!(@frame).get_base()
            };

            /* calculator */
            ([$e: expr] for R) => {
                $e as usize
            };

            /* pc control */
            (move $e: expr) => {
                vm!(@frame mut).pc = ((vm!(@frame).pc as isize) + ($e as isize)) as usize;
            };
            // for all normal instruction
            (continue) => {
                vm!(@frame mut).pc += 1;
            };
            // for conditional instruction
            (skip) => {
                vm!(@frame mut).pc += 1;
            };

            (UpVal($i: expr) $(@get)?) => {
                self.get_up_value($i as usize)?
            };
            (UpVal($i: expr) := $v: expr) => {
                self.get_closure()?.up_values.set($i as usize).and_then(|u| u.get_value()).ok_or(OutOfUpvalue)?
            };

            /* read *HAS BASE */
            (R($ad: expr; $ct: expr) $(@get)?) => {
                (0..$ct as usize).map(|i| self.get_stack(vm!([$ad as usize + i] for R))).collect::<Result<Vec<_>, _>>()?
            };
            (R($ad: expr) $(@get)?) => {
                self.get_stack(vm!([$ad] for R))?
            };
            (K($i: expr) $(@get)?) => {{
                let proto = self.get_closure()?.func;
                proto
                    .runtime_const(heap, $i as usize)
                    .ok_or(OutOfRange(cvm::CONST))?
            }};

            (RK($ad:expr, $flag:expr) $(@get)?) => {
                if $flag { vm!(K($ad) @get) } else { vm!(R($ad) @get).clone() }
            };
            (E() $(@get)?) => {
                extra_arg.take().ok_or(ExtraArgNotFound)?
            };

            /* set *HAS BASE */
            (R($ad: expr; $ct: expr) := fill $v: expr) => {
                for i in 0..$ct as usize {
                    vm!(R($ad as usize + i) := $v);
                }
            };
            (R($a: expr) := R($b: expr)) => {{
                let v = vm!(R($b) @get).clone();
                vm!(R($a) := v);
            }};
            (R($a: expr) := K($b: expr)) => {{
                let v = vm!(K($b) @get);
                vm!(R($a) := v);
            }};
            (R($a: expr) := $v: expr) => {
                self.set_stack(vm!([$a] for R), $v)?;
            };
        }

        'inst: loop {
            if let Some(action) = self.pending_action.take() {
                return Ok(action);
            }
            let inst = self.fetch()?;

            (inst.check_extra().map_err(InvalidInstruction)? && extra_arg.is_none())
                .then_error(|| ExtraArgNotFound)?;

            let decoded = inst.decode().map_err(InvalidInstruction)?;
            match decoded {
                Move(a, b) => {
                    vm!(R(a) := R(b));
                }
                LoadTrue(a) => {
                    vm!(R(a) := Bool(true));
                }
                LoadFalse(a) => {
                    vm!(R(a) := Bool(false));
                }
                LoadNil(a, count) => {
                    vm!(R(a; count) := fill Nil);
                }
                LoadI(a, num) => {
                    vm!(R(a) := Int(num as DukaInt));
                }
                LoadK(a, k) => {
                    vm!(R(a) := K(k));
                }
                LoadKX(a) => {
                    let i = vm!(E()); // checked
                    vm!(R(a) := K(i));
                }
                Xor(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    let res = left.eval_to_bool() ^ right.eval_to_bool();
                    vm!(R(a) := Bool(res));
                }
                Add(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Add, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_add, std::ops::Add::add)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Sub(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Sub, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_sub, std::ops::Sub::sub)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Mul(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Mul, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_mul, std::ops::Mul::mul)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Div(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Div, &left, &right, |l, r| {
                            if l.is_number() && r.is_number() {
                                check_zero(r)?;
                            }
                            unify_float(l, r)
                                .ok_or(InvalidValueType(ctype::NUM))
                                .map(|c| match c {
                                    UnifiedNumber::Floats(a, b) => Float(a / b),
                                    UnifiedNumber::Ints(a, b) => {
                                        Float(a as DukaFloat / b as DukaFloat)
                                    }
                                })
                        })?;
                    vm!(R(a) := result);
                }
                IDiv(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::IDiv, &left, &right, |l, r| {
                            if l.is_number() && r.is_number() {
                                check_zero(r)?;
                            }
                            unify_float(l, r)
                                .ok_or(InvalidValueType(ctype::NUM))
                                .map(|c| match c {
                                    UnifiedNumber::Ints(a, b) => Int(floor_div(a, b)),
                                    UnifiedNumber::Floats(a, b) => Float((a / b).floor()),
                                })
                        })?;
                    vm!(R(a) := result);
                }
                Mod(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Mod, &left, &right, |l, r| {
                            if l.is_number() && r.is_number() {
                                check_zero(r)?;
                            }
                            unify_float(l, r)
                                .ok_or(InvalidValueType(ctype::NUM))
                                .map(|c| match c {
                                    UnifiedNumber::Ints(a, b) => Int(a - floor_div(a, b) * b),
                                    UnifiedNumber::Floats(a, b) => Float(a - (a / b).floor() * b),
                                })
                        })?;
                    vm!(R(a) := result);
                }
                Pow(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, api, &MetaMethod::Pow, &left, &right, |l, r| {
                            unify_float(l, r)
                                .ok_or(InvalidValueType(ctype::NUM))
                                .map(|c| match c {
                                    UnifiedNumber::Floats(a, b) => Float(a.powf(b)),
                                    UnifiedNumber::Ints(a, b) => {
                                        Float((a as DukaFloat).powi(b as i32))
                                    }
                                })
                        })?;
                    vm!(R(a) := result);
                }
                BitAnd(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, api, &MetaMethod::BAnd, &left, &right, |l, r| {
                            ari_bit(l, r, std::ops::BitAnd::bitand)
                                .map(Int)
                                .ok_or(InvalidValueType(ctype::INT))
                        })?;
                    vm!(R(a) := result);
                }
                BitOr(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, api, &MetaMethod::BOr, &left, &right, |l, r| {
                            ari_bit(l, r, std::ops::BitOr::bitor)
                                .map(Int)
                                .ok_or(InvalidValueType(ctype::INT))
                        })?;
                    vm!(R(a) := result);
                }
                BitXor(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, api, &MetaMethod::BXor, &left, &right, |l, r| {
                            ari_bit(l, r, std::ops::BitXor::bitxor)
                                .map(Int)
                                .ok_or(InvalidValueType(ctype::INT))
                        })?;
                    vm!(R(a) := result);
                }
                ShiftL(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, api, &MetaMethod::ShL, &left, &right, |l, r| {
                            let (Int(l), Int(r)) = (l, r) else {
                                return Err(InvalidValueType(ctype::INT));
                            };
                            let v = if *r < 0 {
                                l.wrapping_shr((-*r) as u32)
                            } else {
                                l.wrapping_shl(*r as u32)
                            };
                            Ok(Int(v))
                        })?;
                    vm!(R(a) := result);
                }
                ShiftR(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, api, &MetaMethod::ShR, &left, &right, |l, r| {
                            let (Int(l), Int(r)) = (l, r) else {
                                return Err(InvalidValueType(ctype::INT));
                            };
                            let v = if *r < 0 {
                                l.wrapping_shl((-*r) as u32)
                            } else {
                                l.wrapping_shr(*r as u32)
                            };
                            Ok(Int(v))
                        })?;
                    vm!(R(a) := result);
                }
                Equal(a, b, c, t) => {
                    let (b, c) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let equal = if let Some(r) =
                        self.try_binary_meta_method(heap, api, &MetaMethod::Eq, &b, &c)?
                    {
                        r.eval_to_bool()
                    } else {
                        cmp_eq(&b, &c)?
                    };
                    vm!(R(a) := Bool(equal == t));
                }
                Less(a, b, c) => {
                    let (b, c) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let r =
                        self.compare_meta(heap, api, &MetaMethod::LT, &b, &c, |l, r| cmp_lt(l, r))?;
                    vm!(R(a) := Bool(r));
                }
                LessEqual(a, b, c) => {
                    let (b, c) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let r =
                        self.compare_meta(heap, api, &MetaMethod::LE, &b, &c, |l, r| cmp_le(l, r))?;
                    vm!(R(a) := Bool(r));
                }
                Concat(a, count) => {
                    let mut has_concat = false;
                    let mut has_to_string = false;
                    for i in 0..count as usize {
                        if let Table(t) = self.get_stack(a as usize + i)? {
                            if t.borrow()
                                .get_meta_method(heap, &MetaMethod::Concat)
                                .is_some_and(|m| m.is_function())
                            {
                                has_concat = true;
                                break;
                            }
                            if t.borrow()
                                .get_meta_method(heap, &MetaMethod::ToString)
                                .is_some_and(|m| m.is_function())
                            {
                                has_to_string = true;
                            }
                        }
                    }

                    if has_concat {
                        let mut acc = self.get_stack(a as usize)?.clone();
                        for i in 1..count as usize {
                            let next = self.get_stack(a as usize + i)?.clone();
                            let meta = match (&acc, &next) {
                                (Table(t), _) | (_, Table(t)) => {
                                    t.borrow().get_meta_method(heap, &MetaMethod::Concat)
                                }
                                _ => None,
                            };
                            acc = if let Some(m) = meta.filter(|m| m.is_function()) {
                                self.call_sync(heap, api, m, [acc, next])?
                            } else {
                                // strcat性能问题:
                                let s = format!(
                                    "{}{}",
                                    self.to_concat_string(heap, api, acc)?,
                                    self.to_concat_string(heap, api, next)?
                                );
                                RuntimeValue::from_string(heap, s)
                            };
                        }
                        vm!(R(a) := acc);
                    } else {
                        if count == 2 {
                            //TODO
                        }

                        let operands = vm!(R(a; count));
                        let total_len: usize =
                            operands.iter().map(|i| i.eval_to_string().len()).sum();
                        let mut buf = String::with_capacity(total_len);
                        if has_to_string {
                            for i in 0..count as usize {
                                let val = self.get_stack(a as usize + i)?.clone();
                                let s = match val {
                                    Table(_) => self.to_concat_string(heap, api, val)?,
                                    _ => val.eval_to_string().into_owned(),
                                };
                                buf.push_str(&s);
                            }
                        } else {
                            for i in operands {
                                buf.push_str(&i.eval_to_string());
                            }
                        }
                        let r = RuntimeValue::from_string(heap, buf);
                        vm!(R(a) := r);
                    }
                }
                Minus(a, b) => {
                    let r = vm!(R(b));
                    let v = match r {
                        Int(i) => Int(-i),
                        Float(f) => Float(-f),
                        Table(t) => {
                            let t = *t;
                            if let Some(r) =
                                self.call_unary_meta_method(heap, api, &MetaMethod::Unm, t)?
                            {
                                r
                            } else {
                                return Err(UnsupportedOperation("minus", ctype::TAB));
                            }
                        }
                        _ => return Err(UnsupportedOperation("minus", r.type_name_of())),
                    };
                    vm!(R(a) := v);
                }
                Not(a, b) => {
                    let b = vm!(R(b));
                    let val = b.eval_to_bool();
                    vm!(R(a) := Bool(!val));
                }
                BitNot(a, b) => {
                    if let Table(t) = vm!(R(b))
                        && let Some(r) =
                            self.call_unary_meta_method(heap, api, &MetaMethod::BNot, *t)?
                    {
                        vm!(R(a) := r);
                    } else {
                        let val = vm!(R(b));
                        let num = val
                            .eval_to_int()
                            .ok_or_else(|| UnsupportedOperation("bit not", val.type_name_of()))?;
                        vm!(R(a) := Int(!num));
                    }
                }
                Length(a, b) => {
                    cast!(as a: usize, b: usize);
                    let val = vm!(R(b));
                    match val {
                        LongString(l) => {
                            vm!(R(a) := Int(l.0.len() as DukaInt));
                        }
                        MediumString(m) => {
                            vm!(R(a) := Int(m.0 as DukaInt));
                        }
                        ShortString(s, _) => {
                            vm!(R(a) := Int(*s as DukaInt));
                        }
                        Table(t) => {
                            let t = *t;
                            if let Some(r) =
                                self.call_unary_meta_method(heap, api, &MetaMethod::Len, t)?
                            {
                                vm!(R(a) := r);
                            } else {
                                let b = t.borrow();
                                vm!(R(a) := Int(b.len() as DukaInt));
                            }
                        }
                        Array(arr) => {
                            let b = arr.borrow();
                            vm!(R(a) := Int(b.len() as DukaInt));
                        }
                        _ => return Err(UnsupportedOperation("len", val.type_name_of())),
                    }
                }
                Jump(offset) => {
                    vm!(move offset);
                    continue; // already moved, dont vm!(continue)
                }
                Test(from, target) => {
                    // skip next if R(a) == b
                    let val = vm!(R(from));
                    if val.eval_to_bool() == target {
                        vm!(skip);
                    }
                }

                ForPrepare(a, end_offset) => {
                    fn for_limit(
                        limit: DukaFloat,
                        step_positive: bool,
                    ) -> Result<DukaInt, DukaInt> {
                        if step_positive {
                            {
                                (limit >= DukaInt::MIN as DukaFloat)
                                    .then_some(limit.floor() as DukaInt)
                                    .ok_or(-1)
                            }
                        } else {
                            {
                                (limit <= DukaInt::MAX as DukaFloat)
                                    .then_some(limit.ceil() as DukaInt)
                                    .ok_or(1)
                            }
                        }
                    }

                    if let Int(init) = vm!(R(a))
                        && let Int(step) = vm!(R(a + 2))
                    {
                        cast!(deref init, step);
                        (step == 0).then_error(|| ZeroStepInForLoop)?;

                        let lim_rv = vm!(R(a + 1));
                        let limit = match lim_rv {
                            Int(i) => Ok(*i),
                            Float(f) => for_limit(*f, step.is_positive()),
                            _ => {
                                return Err(InvalidValueType(ctype::NUM));
                            }
                        };

                        if let Ok(limit) = limit {
                            if !for_number_check(init, limit, step.is_negative()) {
                                vm!(move end_offset);
                                continue;
                            } else {
                                vm!(R(a + 1) := Int(limit));
                                // then loop
                            }
                        } else {
                            vm!(move end_offset); // this will move to the last code of inner block
                            continue;
                        }
                    } else {
                        let init = cast!(Number use eval_to_float for vm!(R(a)))?;
                        let limit = cast!(Number use eval_to_float for vm!(R(a + 1)))?;
                        let step = cast!(Number use eval_to_float for vm!(R(a + 2)))?;

                        (step == 0.0).then_error(|| ZeroStepInForLoop)?;

                        if !for_number_check(init, limit, step.is_sign_negative()) {
                            vm!(move end_offset);
                            continue;
                        } else {
                            // then loop
                        }
                    }
                }
                ForLoop(a, start_offset) => {
                    let init = vm!(R(a));
                    let limit = vm!(R(a + 1));
                    let step = vm!(R(a + 2));
                    // status

                    if let Int(step) = step {
                        cast!(Int(deref init) = init);
                        cast!(Int(deref limit) = limit);

                        let new = init + (*step);
                        let neg_step = step.is_negative();

                        vm!(R(a) := Int(new));

                        if for_number_check(new, limit, neg_step) {
                            vm!(move - (start_offset as isize)); // 回跳 to_start
                            continue;
                        }
                    } else {
                        cast!(Float(deref init) = init);
                        cast!(Float(deref limit) = limit);
                        cast!(Float(deref step) = step);

                        let new = init + step;
                        // step != 0, already checked in ForPrepare
                        let neg_step = step.is_sign_negative();

                        vm!(R(a) := Float(new));

                        if for_number_check(new, limit, neg_step) {
                            vm!(move - (start_offset as isize)); // 回跳 to_start
                            continue;
                        }
                    }
                }

                TForPrepare(_, offset) => {
                    cast!(as offset: isize);
                    vm!(move offset);
                    continue;
                }
                TForCall(a, nres) => {
                    cast!(as nres: usize, a: usize);
                    if let RuntimeValue::Table(tab) = vm!(R(a)).clone() {
                        let entries: Vec<(RuntimeValue, RuntimeValue)> = tab
                            .borrow()
                            .inner
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        let iter = if nres == 1 {
                            make_values_iterator(
                                heap,
                                entries.into_iter().map(|(_, v)| v).collect(),
                            )
                        } else {
                            make_pairs_iterator(heap, entries)
                        };
                        vm!(R(a) := iter);
                        vm!(R(a + 1) := RuntimeValue::Table(tab));
                        vm!(R(a + 2) := RuntimeValue::Nil);
                    }
                    vm!(R(a + 3) := R(a));
                    vm!(R(a + 4) := R(a + 1));
                    vm!(R(a + 5) := R(a + 2));
                    vm!(move 1);
                    self.call(
                        heap,
                        api,
                        a + 3,
                        ValueCount::Exact(2),
                        ValueCount::Exact(nres + 1), // See docs/stdlib.md #Generator & Iterator Protocol
                        false,
                    )?;
                    continue 'inst;
                }
                TForLoop(a, offset) => {
                    cast!(as offset: isize);

                    let res = vm!(R(a + 3)).clone(); //第一个返回时代表是否继续
                    if matches!(res, RuntimeValue::Bool(true)) {
                        vm!(R(a + 2) := R(a + 4)); // 取第二个返回值
                        vm!(move -offset);
                        continue;
                    }
                }

                Closure(ad, index) => {
                    cast!(as index: usize);
                    // push closure to stack & initialize its up_values

                    let proto = self
                        .get_closure()?
                        .func
                        .nested_protos
                        .get(index)
                        .expect("NO PROTO FOUND?!")
                        .clone();

                    let mut up_values = vec![];

                    for desc in &proto.up_indexes {
                        let up_val = if desc.local {
                            // Deduplicate: a captured local must be one shared
                            // cell so writes are visible to every closure and
                            // it closes exactly once when its frame returns.
                            let slot = vm!(@base) + desc.index;
                            match self.open_upvalues.get(&slot) {
                                Some(existing) => *existing,
                                None => {
                                    let cell = heap.alloc(GcCell::new(UpValue::Open(slot)));
                                    self.open_upvalues.insert(slot, cell);
                                    cell
                                }
                            }
                        } else {
                            *vm!(UpVal(desc.index))
                        };
                        up_values.push(up_val)
                    }

                    // allocate proto and closure on VM heap
                    let proto_gc = heap.alloc(proto);
                    let closure = heap.alloc(DukaClosure {
                        func: proto_gc,
                        up_values,
                    });
                    vm!(R(ad) := UserFunc(closure));
                }

                Call(func, narg, nwanted) => {
                    cast!(as func: usize);
                    // Advance the caller's pc past the call, then run the
                    // callee from its first instruction
                    vm!(move 1);
                    self.call(heap, api, func, narg.into(), nwanted.into(), false)?;
                    continue 'inst;
                }
                TailCall(func, narg, nwanted) => {
                    cast!(as func: usize);
                    self.call(heap, api, func, narg.into(), nwanted.into(), true)?;
                    continue 'inst;
                }

                SysCall(syscall, narg, nwanted) => {
                    let closure = self.get_closure()?;
                    if let Some(ref logic_proto) = closure.func.logic {
                        let query_idx = narg as usize;
                        let solutions = crate::vm::logic::execute_query(logic_proto, query_idx)
                            .map_err(|e| Custom(e))?;
                        let count = match ValueCount::from(nwanted) {
                            ValueCount::Exact(n) => n.min(solutions.len()),
                            ValueCount::VarArg => solutions.len(),
                        };
                        for i in 0..count {
                            let sol = &solutions[i];
                            let entry = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
                            let mut keys: Vec<&usize> = sol.keys().collect();
                            keys.sort();
                            for (j, k) in keys.iter().enumerate() {
                                let val = RuntimeValue::from_string(heap, sol[k].clone());
                                entry
                                    .borrow_mut()
                                    .set(RuntimeValue::Int((j + 1) as i64), val);
                            }
                            vm!(R(syscall as usize + i) := RuntimeValue::Table(entry));
                        }
                    }
                }

                Return(from, count_) => {
                    cast!(as from: usize);

                    self.close_up_values()?;

                    let actual_count = cast!(
                        for count_
                        all(from vm!([from] for R) + vm!(@base))
                        as usize
                    );

                    let CallProto::Call { wanted, proto, .. } = self.current().proto else {
                        self.status = Dead;
                        // The main chunk may leave its results at register
                        // `from` (e.g. a bare `return f()` keeps them at the
                        // callee slot); compact them down to stack position 0
                        // so `VM::run`/`run_take` read the real results.
                        let src = self.get_base() + from;
                        let stack = &mut self.stack;
                        for i in 0..actual_count {
                            stack[i] = stack.get(src + i).cloned().unwrap_or_default();
                        }
                        stack.truncate(actual_count);
                        return Ok(CoAction::Return(
                            0 as Address,
                            ValueCount::Exact(actual_count),
                        ));
                    };
                    let abs_func = proto;
                    self.frames.pop().ok_or(NoCallFrame)?;

                    // The callee's registers live above its frame base
                    // (`abs_func + 1`), so its results start at
                    // `abs_func + 1 + from`; the caller expects them at
                    // `abs_func..` (the callee slot, matching the codegen's
                    // `Take` placement). Copy the results down and trim the
                    // stack to the results' end.
                    let src = abs_func + 1 + from;
                    let dst = abs_func;
                    let total = if wanted == usize::MAX {
                        actual_count
                    } else {
                        wanted.max(actual_count)
                    };
                    for i in 0..total {
                        let val = if i < actual_count {
                            self.stack.get(src + i).cloned().unwrap_or_default()
                        } else {
                            RuntimeValue::default()
                        };
                        self.stack[dst + i] = val;
                    }
                    self.adjust_stack(dst + total);

                    if let Some(b) = boundary {
                        if self.frames.len() == b {
                            return Ok(CoAction::Return(
                                abs_func as Address,
                                ValueCount::Exact(actual_count),
                            ));
                        }
                    }
                    // The Call handler already advanced the caller's pc past
                    // the call, so the loop's trailing `vm!(continue)` must
                    // not touch it again.
                    continue 'inst;
                }
                Return0() => {
                    self.close_up_values()?;

                    let frame = self.current();
                    let (wanted, abs_func) = match frame.proto {
                        CallProto::Call { wanted, proto, .. } => (wanted, proto),
                        _ => {
                            let base = frame.get_base() as Address;
                            self.status = Dead;
                            return Ok(CoAction::Return(base, ValueCount::Exact(0)));
                        }
                    };
                    self.frames.pop().ok_or(NoCallFrame)?;

                    // Fill the caller's expected result slots with nil and
                    // trim the stack.
                    let n = if wanted == usize::MAX { 0 } else { wanted };
                    for i in 0..n {
                        self.stack[abs_func + i] = RuntimeValue::default();
                    }
                    self.adjust_stack(abs_func + n);
                    if let Some(b) = boundary {
                        if self.frames.len() == b {
                            return Ok(CoAction::Return(abs_func as Address, ValueCount::Exact(0)));
                        }
                    }
                    // Same pc bookkeeping as `Return`: the caller's pc was
                    // already advanced by the Call handler.
                    continue 'inst;
                }
                // Extra argument is before the target instruction
                ExtraArg(arg) => extra_arg = Some(arg),

                GetUpVal(a, i) => {
                    let val = match *vm!(UpVal(i)).borrow() {
                        UpValue::Closed(ref v) => v,
                        UpValue::Open(i) => self
                            .stack
                            .get(i)
                            .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?,
                    }
                    .clone();

                    vm!(R(a) := val);
                }
                SetUpVal(a, i) => {
                    let val = vm!(R(a)).clone();
                    let mut up_val = vm!(UpVal(i)).borrow_mut();
                    match *up_val {
                        UpValue::Open(idx) => {
                            self.stack[idx] = val;
                        }
                        UpValue::Closed(ref mut old_val) => *old_val = val,
                    }
                }

                GetTabUp(a, b, k) => {
                    let up_val = vm!(UpVal(b)).borrow();
                    let table = self.unpack_up_val(&up_val)?;
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let key = vm!(K(k));
                    let res = t.borrow().inner.get(&key).cloned().unwrap_or_default();
                    vm!(R(a) := res);
                }
                GetTable(a, b, c) => {
                    let table = vm!(R(b)).clone();
                    let key = vm!(R(c)).clone();
                    let res = self.get_table_field(heap, api, table, &key)?;
                    vm!(R(a) := res);
                }
                GetI(a, b, i) => {
                    let table = vm!(R(b)).clone();
                    let key = Int(i as DukaInt);
                    let res = self.get_table_field(heap, api, table, &key)?;
                    vm!(R(a) := res);
                }
                GetField(a, b, k) => {
                    let table = vm!(R(b)).clone();
                    let key = vm!(K(k));
                    let res = self.get_table_field(heap, api, table, &key)?;
                    vm!(R(a) := res);
                }
                SetTabUp(a, b, c, k) => {
                    let key = vm!(R(b)).clone();
                    let val = vm!(RK(c, k));

                    self.with_up_val(a as usize, |table| {
                        if let Table(t) = table {
                            t.borrow_mut().inner.insert(key, val);
                        }
                    })?;
                }
                SetTabUpK(a, idx, c, k) => {
                    let key = vm!(K(idx));
                    let val = vm!(RK(c, k));

                    self.with_up_val(a as usize, |table| {
                        if let Table(t) = table {
                            t.borrow_mut().inner.insert(key, val);
                        }
                    })?;
                }
                SetTabUpI(a, i, c, k) => {
                    let key = Int(i as DukaInt);
                    let val = vm!(RK(c, k));

                    self.with_up_val(a as usize, |table| {
                        if let Table(t) = table {
                            t.borrow_mut().inner.insert(key, val);
                        }
                    })?;
                }
                SetI(a, i, b, k) => {
                    let table = vm!(R(a)).clone();
                    let val = vm!(RK(b, k)).clone();
                    self.set_table_field(heap, api, table, Int(i as DukaInt), val)?;
                }
                // SetTable: 索引为R
                // SetField: 索引为K
                SetTable(a, b, c, k) => {
                    let val = vm!(RK(c, k)).clone();
                    let key = vm!(R(b)).clone();
                    let table = vm!(R(a)).clone();
                    self.set_table_field(heap, api, table, key, val)?;
                }
                SetField(a, b, c, k) => {
                    let val = vm!(RK(c, k)).clone();
                    let key = vm!(K(b)).clone();
                    let table = vm!(R(a)).clone();
                    self.set_table_field(heap, api, table, key, val)?;
                }
                NewTable(a) => {
                    let table = Table(heap.alloc(GcCell::new(RuntimeDukaTable::new(0))));
                    vm!(R(a) := table);
                }
                NewArray(a) => {
                    let array = Array(heap.alloc(GcCell::new(RuntimeDukaArray::new(0))));
                    vm!(R(a) := array);
                }
                Self_(a, b, c, k) => {
                    let table = vm!(R(b));
                    let key = vm!(RK(c, k));
                    (!key.is_string()).then_error(|| InvalidValueType(ctype::STR))?;
                    let Table(table) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let table_ref = table.borrow();
                    let func = table_ref
                        .inner
                        .get(&key)
                        .ok_or(NoSuchKey(key.eval_to_string().into_owned(), ctype::TAB))?;
                    (!func.is_function()).then_error(|| InvalidValueType(ctype::FUN))?;

                    vm!(R(a) := func.clone());
                    vm!(R(a + 1) := R(b));
                }
                AddI(a, b, n) => {
                    let (b, nv) = (vm!(R(b)).clone(), Int(n as DukaInt));
                    let r =
                        self.arith_meta(heap, api, &MetaMethod::Add, &b, &nv, |l, r| {
                            match (l, r) {
                                (Int(int), Int(r)) => Ok(Int(int.wrapping_add(*r))),
                                (Float(flt), Int(r)) => Ok(Float(*flt + (*r as DukaFloat))),
                                (Int(int), Float(r)) => Ok(Float(*int as DukaFloat + *r)),
                                (Float(flt), Float(r)) => Ok(Float(*flt + *r)),
                                _ => Err(InvalidValueType(ctype::NUM)),
                            }
                        })?;
                    vm!(R(a) := r);
                }
                AddK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Add, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_add, std::ops::Add::add)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                SubK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Sub, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_sub, std::ops::Sub::sub)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                MulK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Mul, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_mul, std::ops::Mul::mul)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                ModK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Mod, &b, &k, |l, r| {
                        if l.is_number() && r.is_number() {
                            check_zero(r)?;
                        }
                        unify_float(l, r)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Ints(a, b) => Int(a - floor_div(a, b) * b),
                                UnifiedNumber::Floats(a, b) => Float(a - (a / b).floor() * b),
                            })
                    })?;
                    vm!(R(a) := r);
                }
                PowK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Pow, &b, &k, |l, r| {
                        unify_float(l, r)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Floats(a, b) => Float(a.powf(b)),
                                UnifiedNumber::Ints(a, b) => Float((a as DukaFloat).powi(b as i32)),
                            })
                    })?;
                    vm!(R(a) := r);
                }
                DivK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::Div, &b, &k, |l, r| {
                        if l.is_number() && r.is_number() {
                            check_zero(r)?;
                        }
                        unify_float(l, r)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Floats(a, b) => Float(a / b),
                                UnifiedNumber::Ints(a, b) => Float(a as DukaFloat / b as DukaFloat),
                            })
                    })?;
                    vm!(R(a) := r);
                }
                IDivK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, api, &MetaMethod::IDiv, &b, &k, |l, r| {
                        if l.is_number() && r.is_number() {
                            check_zero(r)?;
                        }
                        unify_float(l, r)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Ints(a, b) => Int(floor_div(a, b)),
                                UnifiedNumber::Floats(a, b) => Float((a / b).floor()),
                            })
                    })?;
                    vm!(R(a) := r);
                }
                BitAndK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let result = self.bit_meta(heap, api, &MetaMethod::BAnd, &b, &k, |l, r| {
                        ari_bit(l, r, std::ops::BitAnd::bitand)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                BitOrK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let result = self.bit_meta(heap, api, &MetaMethod::BOr, &b, &k, |l, r| {
                        ari_bit(l, r, std::ops::BitOr::bitor)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                BitXorK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let result = self.bit_meta(heap, api, &MetaMethod::BXor, &b, &k, |l, r| {
                        ari_bit(l, r, std::ops::BitXor::bitxor)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                ShiftRI(a, b, i) => {
                    let b = vm!(R(b)).clone();
                    let amount = if i < 0 { -(i as DukaInt) } else { i as DukaInt };
                    let method = if i < 0 {
                        MetaMethod::ShL
                    } else {
                        MetaMethod::ShR
                    };
                    let nv = RuntimeValue::Int(amount);
                    let r = self.bit_meta(heap, api, &method, &b, &nv, |l, _r| {
                        let Int(b) = l else {
                            return Err(InvalidValueType(ctype::INT));
                        };
                        let r = if i < 0 {
                            b.wrapping_shl(amount as u32)
                        } else {
                            b.wrapping_shr(amount as u32)
                        };
                        Ok(Int(r))
                    })?;
                    vm!(R(a) := r);
                }

                EqualK(a, b, k, t) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let equal = if let Some(r) =
                        self.try_binary_meta_method(heap, api, &MetaMethod::Eq, &b, &k)?
                    {
                        r.eval_to_bool()
                    } else {
                        cmp_eq(&b, &k)?
                    };
                    vm!(R(a) := Bool(equal == t));
                }
                EqualI(a, b, i, t) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let equal = if let Some(r) =
                        self.try_binary_meta_method(heap, api, &MetaMethod::Eq, &n, &nv)?
                    {
                        r.eval_to_bool()
                    } else {
                        cmp_im(|x, y| x == y, |x, y| x == y, i as DukaInt)(&n).is_some_and(|v| v)
                    };
                    vm!(R(a) := Bool(equal == t));
                }
                LessI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, api, &MetaMethod::LT, &n, &nv, |l, _r| {
                        cmp_im(|x, y| x < y, |x, y| x < y, i as DukaInt)(l)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                LessEqualI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, api, &MetaMethod::LE, &n, &nv, |l, _r| {
                        cmp_im(|x, y| x <= y, |x, y| x <= y, i as DukaInt)(l)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                GreaterI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, api, &MetaMethod::LT, &nv, &n, |_l, r| {
                        cmp_mi(|x, y| x < y, |x, y| x < y, i as DukaInt)(r)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                GreaterEqualI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, api, &MetaMethod::LE, &nv, &n, |_l, r| {
                        cmp_mi(|x, y| x <= y, |x, y| x <= y, i as DukaInt)(r)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                SetList(list, start_index, count) => {
                    cast!(as list: usize, start_index: usize);
                    let count = if count == 0 {
                        vm!(@top).saturating_sub(vm!([list] for R) + vm!(@base))
                    } else {
                        count as usize
                    };

                    let mut values: Vec<RuntimeValue> = Vec::with_capacity(count);
                    for i in 1..count {
                        values.push(vm!(R(list + i)).clone());
                    }
                    match vm!(R(list)).clone() {
                        Table(t) => {
                            let mut t = t.borrow_mut();
                            for (o, val) in values.drain(..).enumerate() {
                                t.array_set(o + start_index, val);
                            }
                        }
                        Array(a) => {
                            let mut a = a.borrow_mut();
                            a.items.resize(start_index, RuntimeValue::Nil);
                            for val in values.drain(..) {
                                a.items.push(val);
                            }
                        }
                        _ => return Err(InvalidValueType(ctype::TAB)),
                    }
                }

                // When a duka function needs var_arg, this will appear at the start of function
                VarArgPrepare(fixed_param_count) => {
                    let end_of_params = vm!([fixed_param_count] for R) + vm!(@base);
                    let va = if end_of_params < vm!(@top) {
                        vm!(@stack:remove [end_of_params]..).collect()
                    } else {
                        Default::default()
                    };
                    vm!(@frame mut).var_args = va;
                }
                VarArg(ad, count_) => {
                    let n = match ValueCount::from(count_ as u32) {
                        ValueCount::VarArg => vm!(@frame).var_args.len(),
                        ValueCount::Exact(n) => n,
                    };
                    for o in 0..n {
                        let val = vm!(@frame).var_args.get(o).cloned().unwrap_or(Nil);

                        vm!(R(ad + o as Address) := val);
                    }
                }

                Go(co, from, count_) => {
                    let RuntimeValue::Coroutine(id) = vm!(R(co)).clone() else {
                        return Err(InvalidValueType("coroutine"));
                    };
                    let base = self.get_base();
                    let end = match ValueCount::from(count_) {
                        ValueCount::Exact(n) => ValueCount::Exact(n + base),
                        ValueCount::VarArg => ValueCount::VarArg,
                    };
                    vm!(continue);
                    return Ok(CoAction::Go(
                        id as CoroutineID,
                        (from as usize + base) as Address,
                        end,
                        (co - 1) as Address,
                    ));
                }
                Yield(from, params, results) => {
                    self.last_wanted = results as usize;
                    let base = self.get_base();
                    let end = match ValueCount::from(params) {
                        ValueCount::Exact(n) => ValueCount::Exact(n + base),
                        ValueCount::VarArg => ValueCount::VarArg,
                    };
                    self.resume_slot = Some(((from as usize + base) - 1) as u8);
                    vm!(continue);
                    return Ok(CoAction::Yield((from as usize + base) as Address, end));
                }
                Spawn(to, func) => {
                    vm!(continue);
                    return Ok(CoAction::Spawn(to, func));
                }
            }
            vm!(continue);
        }
    }

    fn close_up_values(&mut self) -> Result<(), DukaRuntimeError> {
        // Close every open upvalue whose slot lies inside the current frame
        // (`>= base`). Their values are copied into the shared cell, so
        // escaping closures keep working after the frame's slots are reused.
        let base = self.get_base();
        let slots: Vec<usize> = self
            .open_upvalues
            .keys()
            .copied()
            .filter(|k| *k >= base)
            .collect();
        for slot in slots {
            if let Some(cell) = self.open_upvalues.remove(&slot) {
                let mut cell = cell.borrow_mut();
                if let UpValue::Open(idx) = *cell {
                    let val = self.stack[idx].clone();
                    *cell = UpValue::Closed(val);
                }
            }
        }
        Ok(())
    }

    fn call_unary_meta_method(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        method: &MetaMethod,
        who: Gc<GcCell<RuntimeDukaTable>>,
    ) -> Result<Option<RuntimeValue>, DukaRuntimeError> {
        let Some(method) = who.borrow().get_meta_method(heap, method) else {
            return Ok(None);
        };
        if !method.is_function() {
            return Ok(None);
        }
        self.call_sync(heap, api, method, [RuntimeValue::Table(who)])
            .map(Some)
    }

    fn to_concat_string(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        val: RuntimeValue,
    ) -> Result<String, DukaRuntimeError> {
        match val {
            RuntimeValue::Table(t) => {
                let m = t.borrow().get_meta_method(heap, &MetaMethod::ToString);
                match m {
                    Some(m) if m.is_function() => Ok(self
                        .call_sync(heap, api, m, [RuntimeValue::Table(t)])?
                        .eval_to_string()
                        .into_owned()),
                    _ => Ok("table".to_owned()),
                }
            }
            v => Ok(v.eval_to_string().into_owned()),
        }
    }

    fn call_sync<const N: usize>(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        callee: RuntimeValue,
        params: [RuntimeValue; N],
    ) -> Result<RuntimeValue, DukaRuntimeError> {
        match &callee {
            RuntimeValue::UserFunc(..) => self.call_user_sync(heap, api, callee, &params),
            _ => {
                let pos = self.call_one_ret(heap, api, callee, params)?;
                Ok(self.get_stack(pos)?.clone())
            }
        }
    }

    pub(crate) fn call_user_sync(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        callee: RuntimeValue,
        params: &[RuntimeValue],
    ) -> Result<RuntimeValue, DukaRuntimeError> {
        let boundary = self.frames.len();
        let func_pos = self.stack.len() - self.get_base();
        self.append_stack(callee)?;
        for p in params {
            self.append_stack(p.clone())?;
        }
        self.call(
            heap,
            api,
            func_pos,
            ValueCount::Exact(params.len()),
            ValueCount::Exact(1),
            false,
        )?;
        match self.execute(heap, api, Some(boundary))? {
            CoAction::Return(from, _res) => Ok(self
                .stack
                .get(from as usize)
                .cloned()
                .unwrap_or(RuntimeValue::Nil)),
            _ => Err(DukaRuntimeError::UnsupportedOperation(
                "coroutine control in metamethod",
                ctype::FUN,
            )),
        }
    }

    pub(crate) fn protected_call(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        callee: RuntimeValue,
        params: &[RuntimeValue],
    ) -> Result<Result<Vec<RuntimeValue>, DukaRuntimeError>, DukaRuntimeError> {
        let boundary = self.frames.len();
        let startup = self.stack.len();
        let entry_base = self.get_base();
        let func_pos = self.stack.len() - entry_base;

        self.append_stack(callee)?;
        for p in params {
            self.append_stack(p.clone())?;
        }

        let result = match self.call(
            heap,
            api,
            func_pos,
            ValueCount::Exact(params.len()),
            ValueCount::VarArg,
            false,
        ) {
            Err(kind) => {
                self.adjust_stack(startup);
                self.frames.truncate(boundary);
                self.set_base(entry_base);
                Err(kind)
            }
            Ok(()) if self.frames.len() == boundary => {
                let has_pending = self.pending_action.take().is_some();
                let n = self.stack.len().saturating_sub(startup);
                let values = self.stack[startup..startup + n].to_vec();
                self.adjust_stack(startup);
                self.set_base(entry_base);
                if has_pending {
                    Err(DukaRuntimeError::UnsupportedOperation(
                        "coroutine control in protected call",
                        ctype::FUN,
                    ))
                } else {
                    Ok(values)
                }
            }
            Ok(()) => match self.execute(heap, api, Some(boundary)) {
                Err(kind) => {
                    let slots: Vec<usize> = self
                        .open_upvalues
                        .keys()
                        .copied()
                        .filter(|k| *k >= startup)
                        .collect();
                    for slot in slots {
                        if let Some(cell) = self.open_upvalues.remove(&slot) {
                            let mut cell = cell.borrow_mut();
                            if let UpValue::Open(idx) = *cell {
                                let val = self.stack[idx].clone();
                                *cell = UpValue::Closed(val);
                            }
                        }
                    }
                    self.adjust_stack(startup);
                    self.frames.truncate(boundary);
                    self.set_base(entry_base);
                    Err(kind)
                }
                Ok(CoAction::Return(from, count)) => {
                    let n = count.to_index(self.stack.len());
                    let from = from as usize;
                    let values = self.stack[from..from + n].to_vec();
                    self.adjust_stack(startup);
                    self.frames.truncate(boundary);
                    self.set_base(entry_base);
                    Ok(values)
                }
                Ok(_) => {
                    let slots: Vec<usize> = self
                        .open_upvalues
                        .keys()
                        .copied()
                        .filter(|k| *k >= startup)
                        .collect();
                    for slot in slots {
                        if let Some(cell) = self.open_upvalues.remove(&slot) {
                            let mut cell = cell.borrow_mut();
                            if let UpValue::Open(idx) = *cell {
                                let val = self.stack[idx].clone();
                                *cell = UpValue::Closed(val);
                            }
                        }
                    }
                    self.adjust_stack(startup);
                    self.frames.truncate(boundary);
                    self.set_base(entry_base);
                    Err(DukaRuntimeError::UnsupportedOperation(
                        "coroutine control in protected call",
                        ctype::FUN,
                    ))
                }
            },
        };

        match result {
            Ok(values) => Ok(Ok(values)),
            Err(kind) => Ok(Err(kind)),
        }
    }

    fn try_binary_meta_method(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        method: &MetaMethod,
        left: &RuntimeValue,
        right: &RuntimeValue,
    ) -> Result<Option<RuntimeValue>, DukaRuntimeError> {
        let has_table =
            matches!(left, RuntimeValue::Table(..)) || matches!(right, RuntimeValue::Table(..));
        if !has_table {
            return Ok(None);
        }
        let method = if let RuntimeValue::Table(t) = left {
            t.borrow().get_meta_method(heap, method)
        } else if let RuntimeValue::Table(t) = right {
            t.borrow().get_meta_method(heap, method)
        } else {
            return Ok(None);
        };
        let Some(method) = method else {
            return Ok(None);
        };
        if !method.is_function() {
            return Ok(None);
        }
        self.call_sync(heap, api, method, [left.clone(), right.clone()])
            .map(Some)
    }

    #[inline]
    fn arith_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        method: &MetaMethod,
        left: &RuntimeValue,
        right: &RuntimeValue,
        native: F,
    ) -> Result<RuntimeValue, DukaRuntimeError>
    where
        F: FnOnce(&RuntimeValue, &RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError>,
    {
        match native(left, right) {
            Ok(v) => Ok(v),
            Err(DukaRuntimeError::InvalidValueType(ctype::NUM)) => {
                if let Some(v) = self.try_binary_meta_method(heap, api, method, left, right)? {
                    Ok(v)
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::NUM))
                }
            }
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn bit_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        method: &MetaMethod,
        left: &RuntimeValue,
        right: &RuntimeValue,
        native: F,
    ) -> Result<RuntimeValue, DukaRuntimeError>
    where
        F: FnOnce(&RuntimeValue, &RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError>,
    {
        match native(left, right) {
            Ok(v) => Ok(v),
            Err(DukaRuntimeError::InvalidValueType(ctype::INT)) => {
                if let Some(v) = self.try_binary_meta_method(heap, api, method, left, right)? {
                    Ok(v)
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::INT))
                }
            }
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn compare_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        method: &MetaMethod,
        left: &RuntimeValue,
        right: &RuntimeValue,
        native: F,
    ) -> Result<bool, DukaRuntimeError>
    where
        F: FnOnce(&RuntimeValue, &RuntimeValue) -> Result<bool, DukaRuntimeError>,
    {
        match native(left, right) {
            Ok(b) => Ok(b),
            Err(DukaRuntimeError::InvalidValueType(ctype::NUM)) => {
                if let Some(v) = self.try_binary_meta_method(heap, api, method, left, right)? {
                    Ok(v.eval_to_bool())
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::NUM))
                }
            }
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn get_table_field(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        container: RuntimeValue,
        key: &RuntimeValue,
    ) -> Result<RuntimeValue, DukaRuntimeError> {
        match container {
            RuntimeValue::Table(tab) => self.get_table_field_inner(heap, api, tab, key),
            RuntimeValue::Array(arr) => {
                let arr_ref = arr.borrow();
                match key {
                    RuntimeValue::Int(i) => {
                        let len = arr_ref.len() as DukaInt;
                        let idx = if *i >= 0 { *i } else { len + *i };
                        if idx >= 0 {
                            Ok(arr_ref
                                .get(idx as usize)
                                .cloned()
                                .unwrap_or(RuntimeValue::Nil))
                        } else {
                            Ok(RuntimeValue::Nil)
                        }
                    }
                    _ => Ok(RuntimeValue::Nil),
                }
            }
            _ => Err(DukaRuntimeError::InvalidValueType(ctype::TAB)),
        }
    }

    fn get_table_field_inner(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        tab: Gc<GcCell<RuntimeDukaTable>>,
        key: &RuntimeValue,
    ) -> Result<RuntimeValue, DukaRuntimeError> {
        let mut seen: Vec<Gc<GcCell<RuntimeDukaTable>>> = Vec::new();
        let mut cur = tab;
        loop {
            if let Some(v) = cur.borrow().inner.get(key) {
                return Ok(v.clone());
            }
            if seen.contains(&cur) {
                return Ok(RuntimeValue::Nil);
            }
            seen.push(cur);
            match cur.borrow().get_meta_method(heap, &MetaMethod::Index) {
                Some(RuntimeValue::Table(fallback)) => cur = fallback,
                Some(m) if m.is_function() => {
                    return self.call_sync(heap, api, m, [RuntimeValue::Table(cur), key.clone()]);
                }
                _ => return Ok(RuntimeValue::Nil),
            }
        }
    }

    #[inline]
    fn set_table_field(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        container: RuntimeValue,
        key: RuntimeValue,
        val: RuntimeValue,
    ) -> Result<(), DukaRuntimeError> {
        match container {
            RuntimeValue::Array(arr) => {
                let RuntimeValue::Int(i) = key else {
                    return Ok(());
                };
                let len = arr.borrow().len() as DukaInt;
                let idx = if i >= 0 { i } else { len + i };
                if idx < 0 {
                    return Ok(());
                }
                arr.borrow_mut().set(idx as usize, val);
                Ok(())
            }
            RuntimeValue::Table(tab) => self.set_table_field_inner(heap, api, tab, key, val),
            _ => Err(DukaRuntimeError::InvalidValueType(ctype::TAB)),
        }
    }

    fn set_table_field_inner(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        tab: Gc<GcCell<RuntimeDukaTable>>,
        key: RuntimeValue,
        val: RuntimeValue,
    ) -> Result<(), DukaRuntimeError> {
        let existed = tab
            .borrow_mut()
            .inner
            .insert(key.clone(), val.clone())
            .is_some();
        if existed {
            return Ok(());
        }
        if let Some(mt) = tab.borrow().metatable {
            let m = mt
                .borrow()
                .get(&RuntimeValue::meta_method_key(heap, &MetaMethod::NewIndex))
                .cloned();
            match m {
                Some(RuntimeValue::Table(fallback)) => {
                    tab.borrow_mut().inner.remove(&key);
                    fallback.borrow_mut().inner.insert(key, val);
                    return Ok(());
                }
                Some(m) if m.is_function() => {
                    tab.borrow_mut().inner.remove(&key);
                    self.call_sync(heap, api, m, [RuntimeValue::Table(tab), key, val])?;
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn call_one_ret<const N: usize>(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        callee: RuntimeValue,
        params: [RuntimeValue; N],
    ) -> Result<usize, DukaRuntimeError> {
        let func_pos = self.stack.len() - self.get_base();

        self.append_stack(callee)?;
        for param in params {
            self.append_stack(param)?;
        }

        self.call(
            heap,
            api,
            func_pos,
            ValueCount::Exact(2),
            ValueCount::Exact(1),
            false,
        )?;

        Ok(func_pos)
    }

    pub fn call(
        &mut self,
        heap: &mut duka_gc::Heap,
        api: &mut NativeApi,
        func: usize,
        narg: ValueCount,
        nwanted: ValueCount,
        tailcall: bool,
    ) -> Result<(), DukaRuntimeError> {
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        let mut narg = narg.clone();
        let callee = self.get_stack(func)?.clone();
        let callee = match callee {
            RuntimeValue::Table(t) => {
                let Some(method) = t.borrow().get_meta_method(heap, &MetaMethod::Call) else {
                    return Err(InvalidValueType(ctype::FUN));
                };
                let base = self.get_base();
                let n = match narg {
                    ValueCount::Exact(a) => a,
                    ValueCount::VarArg => self.stack.len().saturating_sub(func + base + 1),
                };
                let abs = func + base + 1;
                if self.stack.len() >= abs {
                    self.stack.insert(abs, RuntimeValue::Table(t));
                } else {
                    self.stack.resize(abs, RuntimeValue::default());
                    self.stack.push(RuntimeValue::Table(t));
                }
                self.stack[func + base] = method;
                narg = ValueCount::Exact(n + 1);
                self.stack[func + base].clone()
            }
            callee => callee.clone(),
        };
        if !callee.is_function() {
            return Err(InvalidValueType(ctype::FUN));
        }
        let base = self.get_base();

        // 调用前把栈裁剪到实参末尾
        if let ValueCount::Exact(a) = narg {
            self.adjust_stack(func + base + 1 + a);
        }

        match callee {
            NativeFunc(closure) => {
                let mut ptr = closure.borrow_mut();

                // Native functions read args from `base+1..` and write results
                // at `R0..`, so the frame base is the callee slot itself.
                self.set_base(func + base);

                let nreturn = match (ptr.func)(self, heap, api)? {
                    ValueCount::VarArg => self.stack.len() - (func + base),
                    ValueCount::Exact(n) => n,
                };
                if let Some(action) = api.take_pending() {
                    self.pending_action = Some(action);
                }

                let raw_wanted = match nwanted {
                    ValueCount::VarArg => nreturn,
                    ValueCount::Exact(n) => n,
                };
                let keep = raw_wanted.max(nreturn);
                if self.stack.len() < func + base + keep {
                    self.stack
                        .resize_with(func + base + keep, RuntimeValue::default);
                }
                if nreturn < raw_wanted {
                    let from = func + base + nreturn;
                    for i in 0..raw_wanted - nreturn {
                        self.stack[from + i] = RuntimeValue::default();
                    }
                }

                // Results are at `func..func+keep`; truncate above them,
                // keeping the live registers below `func`.
                self.adjust_stack(func + base + keep);
                self.set_base(base);
            }
            UserFunc(closure) => {
                let fixed_count = closure.func.param_count;
                let has_var_arg = closure.func.has_var_arg;

                if tailcall {
                    self.close_up_values()?;
                    self.cut_stack(base - 1, ValueCount::Exact(base + func));
                    let wanted = match nwanted {
                        ValueCount::Exact(n) => n,
                        ValueCount::VarArg => usize::MAX,
                    };
                    if let ValueCount::Exact(a) = narg {
                        if a < fixed_count {
                            let count = fixed_count - a;
                            for i in 0..count {
                                self.set_stack(a + i, Nil)?;
                            }
                        } else if a > fixed_count && !has_var_arg {
                            self.adjust_stack(base + a);
                        }
                    }
                    let frame = CallFrame::call(base, base - 1, wanted);
                    *self.current_mut() = frame;
                    return Ok(());
                }

                if let ValueCount::Exact(a) = narg {
                    if a < fixed_count {
                        // Pad missing params with nil; params live at
                        // `func+1..` (the callee's frame base).
                        let from = func + a + 1;
                        let count = fixed_count - a;
                        for i in 0..count {
                            self.set_stack(i + from, Nil)?;
                        }
                    } else if a > fixed_count && !has_var_arg {
                        let len = func + a + 1 + base;
                        self.adjust_stack(len);
                    }
                }

                // The callee frame base is the first arg slot (`func+1`),
                // so params map to R0..Rn-1; `proto` is the closure slot.
                // `wanted` is the real result count the caller expects.
                let wanted = match nwanted {
                    ValueCount::Exact(n) => n,
                    // VarArg means the caller takes however many results
                    // the callee produces.
                    ValueCount::VarArg => usize::MAX,
                };
                let frame = CallFrame::call(func + base + 1, func + base, wanted);
                let needed = func + base + 1 + closure.func.used_reg_count;
                if self.stack.len() < needed {
                    self.stack.resize_with(needed, RuntimeValue::default);
                }
                self.push_frame(frame);
            }
            _ => unreachable!(),
        };
        Ok(())
    }
}
