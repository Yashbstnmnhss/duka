use std::cmp::Ordering;

use crate::{
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
    constants::{ctype, cvm},
    utils::OrError,
    value::{DukaFloat, DukaInt},
};
use gc::{Gc, GcCell};
use gc_derive::{Finalize, Trace};

const INIT_CAPACITY: usize = 16;

/// 协程运行状态
#[derive(Debug, Trace, Finalize)]
pub struct CoState {
    /// 协程的值栈
    pub stack: Stack,
    /// 调用帧
    pub frames: Vec<CallFrame>,
}
impl CoState {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(INIT_CAPACITY),
            frames: vec![],
        }
    }
    #[inline(always)]
    pub fn from_closure(closure: Gc<DukaClosure>) -> Self {
        Self {
            stack: Vec::with_capacity(INIT_CAPACITY),
            frames: vec![CallFrame::new_main(closure)],
        }
    }
    #[inline(always)]
    pub fn from_proto(proto: Gc<DukaProto>) -> Self {
        Self::from_closure(Gc::new(DukaClosure::new(proto)))
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
        self.frames.last().expect("WHERE IS YOUR MAIN FRAME?")
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

    pub fn get_stack(&self, ad: usize) -> Result<&RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.get_base();
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => Ok(&self.stack[dst]),
            _ => Err(DukaRuntimeError::OutOfRange(ctype::NUM)),
        }
    }
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

pub type CoroutineID = usize;

