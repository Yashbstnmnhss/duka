use duka_gc::Gc;
use duka_gc::{Finalize, Trace, Tracer};

use crate::value::{DukaClosure, RuntimeValue};

pub type Stack = Vec<RuntimeValue>;

/// 调用帧
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub pc: usize,
    pub proto: CallProto,
    /// VarArgPrepare 收集的变长实参
    pub var_args: Vec<RuntimeValue>,
    /// 实际传入的参数数量 (含固定参数 + varargs)
    pub narg: usize,
    /// VarArg 指令写入的值数量，SetList count=0 时使用
    pub va_count: usize,
}
#[derive(Debug, Clone)]
pub enum CallProto {
    Main {
        proto: Gc<DukaClosure>,
        base: usize,
    },
    Call {
        base: usize,
        proto: usize,
        wanted: usize,
    },
}

impl CallFrame {
    pub fn main(proto: Gc<DukaClosure>) -> Self {
        Self {
            pc: 0,
            var_args: vec![],
            narg: 0,
            va_count: 0,
            proto: CallProto::Main { proto, base: 0 },
        }
    }
    pub fn call(base: usize, proto: usize, wanted: usize) -> Self {
        Self {
            pc: 0,
            var_args: vec![],
            narg: 0,
            va_count: 0,
            proto: CallProto::Call {
                proto,
                base,
                wanted,
            },
        }
    }

    pub(crate) fn get_base(&self) -> usize {
        match self.proto {
            CallProto::Main { base, .. } => base,
            CallProto::Call { base, .. } => base,
        }
    }
    pub(crate) fn set_base(&mut self, val: usize) {
        match &mut self.proto {
            CallProto::Main { base, .. } => *base = val,
            CallProto::Call { base, .. } => *base = val,
        }
    }
}

impl Finalize for CallFrame {
    fn finalize(&self) {}
}

impl Trace for CallFrame {
    fn trace(&self, tracer: &mut Tracer) {
        self.proto.trace(tracer);
        for v in &self.var_args {
            v.trace(tracer);
        }
    }
}

impl Finalize for CallProto {
    fn finalize(&self) {}
}

impl Trace for CallProto {
    fn trace(&self, tracer: &mut Tracer) {
        if let CallProto::Main { proto: gc, .. } = self {
            tracer.mark(gc);
        }
    }
}
