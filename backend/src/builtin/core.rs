use duka_gc::Heap;
use duka_shared::constants::{MetaMethod, ctype};
use duka_shared::value::{DukaFloat, DukaInt};
use duka_shared::{builtin::Builtins, types::ValueCount};

use crate::builtin::{BuiltinFn, call_meta, required};
use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeValue, RustClosure, make_pairs_iterator};
use crate::vm::coroutine::CoState;
use duka_gc::GcCell;

pub fn registry() -> Builtins<BuiltinFn> {
    Builtins::new()
        .register("print", impl_print as BuiltinFn)
        .register("type", impl_type as BuiltinFn)
        .register("tostring", impl_to_string as BuiltinFn)
        .register("tonumber", impl_to_number as BuiltinFn)
        .register("to_string", impl_to_string as BuiltinFn)
        .register("to_number", impl_to_number as BuiltinFn)
        .register("assert", impl_assert as BuiltinFn)
        .register("error", impl_error as BuiltinFn)
        .register("require", super::require::impl_require as BuiltinFn)
        .register("getmetatable", impl_get_metatable as BuiltinFn)
        .register("setmetatable", impl_set_metatable as BuiltinFn)
        .register("get_metatable", impl_get_metatable as BuiltinFn)
        .register("set_metatable", impl_set_metatable as BuiltinFn)
        .register("pairs", impl_pairs as BuiltinFn)
        .register("ipairs", impl_ipairs as BuiltinFn)
}

fn impl_print(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let args = sv.take_stack_many(1, ValueCount::VarArg)?;
    for i in 0..args.len() {
        print!("{}", format_arg(sv, h, &args[i])?);
        if i != args.len() - 1 {
            print!(" ")
        }
    }
    println!();
    Ok(ValueCount::Exact(0))
}

fn format_arg(
    sv: &mut CoState,
    h: &mut Heap,
    val: &RuntimeValue,
) -> Result<String, DukaRuntimeError> {
    match val {
        RuntimeValue::Table(t) => match call_meta(sv, h, *t, MetaMethod::ToString, &[])? {
            Some(s) => Ok(s.eval_to_string().into_owned()),
            None => Ok(format!("{}", val)),
        },
        _ => Ok(format!("{}", val)),
    }
}

fn impl_type(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "type", "value")?.clone();
    let name = val.type_of();
    sv.set_stack(0, RuntimeValue::from_short_str_unsafe(name))?;
    Ok(ValueCount::Exact(1))
}

fn impl_to_string(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "to_string", "value")?.clone();
    let s = match val {
        RuntimeValue::Table(t) => match call_meta(sv, h, t, MetaMethod::ToString, &[])? {
            Some(s) => s.eval_to_string().into_owned(),
            None => ctype::TAB.to_owned(),
        },
        RuntimeValue::Nil => ctype::NIL.to_owned(),
        RuntimeValue::Int(n) => n.to_string(),
        RuntimeValue::Float(f) => f.to_string(),
        RuntimeValue::Bool(b) => b.to_string(),
        _ => format!("{}", val),
    };
    sv.set_stack(0, RuntimeValue::from_string(h, s))?;
    Ok(ValueCount::Exact(1))
}

fn impl_to_number(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "to_number", "value")?.clone();
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
    let cond = required(sv, 0, "assert", "condition")?.clone();
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

fn impl_get_metatable(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "get_metatable", "table")?.clone();
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

fn impl_set_metatable(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let t = required(sv, 0, "set_metatable", "table")?.clone();
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

/// `pairs(t)` 返回 `(iter, t, nil)` 三元组:
/// 每次 `iter(s, control)` 消费一个条目,返回 `(k, v)`,耗尽返回 `nil`
fn impl_pairs(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let t = required(sv, 0, "pairs", "table")?.clone();
    let RuntimeValue::Table(tab) = t else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    let entries: Vec<(RuntimeValue, RuntimeValue)> = tab
        .borrow()
        .inner
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let func = make_pairs_iterator(h, entries);
    sv.set_stack(0, func)?;
    sv.set_stack(1, t)?;
    sv.set_stack(2, RuntimeValue::Nil)?;
    Ok(ValueCount::Exact(3))
}

/// `ipairs(t)` 返回 `(iter, t, nil)`,`iter` 从整数键 0 开始连续迭代
fn impl_ipairs(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let t = required(sv, 0, "ipairs", "table")?.clone();
    let RuntimeValue::Table(tab) = t else {
        return Err(DukaRuntimeError::InvalidValueType(ctype::TAB));
    };
    let mut items: Vec<RuntimeValue> = vec![];
    {
        let tab = tab.borrow();
        let mut i: DukaInt = 0;
        while let Some(v) = tab.array_get(i as usize) {
            items.push(v.clone());
            i += 1;
        }
    }
    let mut iter = items.into_iter().enumerate();
    let func = RustClosure::returns(move |c, _h| match iter.next() {
        Some((i, v)) => {
            c.set_stack(0, RuntimeValue::Int(i as DukaInt))?;
            c.set_stack(1, v)?;
            Ok(ValueCount::Exact(2))
        }
        None => {
            c.set_stack(0, RuntimeValue::Nil)?;
            Ok(ValueCount::Exact(1))
        }
    });
    let func = h.alloc(GcCell::new(func));
    sv.set_stack(0, RuntimeValue::NativeFunc(func))?;
    sv.set_stack(1, t)?;
    sv.set_stack(2, RuntimeValue::Nil)?;
    Ok(ValueCount::Exact(3))
}
