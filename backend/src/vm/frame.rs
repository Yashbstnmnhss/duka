use gc::Gc;
use gc::{Finalize, Trace, Tracer};
// gc_derive removed during migration; Trace/Finalize will be implemented by hand where needed.

use crate::value::{DukaClosure, RuntimeValue};

pub type Stack = Vec<RuntimeValue>;

/// 调用帧
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub pc: usize,
    pub proto: CallProto,
}
#[derive(Debug, Clone)]
pub enum CallProto {
    Main(Gc<DukaClosure>),
    Call {
        base: usize,
        proto: usize,
        wanted: usize,
    },
}

impl CallFrame {
    pub fn new_main(proto: Gc<DukaClosure>) -> Self {
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

impl Finalize for CallFrame {
    fn finalize(&self) {}
}

impl Trace for CallFrame {
    fn trace(&self, tracer: &mut Tracer) {
        match &self.proto {
            CallProto::Main(gc) => tracer.mark(gc),
            CallProto::Call { .. } => {}
        }
    }
}

impl Finalize for CallProto {
    fn finalize(&self) {}
}

impl Trace for CallProto {
    fn trace(&self, tracer: &mut Tracer) {
        if let CallProto::Main(gc) = self {
            tracer.mark(gc);
        }
    }
}
