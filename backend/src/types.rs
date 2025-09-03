use duka_shared::value::ConstValue;

use crate::error::DukaRuntimeError;
use crate::value::RuntimeValue;
use crate::vm::instructions::{Address, Instruction};
use std::cmp::Ordering;
use std::collections::HashMap;

/// 函数原型
#[derive(Debug, Clone)]
pub struct DukaProto {
    pub constants: Vec<RuntimeValue>,
    pub instructions: Vec<Instruction>,
    pub upvalue_count: usize,
    pub param_count: usize,
    pub nested_protos: Vec<DukaProto>,
    pub debug_name: Option<String>,
    pub has_vararg: bool,
}

/// 捕获值
#[derive(Debug, Clone)]
pub struct Upvalue {
    pub value: ConstValue,
    pub index: usize,
    pub closed: bool,
}

/// 闭包
#[derive(Debug, Clone)]
pub struct Closure {
    pub proto: DukaProto,
    pub upvalues: Vec<Upvalue>,
}

/// 调用帧
#[derive(Debug)]
pub struct CallFrame {
    pub pc: usize,
    pub base: usize,
    pub proto: DukaProto,
}

/// 运行状态
#[derive(Debug)]
pub struct ExeState {
    pub globals: HashMap<String, RuntimeValue>,
    pub stack: Vec<RuntimeValue>,
    pub upvalues: Vec<Upvalue>,
    pub frames: Vec<CallFrame>,
    pub base: usize,
}
impl ExeState {
    pub fn new() -> Self {
        Self {
            globals: HashMap::new(),
            stack: vec![],
            upvalues: vec![],
            frames: vec![],
            base: 0,
        }
    }
    pub fn get_stack(&self, ad: usize) -> Result<&RuntimeValue, DukaRuntimeError> {
        let dst = ad + self.base;
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => Ok(&self.stack[dst]),
            _ => Err(DukaRuntimeError::OutOfStack),
        }
    }
    pub fn set_stack(&mut self, ad: usize, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
        let dst = ad + self.base;
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => return Err(DukaRuntimeError::OutOfStack),
        }
        Ok(())
    }
}

pub trait DukaVM {
    type OkType;

    fn execute(&mut self, proto: &DukaProto) -> Result<Self::OkType, DukaRuntimeError>;
}
