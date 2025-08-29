use std::collections::HashMap;

use crate::{
    error::DukaRuntimeError,
    types::{DukaProto, DukaVM, ExeState},
    value::RuntimeValue,
    vm::instructions::{Address, Bits25, DecodeInstruction},
};
use duka_shared::{
    constants::sugar,
    gc::{GcHeap, GcObject},
    utils::OrError,
    value::{ConstValue, DukaInt},
};

pub mod instructions;

#[derive(Debug)]
pub struct VM {
    state: ExeState,
    gc_heap: GcHeap,
}

impl VM {
    pub fn new() -> Self {
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
                stack: Vec::new(),
                frames: vec![],
                upvalues: vec![],
            },
            gc_heap: GcHeap::new(),
        }
    }
}

impl VM {
    fn collect_roots(&self) -> Vec<&GcObject> {
        let mut roots = vec![];
        for val in &self.state.stack {
            todo!()
        }
        roots
    }
}

impl DukaVM for VM {
    fn execute(&mut self, proto: &DukaProto) -> Result<(), DukaRuntimeError> {
        use DecodeInstruction::*;
        use DukaRuntimeError::*;
        use RuntimeValue::*;

        let mut pc: usize = 0;
        let mut extra_arg: Option<Bits25> = None;

        macro_rules! vm {
            (goto $e: expr) => {
                pc = ((pc as isize) + ($e as isize)) as usize;
            };
            (next) => {
                pc += 1;
            };
            (skip) => {
                pc += 1;
            };
            (R($ad: expr) $(@get)?) => {
                self.state.get_stack($ad)?
            };
            (K($i: expr) $(@get)?) => {
                proto.constants[$i as usize]
            };
            (R($a: expr) := R($b: expr)) => {
                let v = vm!(R($b) @get).clone();
                vm!(R($a) := v);
            };
            (R($a: expr) := K($b: expr)) => {
                let v = vm!(K($b) @get).clone();
                vm!(R($a) := v);
            };
            (R($a: expr) := $v: expr) => {
                self.state.set_stack($a, $v)?;
            };
        }

        while pc < proto.instructions.len() {
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
                LoadNil(a) => {
                    vm!(R(a) := Nil);
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

                Concat(a, count) => {}

                Minus(a, b) => {
                    let v = match vm!(R(b)) {
                        Bool(..) | Nil => panic!(),
                        rv if rv.is_string() => panic!(),
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
                    // todo
                    let val = vm!(R(b)).eval_to_int().unwrap();
                    vm!(R(a) := Int(!val));
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
                    vm!(goto offset);
                    continue;
                }

                Test(a, b) => {
                    // skip next if R(a) != b
                    let val = vm!(R(a));
                    if val.eval_to_bool() != b {
                        vm!(skip);
                    }
                }
                TestSet(a, b, c) => {
                    let val = vm!(R(a));
                    if val.eval_to_bool() != c {
                        vm!(skip);
                    } else {
                        vm!(R(a) := R(b));
                    }
                }

                MarkToBeClosed(target) => {}
                Close(target) => {}

                ForPrepare(a, b) => {
                    if let Int(i) = vm!(R(a))
                        && let Int(step) = vm!(R(a + 2))
                    {
                        let lim = match vm!(R(a + 1)) {
                            Int(i) => i,
                            Float(f) => todo!(),
                            _ => panic!(),
                        };
                        vm!(goto b);
                    } else {
                    }
                }
                ForLoop(a, b) => {}

                TForPrepare(a, b) => {}
                TForLoop(a, b) => {}
                TForCall(a, b) => {}

                Closure(a, index) => {}
                Call(a, b, c) => {}
                TailCall(a, b, c) => {}
                Return(a, b, c) => {}
                Return0() => {}
                Return1(a) => {}

                ExtraArg(arg) => extra_arg = Some(arg),
                _ => return Err(DukaRuntimeError::UnimplementedInstruction),
            }
            vm!(next);
        }
        Ok(())
    }
}
