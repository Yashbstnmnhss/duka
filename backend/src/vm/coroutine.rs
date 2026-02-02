use std::cmp::Ordering;

use crate::{
    SysCallId,
    error::DukaRuntimeError,
    instructions::{Address, DecodeInstruction, Instruction},
    value::{DukaClosure, DukaProto, RuntimeDukaTable, RuntimeValue, UpValue, ValueCount},
    vm::{
        Bits25, CoAction, VMContext,
        frame::{CallFrame, CallProto, Stack},
    },
};
use duka_macros::Info;
use duka_shared::{
    constants::{MetaMethod, ctype, cvm},
    utils::OrError,
    value::{ConstValue, DukaFloat, DukaInt},
};
use gc::{Finalize, Trace, Tracer};
use gc::{Gc, GcCell};
const INIT_CAPACITY: usize = 16;

/// 协程运行状态
#[derive(Debug)]
pub struct CoState {
    /// 协程的值栈
    pub stack: Stack,
    /// 调用帧
    pub frames: Vec<CallFrame>,
}
impl CoState {
    #[inline(always)]
    pub fn new(reg_count: Option<usize>) -> Self {
        Self {
            stack: Vec::with_capacity(reg_count.unwrap_or(INIT_CAPACITY)),
            frames: vec![],
        }
    }
    #[inline(always)]
    pub fn closure_to_main(closure: Gc<DukaClosure>) -> Self {
        Self {
            stack: Vec::with_capacity(closure.func.reg_count),
            frames: vec![CallFrame::main(closure)],
        }
    }
    #[inline(always)]
    pub fn proto_to_main(proto: Gc<DukaProto>, heap: &mut gc::Heap) -> Self {
        Self::closure_to_main(heap.alloc(DukaClosure::from_proto(proto)))
    }

