use std::{collections::HashMap, ops::Add};

use crate::{
    error::DukaRuntimeError,
    types::{DukaProto, DukaVM, ExeState},
    value::RuntimeValue,
    vm::instructions::{Address, Bits25, DecodeInstruction},
};
use duka_shared::{
    constants::sugar,
    utils::OrError,
    value::{ConstValue, DukaFloat, DukaInt},
};

pub mod instructions;

#[derive(Debug)]
pub struct VM {
    state: ExeState,
    //gc_heap: GcHeap,
}

pub type ReturnCount = usize;

impl VM {
    pub fn new(params: Vec<RuntimeValue>) -> Self {
        let mut globals = HashMap::new();
        // globals.insert(
        //     "print".into(),
        //     ConstValue::Func(|s| {
        //         println!("{}", s.get_stack(1));
        //         0
        //     }),
        // );
        // globals.insert(
        //     sugar::TYPE_IS_TABLE.into(),
        //     ConstValue::Func(|s| {
        //         let res = matches!(s.get_stack(1), ConstValue::Table(_));
        //         s.set_stack(0, ConstValue::Bool(res));
        //         1
        //     }),
        // );
        Self {
            state: ExeState {
                globals,
                base: params.len(),
                stack: params,
                frames: vec![],
                upvalues: vec![],
            },
            //gc_heap: GcHeap::new(),
        }
    }
}

impl VM {
    // fn collect_roots(&self) -> Vec<&GcObject> {
    //     let mut roots = vec![];
    //     for val in &self.state.stack {
    //         todo!()
    //     }
    //     roots
    // }
}

macro_rules! op {
    ($op: tt) => {};
}

impl DukaVM for VM {
    type OkType = ReturnCount;

    fn execute(&mut self, proto: &DukaProto) -> Result<ReturnCount, DukaRuntimeError> {
        use DecodeInstruction::*;
        use DukaRuntimeError::*;
        use RuntimeValue::*;
        let mut pc: usize = 0;
        let mut extra_arg: Option<Bits25> = None;
        let mut var_args: Vec<RuntimeValue> = vec![];

        macro_rules! cast {
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
            /* getter */
            ([$e: expr]) => {
                $e as usize + self.state.base
            };
            (stack.len) => {
                self.state.stack.len()
            };
            (base) => {
                self.state.base
            };

            /* flow control */
            (move $e: expr) => {
                pc = ((pc as isize) + ($e as isize)) as usize;
            };
            (continue) => {
                pc += 1;
            };
            (skip) => {
                pc += 1;
            };

            /* read */
            (R($ad: expr; $ct: expr) $(@get)?) => {
                (0..$ct as Address).map(|i| self.state.get_stack($ad + i)).collect::<Result<Vec<_>, _>>()?
            };
            (R($ad: expr) $(@get)?) => {
                self.state.get_stack(($ad) as usize)?
            };
            (K($i: expr) $(@get)?) => {
                proto.constants[($i) as usize]
            };
            /* store */
            (R($a: expr) := R($b: expr)) => {
                let v = vm!(R($b) @get).clone();
                vm!(R($a) := v);
            };
            (R($a: expr) := K($b: expr)) => {
                let v = vm!(K($b) @get).clone();
                vm!(R($a) := v);
            };
            (R($a: expr) := $v: expr) => {
                self.state.set_stack(($a) as usize, $v)?;
            };
        }

        loop {
            let inst = &proto.instructions[pc];

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
                    for i in 0..count as Address {
                        vm!(R(a + i) := Nil);
                    }
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
                    let i = extra_arg.take().unwrap(); // checked
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
                    vm!(skip);
                }
                Less(a, b) => {
                    vm!(skip);
                }
                LessEqual(a, b) => {
                    vm!(skip);
                }
                Concat(a, count) => {
                    let mut operands = vec![];
                    for i in 0..count as Address {
                        operands.push(vm!(R(a + i)));
                    }
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
                        let (init, step) = (*init, *step);
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

                Closure(a, index) => {
                    // push closure to stack & initialize its upvalues
                }

                Call(ad, narg, nwanted) => {
                    let callee = vm!(R(ad));

                    let count = self.execute(todo!())?;
                    self.state.stack.drain(vm!([ad])..vm!(stack.len) - count);

                    let nwanted = nwanted as usize;
                    if count < nwanted {
                        for o in 0..nwanted - count {
                            vm!(R(count + o) := Nil);
                        }
                    }
                }
                CallSet(ad, func, narg) => {}
                TailCall(a, b, c) => {}

                Return(from, count) => {
                    let from = vm!([from]);
                    let count = count as usize;
                    self.state.stack.truncate(from + count);
                    return Ok(count);
                }
                Return0() => return Ok(0),
                // Return1(res) => {
                //     let res = vm!([res]);
                //     self.state.stack.truncate(res + 1);
                //     return Ok(1);
                // }
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
                EqualK(_, _) => todo!(),
                EqualI(_, _) => todo!(),
                LessI(_, _) => todo!(),
                LessEqualI(_, _) => todo!(),
                GreaterI(_, _) => todo!(),
                GreaterEqualI(_, _) => todo!(),
                SetList(_, _, _) => todo!(),

                VarArgPrepare(fixed_param_count) => {
                    let start = vm!([fixed_param_count]);
                    if proto.has_vararg {
                        var_args = if start >= vm!(stack.len) {
                            vec![]
                        } else {
                            self.state.stack.drain(start..).collect()
                        }
                    }
                }
                VarArg(a, count_plus_1) => {
                    for o in 0..(count_plus_1 - 1) {
                        let val = var_args
                            .get(o as usize)
                            .map(|v| v.clone())
                            .unwrap_or_else(|| RuntimeValue::Nil);

                        vm!(R(a + o as Address) := val);
                    }
                } //_ => return Err(UnimplementedInstruction),
            }
            vm!(continue);
        }

        fn for_number_check<T: PartialOrd>(init: T, limit: T, neg_step: bool) -> bool {
            !neg_step && init < limit || neg_step && init > limit
        }
    }
}
