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
    pub max_stack_size: usize,
    pub nested_protos: Vec<DukaProto>,
    pub debug_name: Option<String>,
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
}
impl ExeState {
    pub fn new() -> Self {
        Self {
            globals: HashMap::new(),
            stack: vec![],
            upvalues: vec![],
            frames: vec![],
        }
    }
    pub fn get_stack(&mut self, ad: Address) -> Result<&RuntimeValue, DukaRuntimeError> {
        let dst = ad as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => Ok(&self.stack[dst]),
            _ => Err(DukaRuntimeError::OutOfStack),
        }
    }
    pub fn set_stack(&mut self, ad: Address, val: RuntimeValue) -> Result<(), DukaRuntimeError> {
        let dst = ad as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => return Err(DukaRuntimeError::OutOfStack),
        }
        Ok(())
    }
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto) -> Result<(), DukaRuntimeError>;
}
