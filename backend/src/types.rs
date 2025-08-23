use duka_shared::error::DukaError;
use duka_shared::types::DukaRuntime;
use duka_shared::value::Value;
use std::cmp::Ordering;
use std::collections::HashMap;

use crate::error::DukaRuntimeError;
use crate::vm::instructions::Instruction;

/// 函数原型
#[derive(Debug, Clone)]
pub struct DukaProto {
    pub constants: Vec<Value>,
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
    pub value: Value,
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
    pub globals: HashMap<String, Value>,
    pub stack: Vec<Value>,
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
}

impl DukaRuntime for ExeState {
    fn get_stack(&mut self, ad: u8) -> &Value {
        let dst = ad as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Greater => &self.stack[dst],
            _ => panic!("[DukaRuntime] Out of stack"),
        }
    }
    fn set_stack(&mut self, ad: u8, val: Value) {
        let dst = ad as usize;
        match self.stack.len().cmp(&dst) {
            Ordering::Equal => self.stack.push(val),
            Ordering::Greater => self.stack[dst] = val,
            _ => panic!("[DukaRuntime] "),
        }
    }
}

pub trait DukaVM {
    fn execute(&mut self, proto: &DukaProto) -> Result<(), DukaRuntimeError>;
}
