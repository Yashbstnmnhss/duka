use std::cmp::Ordering;

use crate::{
    error::DukaRuntimeError,
    instructions::{Address, DecodeInstruction, Instruction},
    value::{DukaProto, RuntimeValue, Upvalue, ValueCount},
    vm::{
        Bits25, ExecuteResult,
        frame::{CallFrame, CallProto, Stack},
    },
};
use duka_macros::Info;
use duka_shared::{
    utils::OrError,
    value::{DukaFloat, DukaInt},
};
use gc::Gc;
use gc_derive::{Finalize, Trace};

const INIT_CAPACITY: usize = 16;

/// 协程运行状态
#[derive(Debug, Trace, Finalize)]
pub struct CoState {
    /// 协程的值栈
    pub stack: Stack,
    /// 上值们
    pub upvalues: Vec<Upvalue>,
    /// 调用帧
    pub frames: Vec<CallFrame>,
}
impl CoState {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(INIT_CAPACITY),
            upvalues: vec![],
            frames: vec![],
        }
    }
    #[inline(always)]
    pub fn with_proto(proto: Gc<DukaProto>) -> Self {
        Self {
            stack: Vec::with_capacity(INIT_CAPACITY),
            upvalues: vec![],
            frames: vec![CallFrame::new_main(proto)],
        }
    }

    fn get_proto(&self) -> Result<&Gc<DukaProto>, DukaRuntimeError> {
        match &self.current().proto {
            CallProto::Main(p) => Ok(p),
            CallProto::Call { proto, .. } => self.get_stack(*proto).and_then(|v| match v {
                RuntimeValue::UserFunc(p) => Ok(p),
                _ => Err(DukaRuntimeError::InvalidValueType("duka prototype")),
            }),
        }
    }
    fn fetch(&self) -> Result<&Instruction, DukaRuntimeError> {
        self.get_proto().map(|p| &p.instructions[self.current().pc])
    }

    #[inline(always)]
    pub fn push_frame(&mut self, frame: CallFrame) {
        self.frames.push(frame);
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
    #[inline(always)]
    fn pc(&mut self) -> &mut usize {
        &mut self.current_mut().pc
    }

    pub fn get_stack(&self, ad: usize) -> Result<&RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.get_base();
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => Ok(&self.stack[dst]),
            _ => Err(DukaRuntimeError::OutOfStack),
        }
    }
    pub fn set_stack(&mut self, ad: usize, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
        let dst = ad + self.get_base();
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => return Err(DukaRuntimeError::OutOfStack),
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
}
impl Coroutine {
    #[inline(always)]
    pub fn new(id: CoroutineID, state: CoState) -> Self {
        Self {
            id,
            status: CoroutineStatus::Ready,
            inner: state,
        }
    }