    fn get_closure(&self) -> Result<&Gc<DukaClosure>, DukaRuntimeError> {
        match &self.current().proto {
            CallProto::Main(p) => Ok(p),
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

    #[inline(always)]
    pub fn push_frame(&mut self, frame: CallFrame) {
        if let CallProto::Main(cls) = frame.proto {
            self.stack.reserve(cls.func.reg_count);
        }
        self.frames.push(frame);
    }

    fn get_upvalue(&self, index: usize) -> Result<&Gc<GcCell<UpValue>>, DukaRuntimeError> {
        self.get_closure()?
            .upvalues
            .get(index)
            .ok_or(DukaRuntimeError::OutOfRange(cvm::UPVAL))
    }

    #[inline(always)]
    pub fn current(&self) -> &CallFrame {
        self.frames.last().expect("WHERE IS YOUR MAIN FRAME?") //bro...
    }
    #[inline(always)]
    pub fn current_mut(&mut self) -> &mut CallFrame {
        self.frames.last_mut().expect("WHERE IS YOUR MAIN FRAME?")
    }

    #[inline(always)]
    fn get_base(&self) -> usize {
        self.current().base()
    }

    pub(crate) fn adjust_stack(&mut self, to_len: usize) {
        self.stack.truncate(to_len);
    }

    pub(crate) fn cut_stack(&mut self, from: usize, count: ValueCount) -> Vec<RuntimeValue> {
        self.stack
            .drain(from..count.to_index(self.stack.len()))
            .collect()
    }

    pub fn get_stack_mut(&mut self, ad: usize) -> Result<&mut RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.get_base();
        self.stack
            .get_mut(dst)
            .ok_or(DukaRuntimeError::OutOfRange(cvm::STACK))
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
        self.set_stack(0, val)
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

#[doc = "Helper for native rust function"]
impl CoState {}

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
    fn get_val_from_upval<'a>(
        &'a self,
        up_value: &'a UpValue,
    ) -> Result<&'a RuntimeValue, DukaRuntimeError> {
        Ok(match up_value {
            UpValue::Closed(c) => c,
            UpValue::Open(i) => self.inner.get_stack(*i)?,
        })
    }
    fn with_val_from_upval_idx<F, R>(
        &mut self,
        upval_idx: usize,
        f: F,
    ) -> Result<R, DukaRuntimeError>
    where
        F: FnOnce(&mut RuntimeValue) -> R,
    {
        let mut borrow = self.inner.get_upvalue(upval_idx)?.borrow_mut();
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
        heap: &mut gc::Heap,
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
                $target . $func ().map_err(|_| InvalidValueType(stringify!($ty)))
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
                vm!(@frame).base()
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
                self.inner.get_upvalue($i as usize)?
            };
            (UpVal($i: expr) := $v: expr) => {
                self.inner.get_closure()?.upvalues.set($i as usize).and_then(|u| u.get_value()).ok_or(OutOfUpvalue)?
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
                extra_arg.take().expect("?")
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
                LoadFalseSkip(a) => {
                    vm!(R(a) := Bool(false));
                    vm!(skip);
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
                Equal(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_eq(a, b)? {
                        vm!(skip);
                    }
                }
                Less(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_le(a, b).is_some_and(|v| v) {
                        vm!(skip);
                    }
                }
                LessEqual(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_lt(a, b).is_some_and(|v| v) {
                        vm!(skip);
                    }
                }
                Concat(a, count) => {
                    let operands = vm!(R(a; count));
                    let buf = operands.into_iter().fold(vec![], |mut a, i| {
                        a.extend(i.eval_to_string().as_bytes());
                        a
                    });
                    let r = RuntimeValue::from_const(heap, ConstValue::String(buf));
                    vm!(R(a) := r);
                }
                Minus(a, b) => {
                    let r = vm!(R(b));
                    let v = match r {
                        Bool(..) | Nil => return Err(UnsupportedOperation("minus", r.type_of())),
                        rv if rv.is_string() => {
                            return Err(UnsupportedOperation("minus", r.type_of()));
                        }
                        Int(i) => Int(-i),
                        Float(f) => Float(-f),
                        _ => todo!(),
                    };
                    vm!(R(a) := v);
                }
                Not(a, b) => {
                    let val = vm!(R(b)).eval_to_bool();
                    vm!(R(a) := Bool(!val));
                }
                BitNot(a, b) => {
                    let val = vm!(R(b));
                    let num = val
                        .eval_to_int()
                        .map_err(|_| UnsupportedOperation("bit not", val.type_of()))?;
                    vm!(R(a) := Int(!num));
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
                            let _self = vm!(R(b)).clone();
                            if let Some(method) =
                                self.get_metamethod(heap, &_self, &MetaMethod::Len)
                            {
                                let pos = vm!(@top) - vm!(@base);
                                self.inner.append_stack(method)?;
                                self.inner.append_stack(_self)?;
                                self.call(heap, pos, 1, 1, false)?;

                                vm!(R(a) := vm!(R(pos)).clone());
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
                TestSet(from, set, target) => {
                    let val = vm!(R(from));
                    if val.eval_to_bool() != target {
                        vm!(skip);
                    } else {
                        vm!(R(from) := R(set));
                    }
                }

                MarkToBeClosed(target) => {
                    let _upval = vm!(UpVal(target));
                }
                Close(_) => {
                    self.close_upvalues()?;
                }

                ForPrepare(a, end_offset) => {
                    fn for_limit(
                        limit: DukaFloat,
                        step_positive: bool,
                    ) -> Result<DukaInt, DukaInt> {
                        step_positive
                            .then(|| {
                                (limit >= DukaInt::MIN as DukaFloat)
                                    .then_some(limit.floor() as DukaInt)
                                    .ok_or(-1)
                            })
                            .unwrap_or_else(|| {
                                (limit <= DukaInt::MAX as DukaFloat)
                                    .then_some(limit.ceil() as DukaInt)
                                    .ok_or(1)
                            })
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
                            vm!(move - (start_offset as isize)); // this will move to the Forprepare
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
                            vm!(move - (start_offset as isize)); // this will move to the Forprepare
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
                    self.call(heap, a, 2, nres, false)?;
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
                    // push closure to stack & initialize its upvalues

                    let proto = self
                        .inner
                        .get_closure()?
                        .func
                        .nested_protos
                        .get(index)
                        .expect("NO PROTO FOUND?!");

                    let mut upvalues = vec![];

                    for desc in &proto.up_indexes {
                        let upval = if desc.local {
                            heap.alloc(GcCell::new(UpValue::Open(vm!(@base) + desc.index)))
                        } else {
                            vm!(UpVal(desc.index)).clone()
                        };
                        upvalues.push(upval)
                    }

                    // allocate proto and closure on VM heap
                    let proto_gc = heap.alloc(proto.clone());
                    let closure = heap.alloc(DukaClosure {
                        func: proto_gc,
                        upvalues,
                    });
                    vm!(R(ad) := UserFunc(closure));
                }

                Call(func, narg, nwanted) => {
                    cast!(as func: usize, narg: usize, nwanted: usize);
                    self.call(heap, func, narg, nwanted, false)?;
                }
                CallSet(ad, func, narg) => {
                    cast!(as func: usize, narg: usize);
                    self.call(heap, func, narg, 1, false)?;
                    let ad = vm!([ad] for R);
                    self.inner.stack.swap(ad, func);
                }
                TailCall(func, narg) => {
                    cast!(as func: usize, narg: usize);
                    self.call(heap, func, narg, 0, true)?;
                }

                SysCall(syscall, narg, nwanted) => {
                    let id = SysCallId::from_disc(syscall)
                        .map_err(|_| NoSuchKey(syscall.to_string(), "syscall"))?;
                }

                Return(from, count_) => {
                    cast!(as from: usize);

                    self.close_upvalues()?;

                    let actual_count = cast!(
                        for count_
                        all(from vm!([from] for R))
                        as usize
                    );

                    let frame = self.inner.frames.pop().ok_or(NoCallFrame)?;
                    let CallProto::Call { wanted, .. } = frame.proto else {
                        self.status = Dead;
                        return Ok(CoAction::Return(
                            from as Address,
                            ValueCount::Exact(actual_count),
                        ));
                    };

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
                    self.close_upvalues()?;

                    let frame = self.inner.frames.pop().ok_or(NoCallFrame)?;
                    let CallProto::Call { wanted, .. } = frame.proto else {
                        self.status = Dead;
                        return Ok(CoAction::Return(
                            vm!(@base) as Address,
                            ValueCount::Exact(0),
                        ));
                    };

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
                    let mut upval = vm!(UpVal(i)).borrow_mut();
                    match *upval {
                        UpValue::Open(idx) => {
                            vm!(R(idx) := val);
                        }
                        UpValue::Closed(ref mut old_val) => *old_val = val,
                    }
                }

                GetTabUp(a, b, k) => {
                    let upval = vm!(UpVal(b)).borrow();
                    let table = self.get_val_from_upval(&upval)?;
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let key = vm!(K(k));
                    let res = t
                        .borrow()
                        .inner
                        .get(&key)
                        .map(|a| a.clone())
                        .unwrap_or_default();
                    vm!(R(a) := res);
                }
                GetTable(a, b, c) => {
                    let table = vm!(R(b));
                    let Table(t) = table else {
                        return Err(InvalidValueType(ctype::TAB));
                    };
                    let key = vm!(R(c));
                    let res = t
                        .borrow()
                        .inner
                        .get(&key)
                        .map(|a| a.clone())
                        .unwrap_or_default();
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
                    let res = t
                        .borrow()
                        .inner
                        .get(&key)
                        .map(|a| a.clone())
                        .unwrap_or_default();
                    vm!(R(a) := res);
                }
                SetTabUp(a, b, c, k) => {
                    let key = vm!(K(b));
                    let val = vm!(RK(c, k));

                    self.with_val_from_upval_idx(a as usize, |table| {
                        if let Table(t) = table {
                            t.borrow_mut().inner.insert(key, val);
                        }
                    })?;
                }
                SetI(a, i, b, k) => {
                    let table = vm!(R(a));
                    let val = vm!(RK(b, k));
                    if let Table(t) = table {
                        t.borrow_mut().array_push(i as usize, val);
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
                NewTable(a, n, new) => {
                    // NO NEED let n = vm!(E());
                    cast!(as n: usize);
                    let table = if new {
                        Table(heap.alloc(GcCell::new(RuntimeDukaTable::new(n))))
                    } else {
                        vm!(K(n))
                    };
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

                // NO NEED ShiftLI(_, _, _) => todo!(),
                MMBinary(a, meta, b) => {
                    let method = MetaMethod::from_disc(meta)
                        .map_err(|_| UnimplementedMetamethod(meta.to_string()))?;
                    let left = vm!(R(a)).clone();
                    let right = vm!(R(b)).clone();
                    self.call_binary_metamethod(heap, method, left, right)?;
                }
                MMBinaryI(a, meta, i) => {
                    let method = MetaMethod::from_disc(meta)
                        .map_err(|_| UnimplementedMetamethod(meta.to_string()))?;
                    let left = vm!(R(a)).clone();
                    let right = Int(i as DukaInt);
                    self.call_binary_metamethod(heap, method, left, right)?;
                }
                MMBinaryK(a, meta, k) => {
                    let method = MetaMethod::from_disc(meta)
                        .map_err(|_| UnimplementedMetamethod(meta.to_string()))?;
                    let left = vm!(R(a)).clone();
                    let right = vm!(K(k));
                    self.call_binary_metamethod(heap, method, left, right)?;
                }

                EqualK(ad, k, target) => {
                    let (a, k_val) = (vm!(R(ad)), vm!(K(k)));

                    if cmp_eq(a, &k_val)? && !target {
                        vm!(skip);
                    }
                }
                EqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x == y, im as DukaInt)(n).is_some_and(|v| v) && !target {
                        vm!(skip);
                    }
                }
                LessI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x < y, im as DukaInt)(n).is_some_and(|v| v) && !target {
                        vm!(skip);
                    }
                }
                LessEqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x <= y, im as DukaInt)(n).is_some_and(|v| v) && !target {
                        vm!(skip);
                    }
                }
                GreaterI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x > y, im as DukaInt)(n).is_some_and(|v| v) && !target {
                        vm!(skip);
                    }
                }
                GreaterEqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x >= y, im as DukaInt)(n).is_some_and(|v| v) && !target {
                        vm!(skip);
                    }
                }
                SetList(list, start_index, count) => {
                    cast!(as list: usize, start_index: usize, count: usize);

                    let mut table = match vm!(R(list)) {
                        Table(t) => t.borrow_mut(),
                        _ => return Err(InvalidValueType(ctype::TAB)),
                    };
                    for i in 0..count as usize {
                        let val = vm!(R(list + i)).clone();
                        table.array_push(i + start_index, val);
                    }
                }

                // When a duka function needs vararg, this will appear at the start of function
                VarArgPrepare(fixed_param_count) => {
                    let end_of_params = vm!([fixed_param_count] for R);
                    var_args = (end_of_params < vm!(@top))
                        .then(|| vm!(@stack:remove [end_of_params]..).collect())
                        .unwrap_or_default();
                }
                VarArg(ad, count_) => {
                    let count = cast!(
                        for count_
                        all(from vm!([ad] for R))
                        as usize
                    );
                    for o in 0..count {
                        let val = var_args
                            .get(o as usize)
                            .map(|v| v.clone())
                            .unwrap_or_else(|| Nil);

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

    fn close_upvalues(&self) -> Result<(), DukaRuntimeError> {
        let closure = self.inner.get_closure()?;
        for upval in &closure.upvalues {
            let mut upval = upval.borrow_mut();
            if let UpValue::Open(idx) = *upval {
                let val = self.inner.get_stack(idx - self.inner.get_base())?.clone();
                *upval = UpValue::Closed(val);
            }
        }
        Ok(())
    }

    fn get_metamethod(
        &self,
        heap: &mut gc::Heap,
        obj: &RuntimeValue,
        method: &MetaMethod,
    ) -> Option<RuntimeValue> {
        if let RuntimeValue::Table(t) = obj {
            t.borrow().metatable.and_then(|mt| {
                mt.borrow()
                    .inner
                    .get(&RuntimeValue::metamethod_key(heap, method))
                    .filter(|v| v.is_function())
                    .cloned()
            })
        } else {
            None
        }
    }

    /// 2 Arguments, 1 Result
    fn call_binary_metamethod(
        &mut self,
        heap: &mut gc::Heap,
        method: MetaMethod,
        left: RuntimeValue,
        right: RuntimeValue,
    ) -> Result<(), DukaRuntimeError> {
        let method_name = method.name();

        // Just try, we won't actually jijijiji
        if !left.is_table() && !right.is_table() {
            return Ok(());
        }

        let metamethod = self
            .get_metamethod(heap, &left, &method)
            .or_else(|| self.get_metamethod(heap, &right, &method))
            .ok_or(DukaRuntimeError::UnsupportedOperation(
                method_name,
                left.type_of(),
            ))?;

        let func_pos = self.inner.stack.len() - self.inner.get_base();

        self.inner.append_stack(metamethod)?;
        self.inner.append_stack(left)?;
        self.inner.append_stack(right)?;

        self.call(heap, func_pos, 2, 1, false)?;

        Ok(())
    }

    // fn call_unary_metamethod(
    //     &mut self,
    //     heap: &mut gc::Heap,
    //     method: MetaMethod,
    //     target: usize,
    // ) -> Result<(), DukaRuntimeError> {
    //     let target = self.inner.get_stack(target)?;
    //     let metamethod = self.get_metamethod(heap, target, &method).ok_or(
    //         DukaRuntimeError::UnsupportedOperation(method.name(), target.type_of()),
    //     )?;

    //     let func_pos = self.inner.stack.len() - self.inner.get_base();
    //     let target = target.clone();

    //     self.inner.append_stack(metamethod)?;
    //     self.inner.append_stack(target)?;

    //     self.call(heap, func_pos, 1, 1, false)?;

    //     Ok(())
    // }

    pub fn call(
        &mut self,
        heap: &mut gc::Heap,
        func: usize,
        narg: usize,
        nwanted: usize,
        tailcall: bool,
    ) -> Result<(), DukaRuntimeError> {
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        let callee = self.inner.get_stack(func)?;
        (!callee.is_function()).then_error(|| InvalidValueType(ctype::FUN))?;
        let base = self.inner.get_base();

        match callee {
            NativeFunc(closure) => {
                let f = closure.clone();
                let mut ptr = f.borrow_mut();

                let nreturn = match (ptr.func)(&mut self.inner, heap)? {
                    ValueCount::VarArg => self.inner.stack.len() - base,
                    ValueCount::Exact(n) => n,
                };

                if nreturn < nwanted {
                    let from = base + nreturn;
                    let count = nwanted - nreturn;
                    for i in 0..count {
                        self.inner.set_stack(i + from, Nil)?;
                    }
                } else {
                    let len = base + nwanted;
                    self.inner.adjust_stack(len);
                }
            }
            UserFunc(closure) => {
                let fixed_count = closure.func.param_count;
                let has_var_arg = closure.func.has_var_arg;

                if tailcall {
                    self.close_upvalues()?;
                    self.inner
                        .cut_stack(base - 1, ValueCount::Exact(base + func));
                }

                if narg < fixed_count {
                    let from = func + narg + 1 + base;
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
