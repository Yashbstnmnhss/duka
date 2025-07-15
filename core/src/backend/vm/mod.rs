use std::{cmp::Ordering, collections::HashMap};

use crate::{
    backend::vm::instructions::DecodeInstruction,
    shared::{
        types::{DukaProto, DukaVM},
        value::Value,
    },
};

pub mod instructions;

#[derive(Debug)]
pub struct ExeState {
    globals: HashMap<String, Value>,
    stack: Vec<Value>,
}

impl ExeState {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        globals.insert(
            "print".into(),
            Value::Func(|s| {
                println!("{}", s.get_stack(1));
                0
            }),
        );
        ExeState {
            globals,
            stack: Vec::new(),
        }
    }
    fn get_stack(&mut self, i: u8) -> &Value {
        let dst = i as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => &self.stack[dst],
            _ => panic!("Invalid get_stack"),
        }
    }
    fn set_stack(&mut self, i: u8, val: Value) {
        let dst = i as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => panic!("Invalid set_stack"),
        }
    }
}

impl DukaVM for ExeState {
    fn execute(&mut self, proto: &DukaProto) {
        for code in proto.instructions.iter() {
            match code.decode() {
                DecodeInstruction::GetGlobal(dst, id) => {
                    let id: &str = (&proto.constants[id as usize]).into();
                    let val = self.globals.get(id).unwrap_or(&Value::Nil).clone();
                    self.set_stack(dst, val);
                }
                DecodeInstruction::LoadConst(dst, c) => {
                    let val = proto.constants[c as usize].clone();
                    self.set_stack(dst, val);
                }
                DecodeInstruction::Call(func, _) => {
                    let func = self.get_stack(func);
                    if let Value::Func(f) = func {
                        f(self);
                    }
                }
                _ => unimplemented!(),
            }
        }
        dbg!(&self.globals);
        dbg!(&self.stack);
    }
}