/// # 协程状态
#[derive(Debug, Trace, Finalize, Info)]
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
#[derive(Debug, Trace, Finalize)]
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
impl Coroutine {
    /// ### Where instructions are executed exactly
    pub fn execute(&mut self, ctx: &mut VMContext) -> Result<CoAction, DukaRuntimeError> {
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
            (K($i: expr) $(@get)?) => {
                self.inner.get_closure()?.func.constants.get($i as usize).ok_or(
                    OutOfRange(cvm::CONST)
                )?
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
                let v = vm!(K($b) @get).clone();
                vm!(R($a) := v);
            }};
            (R($a: expr) := $v: expr) => {
                self.inner.set_stack(vm!([$a] for R), $v)?;
            };
        }

        loop {
            let inst = self.inner.fetch()?;

            (inst.check_extra() && extra_arg.is_none()).then_error(|| ExtraArgNotFound)?;

            let decoded = inst.decode();
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
                Add(a, b, c) => {}
                Sub(a, b, c) => {}
                Mul(a, b, c) => {}
                Div(a, b, c) => {}
                IDiv(a, b, c) => {}
                Mod(a, b, c) => {}
                Pow(a, b, c) => {}
                BitAnd(a, b, c) => {}
                BitOr(a, b, c) => {}
                BitXor(a, b, c) => {}
                ShiftL(a, b, c) => {}
                ShiftR(a, b, c) => {}
                Equal(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_eq(a, b)? {
                        vm!(skip);
                    }
                }
                Less(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_le(a, b)? {
                        vm!(skip);
                    }
                }
                LessEqual(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if cmp_lt(a, b)? {
                        vm!(skip);
                    }
                }
                Concat(a, count) => {
                    let operands = vm!(R(a; count));
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
                    let v = match vm!(R(b)) {
                        LongString(l) => l.len() as DukaInt,
                        MediumString(m) => m.0 as DukaInt,
                        ShortString(s, _) => *s as DukaInt,
                        Table(t) => {
                            let b = t.borrow();
                            b.len() as DukaInt
                        }
                        _ => todo!(),
                    };
                    vm!(R(a) := Int(v));
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

                MarkToBeClosed(target) => {}
                Close(target) => {}

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

                TForPrepare(a, b) => {}
                TForLoop(a, b) => {}
                TForCall(a, b) => {}

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
                    let upvalues = vec![];
                    let closure = Gc::new(DukaClosure {
                        func: Gc::new(proto.clone()),
                        upvalues,
                    });
                    vm!(R(ad) := UserFunc(closure));
                }

                Call(func, narg, nwanted) => {
                    cast!(as func: usize, narg: usize, nwanted: usize);
                    self.call(func, narg, nwanted, false)?;
                }
                CallSet(ad, func, narg) => {
                    cast!(as func: usize, narg: usize);
                    self.call(func, narg, 1, false)?;
                    let ad = vm!([ad] for R);
                    self.inner.stack.swap(ad, func);
                }
                TailCall(func, narg) => {
                    cast!(as func: usize, narg: usize);
                    self.call(func, narg, 0, true)?;
                }

                Return(from, count_) => {
                    cast!(as from: usize);
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
                    *upval = UpValue::Closed(val);
                }

                GetTabUp(_, _, _) => todo!(),
                GetTable(_, _, _) => todo!(),
                GetI(_, _, _) => {}
                GetField(_, _, _) => todo!(),
                SetTabUp(_, _, _, _) => todo!(),
                SetTable(_, _, _) => todo!(),
                SetI(_, _, _) => todo!(),
                SetField(_, _, _) => todo!(),
                NewTable(a, narray, nmap) => {
                    let n = vm!(E());
                    cast!(as narray: usize, nmap: usize);
                    let table = Table(Gc::new(GcCell::new(RuntimeDukaTable::new(narray, nmap))));
                    vm!(R(a) := table);
                }
                Self_(_, _, _) => todo!(),
                AddI(_, _, _) => todo!(),
                AddK(_, _, _) => todo!(),
                SubK(_, _, _) => todo!(),
                MulK(_, _, _) => todo!(),
                ModK(_, _, _) => todo!(),
                PowK(_, _, _) => todo!(),
                DivK(_, _, _) => todo!(),
                IDivK(_, _, _) => todo!(),
                BitAndK(_, _, _) => todo!(),
                BitOrK(_, _, _) => todo!(),
                BitXorK(_, _, _) => todo!(),
                ShiftRI(_, _, _) => todo!(),
                ShiftLI(_, _, _) => todo!(),
                MMBinary(_, _, _) => todo!(),
                MMBinaryI(_, _) => todo!(),
                MMBinaryK(_, _, _) => todo!(),

                EqualK(ad, k, target) => {
                    let (a, k) = (vm!(R(ad)), vm!(K(k)));

                    if cmp_eq(a, k)? && !target {
                        vm!(skip);
                    }
                }
                EqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x == y, im as DukaInt)(n)? && !target {
                        vm!(skip);
                    }
                }
                LessI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x < y, im as DukaInt)(n)? && !target {
                        vm!(skip);
                    }
                }
                LessEqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x <= y, im as DukaInt)(n)? && !target {
                        vm!(skip);
                    }
                }
                GreaterI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x > y, im as DukaInt)(n)? && !target {
                        vm!(skip);
                    }
                }
                GreaterEqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number()
                        .or_else_error(|| InvalidValueType(ctype::NUM))?;

                    if cmp_im(|x, y| x >= y, im as DukaInt)(n)? && !target {
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

        #[inline(always)]
        fn cmp_im(
            fu: fn(DukaInt, DukaInt) -> bool,
            im: DukaInt,
        ) -> impl Fn(&RuntimeValue) -> Result<bool, DukaRuntimeError> {
            move |v| -> Result<bool, DukaRuntimeError> {
                match v {
                    Int(i) => Ok(fu(*i, im)),
                    Float(f) => Ok(fu(*f as DukaInt, im)),
                    _ => Err(InvalidValueType(ctype::CMP)),
                }
            }
        }

        #[inline(always)]
        fn cmp_lt(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
            match (a, b) {
                (Int(a), Int(b)) => Ok(a < b),
                (Int(a), Float(b)) => Ok((*a as DukaFloat) < *b),
                (Float(a), Int(b)) => Ok(*a < *b as DukaFloat),
                (Float(a), Float(b)) => Ok(a < b),
                _ => Err(InvalidValueType(ctype::CMP)),
            }
        }
        #[inline(always)]
        fn cmp_le(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
            match (a, b) {
                (Int(a), Int(b)) => Ok(a <= b),
                (Int(a), Float(b)) => Ok(*a as DukaFloat <= *b),
                (Float(a), Int(b)) => Ok(*a <= *b as DukaFloat),
                (Float(a), Float(b)) => Ok(a <= b),
                _ => Err(InvalidValueType(ctype::CMP)),
            }
        }
        #[inline(always)]
        fn cmp_eq(a: &RuntimeValue, b: &RuntimeValue) -> Result<bool, DukaRuntimeError> {
            Ok(a.eq(b))
        }

        // #[inline(always)]
        // fn arch_im
    }

    #[inline(always)]
    fn call(
        &mut self,
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

                let nreturn = match (ptr.func)(&mut self.inner)? {
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
