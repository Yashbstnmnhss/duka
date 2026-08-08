use std::{
    cmp::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

use hashbrown::HashMap;
use rustc_hash::FxBuildHasher;

use crate::{
    SysCallId,
    errors::{DukaRuntimeError, DukaStackTrace, DukaTraceFrame},
    instructions::{Address, DecodeInstruction, Instruction},
    value::{
        DukaClosure, DukaProto, RuntimeDukaTable, RuntimeValue, RustClosure, UpValue,
        make_pairs_iterator, make_values_iterator,
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
    /// 协程的值栈
    pub stack: Stack,
    /// 调用帧
    pub frames: Vec<CallFrame>,
    /// Open upvalues per absolute stack slot, so escaping closures keep
    /// sharing one cell and slots are closed when their frame returns.
    pub open_upvalues: HashMap<usize, Gc<GcCell<UpValue>>, FxBuildHasher>,
    pub rng_state: u32,
    pub(crate) trace_pending: Vec<DukaTraceFrame>,
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
        frames.extend(self.trace_pending.iter().cloned());
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
            trace_pending: vec![],
        }
    }
    #[inline(always)]
    pub fn with_closure(closure: Gc<DukaClosure>) -> Self {
        Self {
            stack: Vec::with_capacity(closure.func.used_reg_count),
            frames: vec![CallFrame::main(closure)],
            open_upvalues: HashMap::with_capacity_and_hasher(0, FxBuildHasher),
            rng_state: 171912,
            trace_pending: vec![],
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
        let cur = self.current();
        let proto = match &cur.proto {
            CallProto::Main { proto, .. } => proto,
            // `proto` stores the callee's absolute stack slot (its frame base
            // sits one above at `base`), so it must NOT go through the base
            // offsetting of `get_stack`.
            CallProto::Call { proto, .. } => match self.stack.get(*proto) {
                Some(RuntimeValue::UserFunc(p)) => p,
                _ => return Err(DukaRuntimeError::InvalidValueType(ctype::PRO)),
            },
        };
        Ok(&proto.func.instructions[cur.pc])
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
    /// **含偏移**
    pub fn ensure_address(&self, ad: usize) -> bool {
        self.stack.len() > ad + self.get_base()
    }
    /// 获取栈上的值 **含base偏移**
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
    /// 设置栈上的值 **含base偏移**
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

/// # 协程状态
#[derive(Debug, Info)]
pub enum CoroutineStatus {
    /// 准备完毕
    #[tag(go_able)]
    Ready,
    /// 正在运行
    Running,
    /// 已经挂起
    #[tag(go_able)]
    Suspended,
    /// 已经结束
    Dead,
}

/// # 协程
#[derive(Debug)]
pub struct Coroutine {
    pub id: CoroutineID,
    pub status: CoroutineStatus,
    pub inner: CoState,
    pub parent: Option<CoroutineID>,

    pub(super) last_wanted: usize,
}

/// 同步执行用户函数元方法：用临时子协程执行返回单个结果值。供指令与 builtin共用。
pub(crate) fn sync_meta_call(
    parent: &mut CoState,
    heap: &mut duka_gc::Heap,
    closure: Gc<DukaClosure>,
    params: &[RuntimeValue],
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut up_values = Vec::with_capacity(closure.up_values.len());
    for uv in &closure.up_values {
        let snap = match &*uv.borrow() {
            UpValue::Closed(v) => UpValue::Closed(v.clone()),
            UpValue::Open(slot) => UpValue::Closed(
                parent
                    .stack
                    .get(*slot)
                    .cloned()
                    .unwrap_or(RuntimeValue::Nil),
            ),
        };
        up_values.push(heap.alloc(GcCell::new(snap)));
    }
    let child_closure = heap.alloc(DukaClosure {
        func: closure.func,
        up_values,
    });
    // 临时子携程, 不可控制
    let mut child = Coroutine::new(0, CoState::with_closure(child_closure), None);
    for p in params {
        child.inner.append_stack(p.clone())?;
    }
    let need = closure.func.used_reg_count.max(params.len());
    child.inner.stack.resize_with(need, RuntimeValue::default);

    let count = match child.execute(heap) {
        Ok(CoAction::Return(_, n)) => n,
        Err(e) => {
            // 子协程帧比父协程更深，错误时把其帧链转入父的 pending
            let mut trace = child.inner.create_trace();
            parent.trace_pending.append(&mut trace.frames);
            return Err(e);
        }
        _ => {
            return Err(DukaRuntimeError::UnsupportedOperation(
                "coroutine control in metamethod",
                ctype::FUN,
            ));
        }
    };
    let mut state = std::mem::take(&mut child.inner);
    let vals = state.take_stack_many(0, count)?;
    Ok(vals.into_iter().next().unwrap_or(RuntimeValue::Nil))
}

pub(crate) fn call_native_meta_sync(
    sv: &mut CoState,
    heap: &mut duka_gc::Heap,
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
    (closure.borrow_mut().func)(sv, heap)?;
    let results = std::mem::take(&mut sv.stack);
    sv.stack = saved_stack;
    sv.set_base(saved_base);
    Ok(results.into_iter().next().unwrap_or(RuntimeValue::Nil))
}

impl Coroutine {
    #[inline(always)]
    pub fn new(id: CoroutineID, state: CoState, parent: Option<CoroutineID>) -> Self {
        Self {
            id,
            status: CoroutineStatus::Ready,
            inner: state,
            parent,

            last_wanted: 0,
        }
    }

    /// ### Push a frame of calling into this coroutine
    pub fn push_frame(&mut self, frame: CallFrame) {
        self.inner.push_frame(frame);
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
impl Coroutine {
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
                .inner
                .stack
                .get(*i)
                .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?,
        })
    }
    fn with_up_val<F, R>(&mut self, up_val_idx: usize, f: F) -> Result<R, DukaRuntimeError>
    where
        F: FnOnce(&mut RuntimeValue) -> R,
    {
        let mut borrow = self.inner.get_up_value(up_val_idx)?.borrow_mut();
        Ok(match *borrow {
            UpValue::Closed(ref mut v) => f(v),
            UpValue::Open(i) => {
                let val = self
                    .inner
                    .stack
                    .get_mut(i)
                    .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))?;
                f(val)
            }
        })
    }

    pub fn reset(&mut self) {
        self.inner.stack.clear();
        self.inner.frames.clear();
        self.inner.open_upvalues.clear();
        self.inner.trace_pending.clear();
        self.last_wanted = 0;

        self.status = CoroutineStatus::Ready;
    }

    /// ### Where instructions are executed exactly
    pub fn execute(
        &mut self,
        //ctx: &mut VMContext,
        heap: &mut duka_gc::Heap,
    ) -> Result<CoAction, DukaRuntimeError> {
        use CoroutineStatus::*;
        use DecodeInstruction::*;
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        (!self.status.is_go_able()).then_error(|| UnableRunCoroutine(self.id))?;

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
                self.inner.stack.drain($start as usize..$end as usize)
            };
            // drop the tail
            (@stack:remove [$end: expr]..) => {
                self.inner.stack.drain($end as usize..)
            };

            /* getter */
            (@frame) => {
                self.inner.current()
            };
            (@frame mut) => {
                self.inner.current_mut()
            };
            (@top) => {
                self.inner.stack.len()
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
                self.inner.get_up_value($i as usize)?
            };
            (UpVal($i: expr) := $v: expr) => {
                self.inner.get_closure()?.up_values.set($i as usize).and_then(|u| u.get_value()).ok_or(OutOfUpvalue)?
            };

            /* read *HAS BASE */
            (R($ad: expr; $ct: expr) $(@get)?) => {
                (0..$ct as usize).map(|i| self.inner.get_stack(vm!([$ad as usize + i] for R))).collect::<Result<Vec<_>, _>>()?
            };
            (R($ad: expr) $(@get)?) => {
                self.inner.get_stack(vm!([$ad] for R))?
            };
            (K($i: expr) $(@get)?) => {{
                let proto = self.inner.get_closure()?.func;
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
                self.inner.set_stack(vm!([$a] for R), $v)?;
            };
        }

        'inst: loop {
            let inst = self.inner.fetch()?;

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
                        self.arith_meta(heap, &MetaMethod::Add, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_add, std::ops::Add::add)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Sub(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, &MetaMethod::Sub, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_sub, std::ops::Sub::sub)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Mul(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.arith_meta(heap, &MetaMethod::Mul, &left, &right, |l, r| {
                            ari(l, r, DukaInt::wrapping_mul, std::ops::Mul::mul)
                                .ok_or(InvalidValueType(ctype::NUM))
                        })?;
                    vm!(R(a) := result);
                }
                Div(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    // `/` 恒浮点除:整数相除也产生 Float。
                    let result =
                        self.arith_meta(heap, &MetaMethod::Div, &left, &right, |l, r| {
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
                    // `//` floor 除:整数/整数得整数(向下取整),否则浮点。
                    let result =
                        self.arith_meta(heap, &MetaMethod::IDiv, &left, &right, |l, r| {
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
                    // `%` floor 取模:满足 a == a//b*b + a%b。
                    let result =
                        self.arith_meta(heap, &MetaMethod::Mod, &left, &right, |l, r| {
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
                        self.arith_meta(heap, &MetaMethod::Pow, &left, &right, |l, r| {
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
                        self.bit_meta(heap, &MetaMethod::BAnd, &left, &right, |l, r| {
                            ari_bit(l, r, std::ops::BitAnd::bitand)
                                .map(Int)
                                .ok_or(InvalidValueType(ctype::INT))
                        })?;
                    vm!(R(a) := result);
                }
                BitOr(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result = self.bit_meta(heap, &MetaMethod::BOr, &left, &right, |l, r| {
                        ari_bit(l, r, std::ops::BitOr::bitor)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                BitXor(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result =
                        self.bit_meta(heap, &MetaMethod::BXor, &left, &right, |l, r| {
                            ari_bit(l, r, std::ops::BitXor::bitxor)
                                .map(Int)
                                .ok_or(InvalidValueType(ctype::INT))
                        })?;
                    vm!(R(a) := result);
                }
                ShiftL(a, b, c) => {
                    let (left, right) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let result = self.bit_meta(heap, &MetaMethod::ShL, &left, &right, |l, r| {
                        let (Int(l), Int(r)) = (l, r) else {
                            return Err(InvalidValueType(ctype::INT));
                        };
                        // 负移位数反向:<< -n 等价于 >> n。
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
                    let result = self.bit_meta(heap, &MetaMethod::ShR, &left, &right, |l, r| {
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
                        self.try_binary_meta_method(heap, &MetaMethod::Eq, &b, &c)?
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
                        self.compare_meta(heap, &MetaMethod::LT, &b, &c, |l, r| cmp_lt(l, r))?;
                    vm!(R(a) := Bool(r));
                }
                LessEqual(a, b, c) => {
                    let (b, c) = (vm!(R(b)).clone(), vm!(R(c)).clone());
                    let r =
                        self.compare_meta(heap, &MetaMethod::LE, &b, &c, |l, r| cmp_le(l, r))?;
                    vm!(R(a) := Bool(r));
                }
                Concat(a, count) => {
                    // 先扫描操作数：有带 function __concat 的 Table 走元方法左折叠 否则走纯字符串拼接(Table 带 __tostring 时用)
                    let mut has_concat = false;
                    let mut has_to_string = false;
                    for i in 0..count as usize {
                        if let Table(t) = self.inner.get_stack(a as usize + i)? {
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
                        // __concat 左折叠：acc = fold(acc, next) 任一侧带 function __concat 则同步调用，否则按 __tostring 规则字符串拼接
                        let mut acc = self.inner.get_stack(a as usize)?.clone();
                        for i in 1..count as usize {
                            let next = self.inner.get_stack(a as usize + i)?.clone();
                            let meta = match (&acc, &next) {
                                (Table(t), _) | (_, Table(t)) => {
                                    t.borrow().get_meta_method(heap, &MetaMethod::Concat)
                                }
                                _ => None,
                            };
                            acc = if let Some(m) = meta.filter(|m| m.is_function()) {
                                self.call_sync(heap, m, [acc, next])?
                            } else {
                                // strcat性能问题:
                                let s = format!(
                                    "{}{}",
                                    self.to_concat_string(heap, acc)?,
                                    self.to_concat_string(heap, next)?
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
                                let val = self.inner.get_stack(a as usize + i)?.clone();
                                let s = match val {
                                    Table(_) => self.to_concat_string(heap, val)?,
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
                                self.call_unary_meta_method(heap, &MetaMethod::Unm, t)?
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
                        && let Some(r) = self.call_unary_meta_method(heap, &MetaMethod::BNot, *t)?
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
                                self.call_unary_meta_method(heap, &MetaMethod::Len, t)?
                            {
                                vm!(R(a) := r);
                            } else {
                                let b = t.borrow();
                                vm!(R(a) := Int(b.len() as DukaInt));
                            }
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
                                continue; // 已 move,不再 vm!(continue)
                            } else {
                                vm!(R(a + 1) := Int(limit));
                                // then loop
                            }
                        } else {
                            vm!(move end_offset); // this will move to the last code of inner block
                            continue; // 已 move,不再 vm!(continue)
                        }
                    } else {
                        let init = cast!(Number use eval_to_float for vm!(R(a)))?;
                        let limit = cast!(Number use eval_to_float for vm!(R(a + 1)))?;
                        let step = cast!(Number use eval_to_float for vm!(R(a + 2)))?;

                        (step == 0.0).then_error(|| ZeroStepInForLoop)?;

                        if !for_number_check(init, limit, step.is_sign_negative()) {
                            vm!(move end_offset);
                            continue; // 已 move,不再 vm!(continue)
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
                            continue; // 已 move,不再 vm!(continue)
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
                            continue; // 已 move,不再 vm!(continue)
                        }
                    }
                }

                TForPrepare(_, offset) => {
                    cast!(as offset: isize);
                    // 首轮直接跳到 TForCall 取第一个值
                    vm!(move offset);
                    continue;
                }
                TForCall(a, nres) => {
                    cast!(as nres: usize, a: usize);
                    // 糖:`for x in <table>` 直接遍历表 —— 首值若是表,自动套迭代器。
                    // 一次性把 R(a..a+2) 替换为 (iter, t, nil):单变量用值迭代器,
                    // 双变量及以上用 pairs(k, v)。替换后 R(a) 是函数,后续轮不再触发。
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
                        a + 3,
                        ValueCount::Exact(2),
                        ValueCount::Exact(nres),
                        false,
                    )?;
                    continue 'inst;
                }
                TForLoop(a, offset) => {
                    cast!(as offset: isize);

                    let res = vm!(R(a + 3)).clone();
                    if !matches!(res, Nil) {
                        vm!(R(a + 2) := R(a + 3));
                        vm!(move -offset);
                        continue;
                    }
                }

                Closure(ad, index) => {
                    cast!(as index: usize);
                    // push closure to stack & initialize its up_values

                    let proto = self
                        .inner
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
                            match self.inner.open_upvalues.get(&slot) {
                                Some(existing) => *existing,
                                None => {
                                    let cell = heap.alloc(GcCell::new(UpValue::Open(slot)));
                                    self.inner.open_upvalues.insert(slot, cell);
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
                    // callee from its first instruction (the loop's trailing
                    // `vm!(continue)` must not touch the new frame).
                    vm!(move 1);
                    self.call(heap, func, narg.into(), nwanted.into(), false)?;
                    continue 'inst;
                }
                TailCall(func, narg, nwanted) => {
                    cast!(as func: usize);
                    self.call(heap, func, narg.into(), nwanted.into(), true)?;
                    // call() 已替换当前帧(pc=0),尾部的 vm!(continue) 不得再 +1
                    continue 'inst;
                }

                SysCall(syscall, narg, _nwanted) => {
                    let id = SysCallId::from_disc(syscall)
                        .map_err(|_| NoSuchKey(syscall.to_string(), "syscall"))?;
                    match id {
                        SysCallId::Logic => {
                            let closure = self.inner.get_closure()?;
                            if let Some(ref logic_proto) = closure.func.logic {
                                let query_idx = narg as usize;
                                let solutions =
                                    crate::vm::logic::execute_query(logic_proto, query_idx)
                                        .map_err(|e| Custom(e))?;
                                let table = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
                                for (i, sol) in solutions.iter().enumerate() {
                                    let entry = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
                                    let mut keys: Vec<&usize> = sol.keys().collect();
                                    keys.sort();
                                    for (j, k) in keys.iter().enumerate() {
                                        let val = RuntimeValue::from_string(heap, sol[k].clone());
                                        entry
                                            .borrow_mut()
                                            .set(RuntimeValue::Int((j + 1) as i64), val);
                                    }
                                    table.borrow_mut().set(
                                        RuntimeValue::Int((i + 1) as i64),
                                        RuntimeValue::Table(entry),
                                    );
                                }
                                let result = RuntimeValue::Table(table);
                                vm!(R(syscall as Address) := result);
                            }
                        } //_ => unimplemented!(),
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

                    let CallProto::Call { wanted, proto, .. } = self.inner.current().proto else {
                        self.status = Dead;
                        // The main chunk may leave its results at register
                        // `from` (e.g. a bare `return f()` keeps them at the
                        // callee slot); compact them down to stack position 0
                        // so `VM::run`/`run_take` read the real results.
                        let src = self.inner.get_base() + from;
                        let stack = &mut self.inner.stack;
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
                    self.inner.frames.pop().ok_or(NoCallFrame)?;

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
                            self.inner.stack.get(src + i).cloned().unwrap_or_default()
                        } else {
                            RuntimeValue::default()
                        };
                        self.inner.stack[dst + i] = val;
                    }
                    self.inner.adjust_stack(dst + total);
                    // The Call handler already advanced the caller's pc past
                    // the call, so the loop's trailing `vm!(continue)` must
                    // not touch it again.
                    continue 'inst;
                }
                Return0() => {
                    self.close_up_values()?;

                    let frame = self.inner.current();
                    let (wanted, abs_func) = match frame.proto {
                        CallProto::Call { wanted, proto, .. } => (wanted, proto),
                        _ => {
                            self.status = Dead;
                            return Ok(CoAction::Return(
                                frame.get_base() as Address,
                                ValueCount::Exact(0),
                            ));
                        }
                    };
                    self.inner.frames.pop().ok_or(NoCallFrame)?;

                    // Fill the caller's expected result slots with nil and
                    // trim the stack.
                    let n = if wanted == usize::MAX { 0 } else { wanted };
                    for i in 0..n {
                        self.inner.stack[abs_func + i] = RuntimeValue::default();
                    }
                    self.inner.adjust_stack(abs_func + n);
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
                            .inner
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
                            self.inner.stack[idx] = val;
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
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let t = *t;
                    let key = vm!(R(c)).clone();
                    let res = self.get_table_field(heap, t, &key)?;
                    vm!(R(a) := res);
                }
                GetI(a, b, i) => {
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let t = *t;
                    let key = Int(i as DukaInt);
                    let res = self.get_table_field(heap, t, &key)?;
                    vm!(R(a) := res);
                }
                GetField(a, b, k) => {
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let t = *t;
                    let key = vm!(K(k));
                    let res = self.get_table_field(heap, t, &key)?;
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
                    let table = vm!(R(a));
                    let val = vm!(RK(b, k));
                    if let Table(t) = table {
                        self.set_table_field(heap, *t, Int(i as DukaInt), val)?;
                    }
                }
                // SetTable: 索引为R
                // SetField: 索引为K
                SetTable(a, b, c, k) => {
                    let val = vm!(RK(c, k));
                    let key = vm!(R(b)).clone();
                    let table = vm!(R(a));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    self.set_table_field(heap, *t, key, val)?;
                }
                SetField(a, b, c, k) => {
                    let val = vm!(RK(c, k));
                    let key = vm!(K(b));
                    let table = vm!(R(a));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    self.set_table_field(heap, *t, key, val)?;
                }
                NewTable(a) => {
                    let table = Table(heap.alloc(GcCell::new(RuntimeDukaTable::new(0))));
                    vm!(R(a) := table);
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
                        self.arith_meta(heap, &MetaMethod::Add, &b, &nv, |l, r| match (l, r) {
                            (Int(int), Int(r)) => Ok(Int(int.wrapping_add(*r))),
                            (Float(flt), Int(r)) => Ok(Float(*flt + (*r as DukaFloat))),
                            (Int(int), Float(r)) => Ok(Float(*int as DukaFloat + *r)),
                            (Float(flt), Float(r)) => Ok(Float(*flt + *r)),
                            _ => Err(InvalidValueType(ctype::NUM)),
                        })?;
                    vm!(R(a) := r);
                }
                AddK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, &MetaMethod::Add, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_add, std::ops::Add::add)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                SubK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, &MetaMethod::Sub, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_sub, std::ops::Sub::sub)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                MulK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let r = self.arith_meta(heap, &MetaMethod::Mul, &b, &k, |l, r| {
                        ari(l, r, DukaInt::wrapping_mul, std::ops::Mul::mul)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := r);
                }
                ModK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    // `%` floor 取模:满足 a == a//b*b + a%b。
                    let r = self.arith_meta(heap, &MetaMethod::Mod, &b, &k, |l, r| {
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
                    let r = self.arith_meta(heap, &MetaMethod::Pow, &b, &k, |l, r| {
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
                    let r = self.arith_meta(heap, &MetaMethod::Div, &b, &k, |l, r| {
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
                    let r = self.arith_meta(heap, &MetaMethod::IDiv, &b, &k, |l, r| {
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
                    let result = self.bit_meta(heap, &MetaMethod::BAnd, &b, &k, |l, r| {
                        ari_bit(l, r, std::ops::BitAnd::bitand)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                BitOrK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let result = self.bit_meta(heap, &MetaMethod::BOr, &b, &k, |l, r| {
                        ari_bit(l, r, std::ops::BitOr::bitor)
                            .map(Int)
                            .ok_or(InvalidValueType(ctype::INT))
                    })?;
                    vm!(R(a) := result);
                }
                BitXorK(a, b, k) => {
                    let (b, k) = (vm!(R(b)).clone(), vm!(K(k)));
                    let result = self.bit_meta(heap, &MetaMethod::BXor, &b, &k, |l, r| {
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
                    let r = self.bit_meta(heap, &method, &b, &nv, |l, _r| {
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
                        self.try_binary_meta_method(heap, &MetaMethod::Eq, &b, &k)?
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
                        self.try_binary_meta_method(heap, &MetaMethod::Eq, &n, &nv)?
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
                    let r = self.compare_meta(heap, &MetaMethod::LT, &n, &nv, |l, _r| {
                        cmp_im(|x, y| x < y, |x, y| x < y, i as DukaInt)(l)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                LessEqualI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, &MetaMethod::LE, &n, &nv, |l, _r| {
                        cmp_im(|x, y| x <= y, |x, y| x <= y, i as DukaInt)(l)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                GreaterI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, &MetaMethod::LT, &nv, &n, |_l, r| {
                        cmp_mi(|x, y| x < y, |x, y| x < y, i as DukaInt)(r)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                GreaterEqualI(a, b, i) => {
                    let n = vm!(R(b)).clone();
                    let nv = RuntimeValue::Int(i as DukaInt);
                    let r = self.compare_meta(heap, &MetaMethod::LE, &nv, &n, |_l, r| {
                        cmp_mi(|x, y| x <= y, |x, y| x <= y, i as DukaInt)(r)
                            .ok_or(InvalidValueType(ctype::NUM))
                    })?;
                    vm!(R(a) := Bool(r));
                }
                SetList(list, start_index, count) => {
                    cast!(as list: usize, start_index: usize);
                    // count 编码:0 => VarArg(按栈顶计算,含 table 自身寄存器),
                    // N => 精确 N 个寄存器(同样含 table)。`{...}` 场景下
                    let count = if count == 0 {
                        vm!(@top).saturating_sub(vm!([list] for R) + vm!(@base))
                    } else {
                        count as usize
                    };

                    let mut table = match vm!(R(list)) {
                        Table(t) => t.borrow_mut(),
                        _ => return Err(InvalidValueType(ctype::TAB)),
                    };
                    for i in 1..count {
                        let val = vm!(R(list + i)).clone();
                        table.array_set(i + start_index - 1, val);
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
                    // count_==0 编码 VarArg:展开全部变长实参。
                    // 不能用 `top - (ad+base)` 计算:VarArgPrepare 已把实参从栈上移走。
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
                    return Ok(CoAction::Go(co as CoroutineID, from, count_.into()));
                }
                Yield(from, params, results) => {
                    self.last_wanted = results as usize;
                    return Ok(CoAction::Yield(from, params.into()));
                }
                Spawn(to, func) => return Ok(CoAction::Spawn(to, func)),
            }
            vm!(continue);
        }
    }

    fn close_up_values(&mut self) -> Result<(), DukaRuntimeError> {
        // Close every open upvalue whose slot lies inside the current frame
        // (`>= base`). Their values are copied into the shared cell, so
        // escaping closures keep working after the frame's slots are reused.
        let base = self.inner.get_base();
        let slots: Vec<usize> = self
            .inner
            .open_upvalues
            .keys()
            .copied()
            .filter(|k| *k >= base)
            .collect();
        for slot in slots {
            if let Some(cell) = self.inner.open_upvalues.remove(&slot) {
                let mut cell = cell.borrow_mut();
                if let UpValue::Open(idx) = *cell {
                    let val = self.inner.stack[idx].clone();
                    *cell = UpValue::Closed(val);
                }
            }
        }
        Ok(())
    }

    fn call_unary_meta_method(
        &mut self,
        heap: &mut duka_gc::Heap,
        method: &MetaMethod,
        who: Gc<GcCell<RuntimeDukaTable>>,
    ) -> Result<Option<RuntimeValue>, DukaRuntimeError> {
        let Some(method) = who.borrow().get_meta_method(heap, method) else {
            return Ok(None);
        };
        if !method.is_function() {
            return Ok(None);
        }
        self.call_sync(heap, method, [RuntimeValue::Table(who)])
            .map(Some)
    }

    /// 将值转为拼接字符串：Table 带 function __tostring 时同步调用，否则默认格式化。
    fn to_concat_string(
        &mut self,
        heap: &mut duka_gc::Heap,
        val: RuntimeValue,
    ) -> Result<String, DukaRuntimeError> {
        match val {
            RuntimeValue::Table(t) => {
                let m = t.borrow().get_meta_method(heap, &MetaMethod::ToString);
                match m {
                    Some(m) if m.is_function() => Ok(self
                        .call_sync(heap, m, [RuntimeValue::Table(t)])?
                        .eval_to_string()
                        .into_owned()),
                    _ => Ok("table".to_owned()),
                }
            }
            v => Ok(v.eval_to_string().into_owned()),
        }
    }

    /// 同步执行元方法：UserFunc 用临时子协程(快照 open upvalue)执行，
    /// NativeFunc 直接调用。返回单个结果值。
    fn call_sync<const N: usize>(
        &mut self,
        heap: &mut duka_gc::Heap,
        callee: RuntimeValue,
        params: [RuntimeValue; N],
    ) -> Result<RuntimeValue, DukaRuntimeError> {
        match &callee {
            RuntimeValue::UserFunc(closure) => {
                sync_meta_call(&mut self.inner, heap, *closure, &params)
            }
            _ => {
                let pos = self.call_one_ret(heap, callee, params)?;
                Ok(self.inner.get_stack(pos)?.clone())
            }
        }
    }

    /// 二元运算元方法：任一操作数为 Table 且持有对应元方法时同步调用，
    /// 始终以 (left, right) 原始顺序传参。无元方法返回 Ok(None)。
    fn try_binary_meta_method(
        &mut self,
        heap: &mut duka_gc::Heap,
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
        self.call_sync(heap, method, [left.clone(), right.clone()])
            .map(Some)
    }

    /// 算术指令：先跑原生算术，非数值操作数且存在元方法时同步调用，
    /// 否则报类型错误。DividedByZero 等原生错误直接传播。
    #[inline]
    fn arith_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
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
                if let Some(v) = self.try_binary_meta_method(heap, method, left, right)? {
                    Ok(v)
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::NUM))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// 位运算指令：先跑原生位运算，非整数操作数且存在元方法时同步调用，否则报类型错误
    #[inline]
    fn bit_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
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
                if let Some(v) = self.try_binary_meta_method(heap, method, left, right)? {
                    Ok(v)
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::INT))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// 比较指令：先跑原生比较，不可比较且存在元方法时同步调用，否则报错。
    #[inline]
    fn compare_meta<F>(
        &mut self,
        heap: &mut duka_gc::Heap,
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
                if let Some(v) = self.try_binary_meta_method(heap, method, left, right)? {
                    Ok(v.eval_to_bool())
                } else {
                    Err(DukaRuntimeError::InvalidValueType(ctype::NUM))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// 读表字段，键缺失时沿 __index 链逐级回退(函数同步调用或表索引)。
    #[inline]
    fn get_table_field(
        &mut self,
        heap: &mut duka_gc::Heap,
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
                    return self.call_sync(heap, m, [RuntimeValue::Table(cur), key.clone()]);
                }
                _ => return Ok(RuntimeValue::Nil),
            }
        }
    }

    /// 写表字段，键缺失时回退 __newindex(仅查 metatable,不查表自身字段)。
    ///
    /// 性能:无 metatable 的表快路径为一次 insert;有 metatable 时也只在键原先
    /// 不存在时才探测。__newindex 只从 metatable 取,避免"自身字段写入即触发
    /// 自身"的递归以及每次新键写入的多余 hashmap 探测。
    #[inline]
    fn set_table_field(
        &mut self,
        heap: &mut duka_gc::Heap,
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
                    self.call_sync(heap, m, [RuntimeValue::Table(tab), key, val])?;
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
        callee: RuntimeValue,
        params: [RuntimeValue; N],
    ) -> Result<usize, DukaRuntimeError> {
        let func_pos = self.inner.stack.len() - self.inner.get_base();

        self.inner.append_stack(callee)?;
        for param in params {
            self.inner.append_stack(param)?;
        }

        self.call(
            heap,
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
        func: usize,
        narg: ValueCount,
        nwanted: ValueCount,
        tailcall: bool,
    ) -> Result<(), DukaRuntimeError> {
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        let mut narg = narg.clone();
        let callee = self.inner.get_stack(func)?.clone();
        let callee = match callee {
            RuntimeValue::Table(t) => {
                // __call 元方法:把 table 作为首个实参插到参数之前,再调用方法。
                let Some(method) = t.borrow().get_meta_method(heap, &MetaMethod::Call) else {
                    return Err(InvalidValueType(ctype::FUN));
                };
                let base = self.inner.get_base();
                let n = match narg {
                    ValueCount::Exact(a) => a,
                    ValueCount::VarArg => self.inner.stack.len().saturating_sub(func + base + 1),
                };
                let abs = func + base + 1;
                if self.inner.stack.len() >= abs {
                    self.inner.stack.insert(abs, RuntimeValue::Table(t));
                } else {
                    self.inner.stack.resize(abs, RuntimeValue::default());
                    self.inner.stack.push(RuntimeValue::Table(t));
                }
                self.inner.stack[func + base] = method;
                narg = ValueCount::Exact(n + 1);
                self.inner.stack[func + base].clone()
            }
            callee => callee.clone(),
        };
        (!callee.is_function()).then_error(|| InvalidValueType(ctype::FUN))?;
        let base = self.inner.get_base();

        // 调用前把栈裁剪到实参末尾
        if let ValueCount::Exact(a) = narg {
            self.inner.adjust_stack(func + base + 1 + a);
        }

        match callee {
            NativeFunc(closure) => {
                let mut ptr = closure.borrow_mut();

                // Native functions read args from `base+1..` and write results
                // at `R0..`, so the frame base is the callee slot itself.
                self.inner.set_base(func + base);

                let nreturn = match (ptr.func)(&mut self.inner, heap)? {
                    ValueCount::VarArg => self.inner.stack.len() - (func + base),
                    ValueCount::Exact(n) => n,
                };

                let raw_wanted = match nwanted {
                    ValueCount::VarArg => nreturn,
                    ValueCount::Exact(n) => n,
                };
                let keep = raw_wanted.max(nreturn);
                if self.inner.stack.len() < func + base + keep {
                    self.inner
                        .stack
                        .resize_with(func + base + keep, RuntimeValue::default);
                }
                if nreturn < raw_wanted {
                    let from = func + base + nreturn;
                    for i in 0..raw_wanted - nreturn {
                        self.inner.stack[from + i] = RuntimeValue::default();
                    }
                }

                // Results are at `func..func+keep`; truncate above them,
                // keeping the live registers below `func`.
                self.inner.adjust_stack(func + base + keep);
                self.inner.set_base(base);
            }
            UserFunc(closure) => {
                let fixed_count = closure.func.param_count;
                let has_var_arg = closure.func.has_var_arg;

                if tailcall {
                    self.close_up_values()?;
                    self.inner
                        .cut_stack(base - 1, ValueCount::Exact(base + func));
                    let wanted = match nwanted {
                        ValueCount::Exact(n) => n,
                        ValueCount::VarArg => usize::MAX,
                    };
                    if let ValueCount::Exact(a) = narg {
                        if a < fixed_count {
                            let count = fixed_count - a;
                            for i in 0..count {
                                self.inner.set_stack(a + i, Nil)?;
                            }
                        } else if a > fixed_count && !has_var_arg {
                            self.inner.adjust_stack(base + a);
                        }
                    }
                    let frame = CallFrame::call(base, base - 1, wanted);
                    *self.inner.current_mut() = frame;
                    return Ok(());
                }

                if let ValueCount::Exact(a) = narg {
                    if a < fixed_count {
                        // Pad missing params with nil; params live at
                        // `func+1..` (the callee's frame base).
                        let from = func + a + 1;
                        let count = fixed_count - a;
                        for i in 0..count {
                            self.inner.set_stack(i + from, Nil)?;
                        }
                    } else if a > fixed_count && !has_var_arg {
                        let len = func + a + 1 + base;
                        self.inner.adjust_stack(len);
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
                if self.inner.stack.len() < needed {
                    self.inner.stack.resize_with(needed, RuntimeValue::default);
                }
                self.push_frame(frame);
            }
            _ => unreachable!(),
        };
        Ok(())
    }
}
