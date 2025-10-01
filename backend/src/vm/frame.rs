use gc::Gc;
use gc_derive::{Finalize, Trace};

use crate::value::{DukaProto, RuntimeValue};

pub type Stack = Vec<RuntimeValue>;

/// 调用帧
#[derive(Debug, Trace, Finalize, Clone)]
pub struct CallFrame {
    pub pc: usize,
    pub proto: CallProto,
}
#[derive(Debug, Trace, Finalize, Clone)]
pub enum CallProto {
    Main(Gc<DukaProto>),
    Call {
        base: usize,
        proto: usize,
        wanted: usize,
    },
}

impl CallFrame {
    pub fn new_main(proto: Gc<DukaProto>) -> Self {
        Self {
            pc: 0,
            proto: CallProto::Main(proto),
        }
    }
    pub fn call(base: usize, proto: usize, wanted: usize) -> Self {
        Self {
            pc: 0,
            proto: CallProto::Call {
                proto,
                base,
                wanted,
            },
        }
    }

    pub(crate) const fn base(&self) -> usize {
        match self.proto {
            CallProto::Main { .. } => 0,
            CallProto::Call { base, .. } => base,
        }
    }
}
