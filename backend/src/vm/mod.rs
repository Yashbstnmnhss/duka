use std::{cmp::Ordering, collections::HashMap};

use crate::{
    types::{DukaProto, DukaVM},
    vm::instructions::DecodeInstruction,
};
use duka_shared::{types::ExeState, value::Value};

pub mod instructions;

#[derive(Debug)]
pub struct VM {
    state: ExeState,
}

impl VM {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        globals.insert(
            "print".into(),
            Value::Func(|s| {
                //println!("{}", s.get(1));
                0
            }),
        );
        Self {
            state: ExeState {
                globals,
                stack: Vec::new(),
            },
        }
    }
}

impl DukaVM for ExeState {
    fn execute(&mut self, proto: &DukaProto) {
        let mut iter = proto.instructions.iter();

        for code in proto.instructions.iter() {
            match code.decode() {
                _ => unimplemented!(),
            }
        }
        dbg!(&self.globals);
        dbg!(&self.stack);
    }
}