    /// ### Push a frame of calling into this coroutine
    pub fn push_frame(&mut self, frame: CallFrame) {
        self.inner.push_frame(frame);
    }
}
impl Coroutine {
    /// ### Where instructions are executed exactly
    pub fn execute(&mut self) -> Result<ExecuteResult, DukaRuntimeError> {
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
            (@stack:pad ..[$to: expr]) => {
                for _ in 0..$to - self.inner.stack.len() {
                    self.inner.stack.push(Nil);
                }
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

            /* read *HAS BASE */
            (R($ad: expr; $ct: expr) $(@get)?) => {
                (0..$ct as usize).map(|i| self.inner.get_stack(vm!([$ad as usize + i] for R))).collect::<Result<Vec<_>, _>>()?
            };
            (R($ad: expr) $(@get)?) => {
                self.inner.get_stack(vm!([$ad] for R))?
            };
            (K($i: expr) $(@get)?) => {
                self.inner.get_proto()?.constants[($i) as usize]
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
                    if a.eq(b) {
                        vm!(skip);
                    }
                }
                Less(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if true {
                        vm!(skip);
                    }
                }
                LessEqual(a, b) => {
                    let (a, b) = (vm!(R(a)), vm!(R(b)));
                    if true {
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
                            (b.array.len() + b.map.len()) as DukaInt
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
                                return Err(InvalidValueType("int or float for limit value"));
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
                    // push closure to stack & initialize its upvalues
                }

                Call(func, narg, nwanted) => {
                    cast!(as narg: usize, nwanted: usize, func: usize);

                    let callee = vm!(R(func));
                    (!callee.is_function()).then_error(|| InvalidValueType("function"))?;

                    match callee {
                        NativeFunc(f) => {
                            let count_ = (f.func)(&mut self.inner)?;
                            let nreturn = cast!(
                                for count_
                                all(from vm!([func + 1] for R))
                                as usize
                            );

                            if nreturn < nwanted {
                                vm!(R(vm!([nreturn] for R); nwanted - nreturn) := fill Nil);
                            } else {
                                vm!(@stack:remove [vm!([nwanted] for R)]..);
                            }
                        }
                        UserFunc(p) => {
                            let fixed_param_count = p.param_count;
                            if narg < fixed_param_count {
                                vm!(R(vm!([func + narg + 1] for R); fixed_param_count - narg) := fill Nil);
                            } else if narg > fixed_param_count && !p.has_vararg {
                                vm!(@stack:remove [vm!([func + narg + 1] for R)]..);
                            }

                            let frame = CallFrame::call(vm!(@top) - narg, func, nwanted);
                            self.push_frame(frame);
                        }
                        _ => unreachable!(),
                    };
                }
                CallSet(ad, func, narg) => {}
                TailCall(a, b, c) => {}

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
                        return Ok(ExecuteResult::Return(ValueCount::Exact(actual_count)));
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
                        return Ok(ExecuteResult::Return(ValueCount::Exact(0)));
                    };

                    vm!(R(0; wanted) := fill Nil); // fill with nil
                    vm!(@stack:remove [vm!([wanted] for R)]..); // remove tail
                }
                // Extra argument is before the target instruction
                ExtraArg(arg) => extra_arg = Some(arg),

                GetUpVal(_, _) => todo!(),
                SetUpVal(_, _) => todo!(),
                GetTabUp(_, _, _) => todo!(),
                GetTable(_, _, _) => todo!(),
                GetI(_, _, _) => todo!(),
                GetField(_, _, _) => todo!(),
                SetTabUp(_, _, _) => todo!(),
                SetTable(_, _, _) => todo!(),
                SetI(_, _, _) => todo!(),
                SetField(_, _, _) => todo!(),
                NewTable(_, _, _) => todo!(),
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
                EqualK(ad, k) => todo!(),
                EqualI(target, ad, im) => {
                    let n = vm!(R(ad));
                    n.is_number().or_else_error(|| InvalidValueType("number"))?;

                    if cmp_im(|x, y| x == y, im as DukaInt)(n) && target {
                        vm!(skip);
                    }
                }
                LessI(target, ad, i) => todo!(),
                LessEqualI(target, ad, i) => todo!(),
                GreaterI(target, ad, i) => todo!(),
                GreaterEqualI(target, ad, i) => todo!(),
                SetList(_, _, _) => todo!(),

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
            }
            vm!(continue);
        }

        #[inline(always)]
        fn for_number_check<T: PartialOrd>(init: T, limit: T, neg_step: bool) -> bool {
            !neg_step && init < limit || neg_step && init > limit
        }

        #[inline(always)]
        fn cmp_im(fu: fn(DukaInt, DukaInt) -> bool, im: DukaInt) -> impl Fn(&RuntimeValue) -> bool {
            move |v| -> bool {
                match v {
                    Int(i) => fu(*i, im),
                    Float(f) => fu(*f as DukaInt, im),
                    _ => panic!("INVALID OPERATION"),
                }
            }
        }
    }
}
