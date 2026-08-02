use duka_gc::Heap;
use duka_shared::constants::ctype;
use duka_shared::value::{DukaFloat, DukaInt};
use duka_shared::{builtin::Builtins, types::ValueCount};

use crate::errors::DukaRuntimeError;
use crate::value::RuntimeValue;
use crate::vm::coroutine::CoState;

type BuiltinFn = fn(&mut CoState, &mut Heap) -> Result<ValueCount, DukaRuntimeError>;

pub fn registry() -> Builtins<BuiltinFn> {
    Builtins::new()
        .register("print", impl_print as BuiltinFn)
        .register("type", impl_type as BuiltinFn)
        .register("tostring", impl_tostring as BuiltinFn)
        .register("tonumber", impl_tonumber as BuiltinFn)
        .register("assert", impl_assert as BuiltinFn)
        .register("error", impl_error as BuiltinFn)
        .register("require", super::require::impl_require as BuiltinFn)
        .register("getmetatable", impl_getmetatable as BuiltinFn)
        .register("setmetatable", impl_setmetatable as BuiltinFn)
}

fn impl_print(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let args = sv.take_stack_many(1, ValueCount::VarArg)?;
    for i in 0..args.len() {
        print!("{}", args[i]);
        if i != args.len() - 1 {
            print!("\t")
        }
    }
    println!();
    Ok(ValueCount::Exact(0))
}

fn impl_type(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = sv.get_stack(1)?.clone();
    let name = val.type_of();
    sv.set_stack(0, RuntimeValue::from_short_str_unsafe(name))?;
    Ok(ValueCount::Exact(1))
}

fn impl_tostring(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = sv.get_stack(1)?.clone();
    let s = match val {
        RuntimeValue::Nil => "nil".to_owned(),
        RuntimeValue::Int(n) => n.to_string(),
        RuntimeValue::Float(f) => f.to_string(),
        RuntimeValue::Bool(b) => b.to_string(),
        _ => format!("{}", val),
    };
    sv.set_stack(0, RuntimeValue::from_string(_h, s))?;
    Ok(ValueCount::Exact(1))
}

fn impl_tonumber(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = sv.get_stack(1)?.clone();
    let n = match val {
        RuntimeValue::Int(n) => {
            sv.set_stack(0, RuntimeValue::Int(n))?;
            return Ok(ValueCount::Exact(1));
        }
        RuntimeValue::Float(f) => {
            sv.set_stack(0, RuntimeValue::Float(f))?;
            return Ok(ValueCount::Exact(1));
        }
        RuntimeValue::ShortString(..)
        | RuntimeValue::MediumString(_)
        | RuntimeValue::LongString(_) => {
            let s = format!("{}", val);
            if let Ok(n) = s.parse::<DukaInt>() {
                RuntimeValue::Int(n)
            } else if let Ok(f) = s.parse::<DukaFloat>() {
                RuntimeValue::Float(f)
            } else {
                sv.set_stack(0, RuntimeValue::Nil)?;
                return Ok(ValueCount::Exact(1));
            }
        }
        _ => {
            sv.set_stack(0, RuntimeValue::Nil)?;
            return Ok(ValueCount::Exact(1));
        }
    };
    sv.set_stack(0, n)?;
    Ok(ValueCount::Exact(1))
}

fn impl_assert(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let cond = sv.get_stack(1)?.clone();
    if !cond.eval_to_bool() {
        let msg = sv
            .get_stack(2)
            .ok()
            .map_or("assertion failed".to_owned(), |v| format!("{}", v));
        return Err(DukaRuntimeError::Custom(msg));
    }
    sv.set_stack(0, cond)?;
    Ok(ValueCount::Exact(1))
}

fn impl_error(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let msg = sv
        .get_stack(1)
        .map_or("error".to_owned(), |v| format!("{}", v));
    Err(DukaRuntimeError::Custom(msg))
}

fn impl_getmetatable(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = sv.get_stack(1)?.clone();
    let r = match val {
        RuntimeValue::Table(t) => t
            .borrow()
            .metatable
            .map(RuntimeValue::Table)
            .unwrap_or_default(),
        _ => RuntimeValue::Nil,
    };
    sv.set_stack(0, r)?;
    Ok(ValueCount::Exact(1))
}

fn impl_setmetatable(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let t = sv.get_stack(1)?.clone();
    let mt = sv
        .get_stack(2)
        .ok()
        .map_or(RuntimeValue::Nil, |v| v.clone());
    let RuntimeValue::Table(tab) = t else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    match mt {
        RuntimeValue::Nil => {
            tab.borrow_mut().metatable = None;
        }
        RuntimeValue::Table(mt) => {
            tab.borrow_mut().metatable = Some(mt);
        }
        _ => return Err(DukaRuntimeError::InvalidValueType(ctype::TAB)),
    }
    sv.set_stack(0, RuntimeValue::Table(tab))?;
    Ok(ValueCount::Exact(1))
}
