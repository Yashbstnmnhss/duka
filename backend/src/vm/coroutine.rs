use std::cmp::Ordering;

use crate::{
    SysCallId,
    errors::DukaRuntimeError,
    instructions::{Address, DecodeInstruction, Instruction},
    value::{DukaClosure, DukaProto, RuntimeDukaTable, RuntimeValue, UpValue},
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
    value::{ConstValue, DukaFloat, DukaInt},
};
const INIT_CAPACITY: usize = 16;

/// 协程运行状态
#[derive(Debug, Default)]
pub struct CoState {
    /// 协程的值栈
    pub stack: Stack,
    /// 调用帧
    pub frames: Vec<CallFrame>,
}
impl CoState {
    #[inline]
    pub(crate) fn new_unsafe(reg_count: Option<usize>) -> Self {
        Self {
            stack: Vec::with_capacity(reg_count.unwrap_or(INIT_CAPACITY)),
            frames: vec![],
        }
    }
    #[inline(always)]
    pub fn with_closure(closure: Gc<DukaClosure>) -> Self {
        Self {
            stack: Vec::with_capacity(closure.func.used_reg_count),
            frames: vec![CallFrame::main(closure)],
        }
    }
    #[inline(always)]
    pub fn with_proto(proto: Gc<DukaProto>, heap: &mut duka_gc::Heap) -> Self {
        Self::with_closure(heap.alloc(DukaClosure::from_proto(proto)))
    }

