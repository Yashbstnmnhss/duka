use std::{cmp::Ordering, collections::HashMap};

use crate::{
    error::DukaRuntimeError,
    types::{DukaProto, DukaVM, ExeState},
    vm::instructions::{Address, Bits25, DecodeInstruction},
};
use duka_shared::{
    gc::{GcHeap, GcObject},
    types::DukaRuntime,
    value::{DukaInt, Value},
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
        globals.insert(
            "print".into(),
            Value::Func(|s| {
                println!("{}", s.get_stack(1));
                0
            }),
        );
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

macro_rules! setA {
    ($s: ident: $a: expr, $v: expr) => {
        $s.state.set_stack($a, $v);
    };
}

impl DukaVM for VM {
    fn execute(&mut self, proto: &DukaProto) -> Result<(), DukaRuntimeError> {
        use DecodeInstruction::*;

        let mut pc: usize = 0;
        let mut extra_arg: Option<Bits25> = None;

        while pc < proto.instructions.len() {
            let inst = &proto.instructions[pc];

            if inst.check_extra() && extra_arg.is_none() {
                return Err(DukaRuntimeError::ExtraArgNotFound);
            }

            let decoded = inst.decode();
            match decoded {
                Move(a, b) => {
                    let val = self.state.get_stack(b).clone();
                    setA!(self: a, val);
                }
                LoadTrue(a) => {
                    setA!(self: a, Value::Bool(true));
                }
                LoadFalse(a) => {
                    setA!(self: a, Value::Bool(false));
                }
                LoadNil(a) => {
                    setA!(self: a, Value::Nil);
                }
                LoadFalseSkip(a) => {
                    setA!(self: a, Value::Bool(false));
                    pc += 1; // skip next
                }
                LoadI(a, num) => {
                    setA!(self: a, Value::Int(num as DukaInt));
                }
                LoadK(a, k) => {
                    let v = proto.constants[k as usize].clone();
                    setA!(self: a, v);
                }
                LoadKX(a) => {
                    let i = extra_arg.take().unwrap();
                    let v = proto.constants[i as usize].clone();
                    setA!(self: a, v);
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

                Minus(a, b) => {}
                Not(a, b) => {}
                BitNot(a, b) => {}
                Length(a, b) => {}

                Jump(offset) => {
                    pc = ((pc as isize) + (offset as isize)) as usize;
                    continue;
                }

                Closure(a, index) => {}

                MarkToBeClosed(target) => {}
                Close(target) => {}

                ForPrepare(a, b) => {}
                ForLoop(a, b) => {}

                TForPrepare(a, b) => {}
                TForLoop(a, b) => {}
                TForCall(a, b) => {}

                ExtraArg(arg) => extra_arg = Some(arg),
                _ => unimplemented!(),
            }
            pc += 1;
        }
        Ok(())
    }
}