    fn get_closure(&self) -> Result<&Gc<DukaClosure>, DukaRuntimeError> {
        match &self.current().proto {
            CallProto::Main { proto, .. } => Ok(proto),
            CallProto::Call { proto, .. } => self.get_stack(*proto).and_then(|v| match v {
                RuntimeValue::UserFunc(p) => Ok(p),
                _ => Err(DukaRuntimeError::InvalidValueType(ctype::PRO)),
            }),
        }
    }
    fn fetch(&self) -> Result<&Instruction, DukaRuntimeError> {
        self.get_closure()
            .map(|p| &p.func.instructions[self.current().pc])
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

    /// 获取栈上的值 **含base偏移**
    pub fn get_stack(&self, ad: usize) -> Result<&RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.get_base();
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => Ok(&self.stack[dst]),
            _ => Err(DukaRuntimeError::OutOfRange(cvm::STACK)),
        }
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
            _ => return Err(DukaRuntimeError::OutOfRange(cvm::STACK)),
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
            UpValue::Open(i) => self.inner.get_stack(*i)?,
        })
    }
    fn with_up_val<F, R>(
        &mut self,
        up_val_idx: usize,
        f: F,
    ) -> Result<R, DukaRuntimeError>
    where
        F: FnOnce(&mut RuntimeValue) -> R,
    {
        let mut borrow = self.inner.get_up_value(up_val_idx)?.borrow_mut();
        Ok(match *borrow {
            UpValue::Closed(ref mut v) => f(v),
            UpValue::Open(i) => {
                let val = self.inner.get_stack_mut(i)?;
                f(val)
            }
        })
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
        let mut var_args: Vec<RuntimeValue> = vec![];

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
                $e as usize + vm!(@base)
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
                let cv = self
                    .inner
                    .get_closure()?
                    .func
                    .constants
                    .get($i as usize)
                    .ok_or(OutOfRange(cvm::CONST))?;
                RuntimeValue::const2runtime(heap, cv)
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

        loop {
            let inst = self.inner.fetch()?;

            (inst.check_extra().map_err(InvalidInstruction)? && extra_arg.is_none())
                .then_error(|| ExtraArgNotFound)?;

            let decoded = inst.decode().map_err(InvalidInstruction)?;
            match decoded {
                Move(a, b) => {
                    println!("{a} <- {b}");
                    dbg!(&self.inner.stack);
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
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari(left, right, std::ops::Add::add, std::ops::Add::add) {
                        vm!(R(a) := result);
                    }
                }
                Sub(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari(left, right, std::ops::Sub::sub, std::ops::Sub::sub) {
                        vm!(R(a) := result);
                    }
                }
                Mul(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari(left, right, std::ops::Mul::mul, std::ops::Mul::mul) {
                        vm!(R(a) := result);
                    }
                }
                Div(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    check_zero(right)?;
                    if let Some(result) = ari(left, right, std::ops::Div::div, std::ops::Div::div) {
                        vm!(R(a) := result);
                    }
                }
                IDiv(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    check_zero(right)?;
                    let result = unify_float(left, right)
                        .ok_or(InvalidValueType(ctype::NUM))
                        .map(|c| match c {
                            UnifiedNumber::Floats(a, b) => Int((a / b) as DukaInt),
                            UnifiedNumber::Ints(a, b) => Int(a / b),
                        })?;
                    vm!(R(a) := result);
                }
                Mod(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari(left, right, std::ops::Rem::rem, std::ops::Rem::rem) {
                        vm!(R(a) := result);
                    }
                }
                Pow(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    let result = Float(
                        unify_float(left, right)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Floats(a, b) => a.powf(b),
                                UnifiedNumber::Ints(a, b) => (a as DukaFloat).powi(b as i32),
                            })?,
                    );
                    vm!(R(a) := result);
                }
                BitAnd(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari_bit(left, right, std::ops::BitAnd::bitand) {
                        vm!(R(a) := Int(result));
                    }
                }
                BitOr(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari_bit(left, right, std::ops::BitOr::bitor) {
                        vm!(R(a) := Int(result));
                    }
                }
                BitXor(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari_bit(left, right, std::ops::BitXor::bitxor) {
                        vm!(R(a) := Int(result));
                    }
                }
                ShiftL(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari_bit(left, right, std::ops::Shl::shl) {
                        vm!(R(a) := Int(result));
                    }
                }
                ShiftR(a, b, c) => {
                    let left = vm!(R(b));
                    let right = vm!(R(c));
                    if let Some(result) = ari_bit(left, right, std::ops::Shr::shr) {
                        vm!(R(a) := Int(result));
                    }
                }
                Equal(a, b, c, t) => {
                    let (b, c) = (vm!(R(b)), vm!(R(c)));
                    // check __eq metamethod first
                    if let Table(t) = b
                        && let Some(method) = t.borrow().get_meta_method(heap, &MetaMethod::Eq)
                        && method.is_function()
                    {
                    } else if let Table(t) = c
                        && let Some(method) = t.borrow().get_meta_method(heap, &MetaMethod::Eq)
                        && method.is_function()
                    {
                    }

                    let r = cmp_eq(b, c)? == t;
                    vm!(R(a) := Bool(r));
                }
                Less(a, b, c) => {
                    let (b, c) = (vm!(R(b)), vm!(R(c)));
                    let r = cmp_le(b, c).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                LessEqual(a, b, c) => {
                    let (b, c) = (vm!(R(b)), vm!(R(c)));
                    let r = cmp_lt(b, c).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                Concat(a, count) => {
                    let operands = vm!(R(a; count));
                    let buf = operands.into_iter().fold(vec![], |mut a, i| {
                        a.extend(i.eval_to_string().as_bytes());
                        a
                    });
                    let r =
                        RuntimeValue::from_const(heap, ConstValue::String(buf.as_slice().into()));
                    vm!(R(a) := r);
                }
                Minus(a, b) => {
                    let r = vm!(R(b));
                    let v = match r {
                        Int(i) => Int(-i),
                        Float(f) => Float(-f),
                        _ => return Err(UnsupportedOperation("minus", r.type_of())),
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
                        && let Some(pos) =
                            self.call_unary_meta_method(heap, &MetaMethod::BNot, *t)?
                    {
                        vm!(R(a) := R(pos));
                    } else {
                        let val = vm!(R(b));
                        let num = val
                            .eval_to_int()
                            .ok_or_else(|| UnsupportedOperation("bit not", val.type_of()))?;
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
                            if let Some(pos) =
                                self.call_unary_meta_method(heap, &MetaMethod::Len, t)?
                            {
                                vm!(R(a) := R(pos));
                            } else {
                                let b = t.borrow();
                                vm!(R(a) := Int(b.len() as DukaInt));
                            }
                        }
                        _ => return Err(UnsupportedOperation("len", val.type_of())),
                    }
                }
                Jump(offset) => {
                    vm!(move offset);
                    continue; // already moved, dont vm!(continue)
                }
                Test(from, target) => {
                    // skip next if R(a) != b
                    let val = vm!(R(from));
                    if val.eval_to_bool() != target {
                        vm!(skip);
                    }
                }

                MarkToBeClosed(target) => {
                    let _up_val = vm!(UpVal(target));
                }
                Close(_) => {
                    self.close_up_values()?;
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
                            } else {
                                vm!(R(a + 3) := Int(init));
                                vm!(R(a + 1) := Int(limit));
                                // then loop
                            }
                        } else {
                            vm!(move end_offset); // this will move to the last code of inner block
                            // instead of the correct position, cause we have vm!(continue) at bottom
                        }
                    } else {
                        let init = cast!(Number use eval_to_float for vm!(R(a)))?;
                        let limit = cast!(Number use eval_to_float for vm!(R(a + 1)))?;
                        let step = cast!(Number use eval_to_float for vm!(R(a + 2)))?;

                        (step == 0.0).then_error(|| ZeroStepInForLoop)?;

                        if !for_number_check(init, limit, step.is_sign_negative()) {
                            vm!(move end_offset);
                            // this will move to the last code of inner block
                            // instead of the correct position, cause we have vm!(continue) at bottom
                        } else {
                            vm!(R(a + 3) := Float(init));
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
                        vm!(R(a + 3) := Int(new));

                        if for_number_check(new, limit, neg_step) {
                            vm!(move - (start_offset as isize)); // this will move to the For prepare
                            // instead of the first code of inner block, cause we have vm!(continue) at bottom
                        }
                    } else {
                        cast!(Float(deref init) = init);
                        cast!(Float(deref limit) = limit);
                        cast!(Float(deref step) = step);

                        let new = init + step;
                        // step != 0, already checked in ForPrepare
                        let neg_step = step.is_sign_negative();

                        vm!(R(a) := Float(init));
                        vm!(R(a + 3) := Float(init));

                        if for_number_check(new, limit, neg_step) {
                            vm!(move - (start_offset as isize)); // this will move to the For-prepare
                            // instead of the first code of inner block, cause we have vm!(continue) at bottom
                        }
                    }
                }

                TForPrepare(a, offset) => {
                    vm!(R(a + 3) := R(a + 2));
                    vm!(move offset);
                }
                TForCall(a, nres) => {
                    cast!(as nres: usize, a: usize);
                    self.call(heap, a, 2u8.into(), nres.into(), false)?;
                }
                TForLoop(a, offset) => {
                    cast!(as offset: isize);

                    let res = vm!(R(a + 3));
                    if !matches!(res, Nil) {
                        vm!(move -offset);
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
                        .expect("NO PROTO FOUND?!");

                    let mut up_values = vec![];

                    for desc in &proto.up_indexes {
                        let up_val = if desc.local {
                            heap.alloc(GcCell::new(UpValue::Open(vm!(@base) + desc.index)))
                        } else {
                            *vm!(UpVal(desc.index))
                        };
                        up_values.push(up_val)
                    }

                    // allocate proto and closure on VM heap
                    let proto_gc = heap.alloc(proto.clone());
                    let closure = heap.alloc(DukaClosure {
                        func: proto_gc,
                        up_values,
                    });
                    vm!(R(ad) := UserFunc(closure));
                }

                Call(func, narg, nwanted) => {
                    cast!(as func: usize);
                    self.call(heap, func, narg.into(), nwanted.into(), false)?;
                }
                TailCall(func, narg, nwanted) => {
                    cast!(as func: usize);
                    self.call(heap, func, narg.into(), nwanted.into(), true)?;
                }

                SysCall(syscall, narg, _nwanted) => {
                    let _id = SysCallId::from_disc(syscall)
                        .map_err(|_| NoSuchKey(syscall.to_string(), "syscall"))?;
                    let closure = self.inner.get_closure()?;
                    if let Some(ref logic_proto) = closure.func.logic {
                        let query_idx = narg as usize;
                        let solutions = crate::vm::logic::execute_query(logic_proto, query_idx)
                            .map_err(|e| Custom(e))?;
                        let table = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
                        for (i, sol) in solutions.iter().enumerate() {
                            let entry = heap.alloc(GcCell::new(RuntimeDukaTable::new(0)));
                            let mut keys: Vec<&usize> = sol.keys().collect();
                            keys.sort();
                            for (j, k) in keys.iter().enumerate() {
                                let val = RuntimeValue::from_string(heap, sol[k].clone());
                                entry.borrow_mut().set(RuntimeValue::Int((j + 1) as i64), val);
                            }
                            table.borrow_mut().set(RuntimeValue::Int((i + 1) as i64), RuntimeValue::Table(entry));
                        }
                        let result = RuntimeValue::Table(table);
                        vm!(R(syscall as Address) := result);
                    }
                }

                Return(from, count_) => {
                    cast!(as from: usize);

                    self.close_up_values()?;

                    let actual_count = cast!(
                        for count_
                        all(from vm!([from] for R))
                        as usize
                    );

                    let CallProto::Call { wanted, .. } = self.inner.current().proto else {
                        self.status = Dead;
                        return Ok(CoAction::Return(
                            from as Address,
                            ValueCount::Exact(actual_count),
                        ));
                    };
                    self.inner.frames.pop().ok_or(NoCallFrame)?;

                    if actual_count < wanted {
                        // want more, fill nil
                        vm!(R(from + actual_count; wanted - actual_count) := fill Nil);
                    } else {
                        // want less, cut off
                        vm!(@stack:remove [vm!([from + wanted] for R)]..);
                    }
                    vm!(@stack:remove [vm!(@base)]..[from]); // remove before
                }
                Return0() => {
                    dbg!("return");
                    self.close_up_values()?;

                    let frame = self.inner.current();
                    let wanted = match frame.proto {
                        CallProto::Call {wanted, ..} => wanted,
                        _ => {
                            self.status = Dead;
                            return Ok(CoAction::Return(
                                frame.get_base() as Address,
                                ValueCount::Exact(0),
                            ));
                        }
                    };
                        self.inner.frames.pop().ok_or(NoCallFrame)?;

                    vm!(R(0; wanted) := fill Nil); // fill with nil
                    vm!(@stack:remove [vm!([wanted] for R)]..); // remove tail
                }
                // Extra argument is before the target instruction
                ExtraArg(arg) => extra_arg = Some(arg),

                GetUpVal(a, i) => {
                    let val = match *vm!(UpVal(i)).borrow() {
                        UpValue::Closed(ref v) => v,
                        UpValue::Open(i) => vm!(R(i)),
                    }
                    .clone();

                    vm!(R(a) := val);
                }
                SetUpVal(a, i) => {
                    let val = vm!(R(a)).clone();
                    let mut up_val = vm!(UpVal(i)).borrow_mut();
                    match *up_val {
                        UpValue::Open(idx) => {
                            vm!(R(idx) := val);
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
                    let key = vm!(R(c));
                    let res = t.borrow().inner.get(key).cloned().unwrap_or_default();
                    vm!(R(a) := res);
                }
                GetI(a, b, i) => {
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let res = t
                        .borrow()
                        .array_get(i as usize)
                        .cloned()
                        .unwrap_or_default();
                    vm!(R(a) := res);
                }
                GetField(a, b, k) => {
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let key = vm!(K(k));
                    let res = t.borrow().inner.get(&key).cloned().unwrap_or_default();
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
                        t.borrow_mut().array_set(i as usize, val);
                    }
                }
                // SetTable: 索引为R
                // SetField: 索引为K
                SetTable(a, b, c, k) => {
                    let val = vm!(RK(c, k));
                    let key = vm!(R(b));
                    let table = vm!(R(a));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    t.borrow_mut().inner.insert(key.clone(), val);
                }
                SetField(a, b, c, k) => {
                    let val = vm!(RK(c, k));
                    let key = vm!(K(b));
                    let table = vm!(R(a));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    t.borrow_mut().inner.insert(key, val);
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
                    let val = vm!(R(b));
                    (!val.is_number()).then_error(|| InvalidValueType(ctype::NUM))?;
                    let res = match val {
                        Int(int) => Int(*int + (n as DukaInt)),
                        Float(flt) => Float(*flt + (n as DukaFloat)),
                        _ => unreachable!(),
                    };
                    vm!(R(a) := res);
                }
                AddK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(r) = ari(b, &k, std::ops::Add::add, std::ops::Add::add) {
                        vm!(R(a) := r);
                    }
                }
                SubK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(r) = ari(b, &k, std::ops::Sub::sub, std::ops::Sub::sub) {
                        vm!(R(a) := r);
                    }
                }
                MulK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(r) = ari(b, &k, std::ops::Mul::mul, std::ops::Mul::mul) {
                        vm!(R(a) := r);
                    }
                }
                ModK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(r) = ari(b, &k, std::ops::Rem::rem, std::ops::Rem::rem) {
                        vm!(R(a) := r);
                    }
                }
                PowK(a, b, k) => {
                    let left = vm!(R(b));
                    let right = vm!(K(k));
                    let result = Float(
                        unify_float(left, &right)
                            .ok_or(InvalidValueType(ctype::NUM))
                            .map(|c| match c {
                                UnifiedNumber::Floats(a, b) => a.powf(b),
                                UnifiedNumber::Ints(a, b) => (a as DukaFloat).powi(b as i32),
                            })?,
                    );
                    vm!(R(a) := result);
                }
                DivK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(result) = ari(b, &k, std::ops::Div::div, std::ops::Div::div) {
                        vm!(R(a) := result);
                    }
                }
                IDivK(a, b, k) => {
                    let left = vm!(R(b));
                    let right = vm!(K(k));
                    //check_zero(right)?;
                    let result = unify_float(left, &right)
                        .ok_or(InvalidValueType(ctype::NUM))
                        .map(|c| match c {
                            UnifiedNumber::Floats(a, b) => Int((a / b) as DukaInt),
                            UnifiedNumber::Ints(a, b) => Int(a / b),
                        })?;
                    vm!(R(a) := result);
                }
                BitAndK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(result) = ari_bit(b, &k, std::ops::BitAnd::bitand) {
                        vm!(R(a) := Int(result));
                    }
                }
                BitOrK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(result) = ari_bit(b, &k, std::ops::BitOr::bitor) {
                        vm!(R(a) := Int(result));
                    }
                }
                BitXorK(a, b, k) => {
                    let b = vm!(R(b));
                    let k = vm!(K(k));
                    if let Some(result) = ari_bit(b, &k, std::ops::BitXor::bitxor) {
                        vm!(R(a) := Int(result));
                    }
                }
                ShiftRI(a, b, i) => {
                    let b = vm!(R(b));
                    let Int(b) = b else {
                        return Err(InvalidValueType(ctype::INT));
                    };
                    let r = Int(*b >> i);
                    vm!(R(a) := r);
                }
                MMBinary(a, b, meta) => {
                    let left = vm!(R(a));
                    let right = vm!(R(b));
                    if let Some((func, tab, oth)) =
                        self.get_binary_meta_method(heap, left, right, &meta)
                    {
                        self.call_one_ret(heap, func, [Table(tab), oth])?;
                    }
                }
                MMBinaryI(a, i, meta, flip) => {
                    let (left, right) = if flip {
                        (&Int(i as DukaInt), vm!(R(a)))
                    } else {
                        (vm!(R(a)), &Int(i as DukaInt))
                    };
                    if let Some((func, tab, oth)) =
                        self.get_binary_meta_method(heap, left, right, &meta)
                    {
                        self.call_one_ret(heap, func, [Table(tab), oth])?;
                    }
                }
                MMBinaryK(a, k, meta, flip) => {
                    let (left, right) = if flip {
                        (&vm!(K(k)), vm!(R(a)))
                    } else {
                        (vm!(R(a)), &vm!(K(k)))
                    };
                    if let Some((func, tab, oth)) =
                        self.get_binary_meta_method(heap, left, right, &meta)
                    {
                        self.call_one_ret(heap, func, [Table(tab), oth])?;
                    }
                }

                EqualK(a, b, k, t) => {
                    let (b, k) = (vm!(R(b)), vm!(K(k)));
                    let r = cmp_eq(b, &k)? == t;
                    vm!(R(a) := Bool(r));
                }
                EqualI(a, b, i, t) => {
                    let n = vm!(R(b));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    let r = cmp_im(|x, y| x == y, i as DukaInt)(n).is_some_and(|v| v) == t;
                    vm!(R(a) := Bool(r));
                }
                LessI(a, b, i) => {
                    let n = vm!(R(b));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    let r = cmp_im(|x, y| x < y, i as DukaInt)(n).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                LessEqualI(a, b, i) => {
                    let n = vm!(R(b));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    let r = cmp_im(|x, y| x <= y, i as DukaInt)(n).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                GreaterI(a, b, i) => {
                    let n = vm!(R(b));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    let r = cmp_im(|x, y| x > y, i as DukaInt)(n).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                GreaterEqualI(a, b, i) => {
                    let n = vm!(R(b));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    let r = cmp_im(|x, y| x >= y, i as DukaInt)(n).is_some_and(|v| v);
                    vm!(R(a) := Bool(r));
                }
                SetList(list, start_index, count) => {
                    cast!(as list: usize, start_index: usize, count: usize);

                    let mut table = match vm!(R(list)) {
                        Table(t) => t.borrow_mut(),
                        _ => return Err(InvalidValueType(ctype::TAB)),
                    };
                    for i in 0..count {
                        let val = vm!(R(list + i)).clone();
                        table.array_set(i + start_index, val);
                    }
                }

                // When a duka function needs var_arg, this will appear at the start of function
                VarArgPrepare(fixed_param_count) => {
                    let end_of_params = vm!([fixed_param_count] for R);
                    var_args = if end_of_params < vm!(@top) {
                        vm!(@stack:remove [end_of_params]..).collect()
                    } else {
                        Default::default()
                    };
                }
                VarArg(ad, count_) => {
                    let count = cast!(
                        for count_
                        all(from vm!([ad] for R))
                        as usize
                    );
                    for o in 0..count {
                        let val = var_args.get(o).cloned().unwrap_or(Nil);

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

        #[inline(always)]
        fn for_number_check<T: PartialOrd>(init: T, limit: T, neg_step: bool) -> bool {
            !neg_step && init < limit || neg_step && init > limit
        }

        fn check_zero(right: &RuntimeValue) -> Result<(), DukaRuntimeError> {
            (right.is_number()
                && (match right {
                    Int(v) => *v == 0,
                    Float(v) => *v == 0.0,
                    _ => unreachable!(),
                }))
            .then_error(|| DividedByZero)
        }

        #[inline(always)]
        fn ari_bit(
            a: &RuntimeValue,
            b: &RuntimeValue,
            f: fn(DukaInt, DukaInt) -> DukaInt,
        ) -> Option<DukaInt> {
            let (Int(a), Int(b)) = (a, b) else {
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
                UnifiedNumber::Floats(a, b) => Float(ff(a, b)),
                UnifiedNumber::Ints(a, b) => Int(fi(a, b)),
            })
        }

        #[inline(always)]
        fn cmp_im(
            fu: fn(DukaInt, DukaInt) -> bool,
            im: DukaInt,
        ) -> impl Fn(&RuntimeValue) -> Option<bool> {
            move |v| -> Option<bool> {
                Some(match v {
                    Int(i) => fu(*i, im),
                    Float(f) => fu(*f as DukaInt, im),
                    _ => return None,
                })
            }
        }

        #[inline(always)]
        fn cmp_lt(a: &RuntimeValue, b: &RuntimeValue) -> Option<bool> {
            unify_float(a, b).map(|c| match c {
                UnifiedNumber::Floats(a, b) => a < b,
                UnifiedNumber::Ints(a, b) => a < b,
            })
        }
        #[inline(always)]
        fn cmp_le(a: &RuntimeValue, b: &RuntimeValue) -> Option<bool> {
            unify_float(a, b).map(|c| match c {
                UnifiedNumber::Floats(a, b) => a <= b,
                UnifiedNumber::Ints(a, b) => a <= b,
            })
        }
        #[inline(always)]
        fn cmp_eq(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
            Ok(a.eq(b))
        }

        enum UnifiedNumber {
            Ints(DukaInt, DukaInt),
            Floats(DukaFloat, DukaFloat),
        }
        fn unify_float(a: &RuntimeValue, b: &RuntimeValue) -> Option<UnifiedNumber> {
            use UnifiedNumber::*;
            Some(match (a, b) {
                (Int(a), Int(b)) => Ints(*a, *b),
                (Int(a), Float(b)) => Floats(*a as DukaFloat, *b),
                (Float(a), Int(b)) => Floats(*a, *b as DukaFloat),
                (Float(a), Float(b)) => Floats(*a, *b),
                _ => return None,
            })
        }
    }

    fn close_up_values(&self) -> Result<(), DukaRuntimeError> {
        let closure = self.inner.get_closure()?;
        for up_val in &closure.up_values {
            let mut up_val = up_val.borrow_mut();
            if let UpValue::Open(idx) = *up_val {
                let val = self.inner.get_stack(idx - self.inner.get_base())?.clone();
                *up_val = UpValue::Closed(val);
            }
        }
        Ok(())
    }

    fn call_unary_meta_method(
        &mut self,
        heap: &mut duka_gc::Heap,
        method: &MetaMethod,
        who: Gc<GcCell<RuntimeDukaTable>>,
    ) -> Result<Option<usize>, DukaRuntimeError> {
        if let Some(method) = who.borrow().get_meta_method(heap, method) {
            let pos = self.call_one_ret(heap, method, [RuntimeValue::Table(who)])?;
            Ok(Some(pos))
        } else {
            Ok(None)
        }
    }

    fn get_binary_meta_method(
        &self,
        heap: &mut duka_gc::Heap,
        left: &RuntimeValue,
        right: &RuntimeValue,
        method: &MetaMethod,
    ) -> Option<(RuntimeValue, Gc<GcCell<RuntimeDukaTable>>, RuntimeValue)> {
        let (tab, oth) = match (left, right) {
            (RuntimeValue::Table(tab), oth) => (*tab, oth),
            (oth, RuntimeValue::Table(tab)) => (*tab, oth),
            _ => return None,
        };
        tab.borrow()
            .get_meta_method(heap, method)
            .filter(|v| v.is_function())
            .map(|f| (f, tab, oth.clone()))
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

        let callee = self.inner.get_stack(func)?;
        (!callee.is_function()).then_error(|| InvalidValueType(ctype::FUN))?;
        let base = self.inner.get_base();

        let (narg, nwanted): (usize, usize) = (narg.into(), nwanted.into());

        match callee {
            NativeFunc(closure) => {
                let f = *closure;
                let mut ptr = f.borrow_mut();

                self.inner.set_base(func);

                let nreturn = match (ptr.func)(&mut self.inner, heap)? {
                    ValueCount::VarArg => self.inner.stack.len() - base,
                    ValueCount::Exact(n) => n,
                };

                if nreturn < nwanted {
                    let from = nreturn;
                    let count = nwanted - nreturn;
                    for i in 0..count {
                        self.inner.set_stack(i + from, Nil)?;
                    }
                } else {
                    let len = base + nwanted;
                    self.inner.adjust_stack(len);
                }

                self.inner.set_base(base);
            }
            UserFunc(closure) => {
                let fixed_count = closure.func.param_count;
                let has_var_arg = closure.func.has_var_arg;

                if tailcall {
                    self.close_up_values()?;
                    self.inner
                        .cut_stack(base - 1, ValueCount::Exact(base + func));
                }

                if narg < fixed_count {
                    let from = func + narg + 1;
                    let count = fixed_count - narg;
                    for i in 0..count {
                        self.inner.set_stack(i + from, Nil)?;
                    }
                } else if narg > fixed_count && !has_var_arg {
                    let len = func + narg + 1 + base;
                    self.inner.adjust_stack(len);
                }

                if !tailcall {
                    let frame =
                        CallFrame::call(self.inner.stack.len() - narg, func + base, nwanted);
                    self.push_frame(frame);
                }
            }
            _ => unreachable!(),
        };
        Ok(())
    }
}
