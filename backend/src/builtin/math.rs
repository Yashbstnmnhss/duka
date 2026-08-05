use std::{
    cmp::Ordering,
    f64::consts::{E, PI},
};

use duka_gc::{Gc, GcCell, Heap};
use duka_shared::{
    builtin::Builtins,
    constants::{MetaMethod, ctype},
    types::ValueCount,
    value::{DukaFloat, DukaInt},
};

use crate::{
    builtin::{BuiltinFn, call_meta, required},
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::CoState,
};

fn ensure_num(input: &RuntimeValue, func: &str, param: usize) -> Result<(), DukaRuntimeError> {
    if input.is_number() {
        Ok(())
    } else {
        Err(DukaRuntimeError::ArgumentInvalidType(
            param,
            func.to_string(),
            ctype::NUM,
            input.type_of(),
        ))
    }
}

pub fn registry() -> Builtins<BuiltinFn> {
    Builtins::new()
        .register("max", impl_max as BuiltinFn)
        .register("min", impl_min as BuiltinFn)
        .register("sum", impl_sum as BuiltinFn)
        .register("abs", impl_abs as BuiltinFn)
        .register("round", impl_round as BuiltinFn)
        .register("ceil", impl_ceil as BuiltinFn)
        .register("floor", impl_floor as BuiltinFn)
        .register("sin", impl_sin as BuiltinFn)
        .register("cos", impl_cos as BuiltinFn)
        .register("tan", impl_tan as BuiltinFn)
        .register("arcsin", impl_arcsin as BuiltinFn)
        .register("arccos", impl_arccos as BuiltinFn)
        .register("arctan", impl_arctan as BuiltinFn)
        .register("arctan2", impl_arctan2 as BuiltinFn)
        .register("sqrt", impl_sqrt as BuiltinFn)
        .register("deg_to_rad", impl_deg_to_rad as BuiltinFn)
        .register("rad_to_deg", impl_rad_to_deg as BuiltinFn)
        .register("randf", impl_randf as BuiltinFn)
        .register("set_seed", impl_set_seed as BuiltinFn)
        .register("randi", impl_randi as BuiltinFn)
}
pub fn consts_registry() -> Builtins<RuntimeValue> {
    Builtins::new()
        .register("PI", RuntimeValue::Float(PI))
        .register("E", RuntimeValue::Float(E))
}

fn call_compare_meta(
    sv: &mut CoState,
    h: &mut Heap,
    t: Gc<GcCell<RuntimeDukaTable>>,
    other: &RuntimeValue,
) -> Result<Ordering, DukaRuntimeError> {
    Ok(
        if call_meta(sv, h, t, MetaMethod::LT, &[other.clone()])?
            .map(|t| t.eval_to_bool())
            .unwrap_or(false)
        {
            Ordering::Less
        } else if call_meta(sv, h, t, MetaMethod::Eq, &[other.clone()])?
            .map(|t| t.eval_to_bool())
            .unwrap_or(false)
        {
            Ordering::Equal
        } else {
            Ordering::Greater
        },
    )
}

fn compare(
    sv: &mut CoState,
    h: &mut Heap,
    val: &RuntimeValue,
    other: &RuntimeValue,
) -> Result<Ordering, DukaRuntimeError> {
    if val.is_number() && other.is_number() {
        let v = val.eval_to_float().expect("Checked");
        let o = other.eval_to_float().expect("Checked");
        return Ok(v.total_cmp(&o));
    }

    if let RuntimeValue::Table(t) = val {
        return call_compare_meta(sv, h, t.clone(), other);
    }

    if let RuntimeValue::Table(t) = other {
        return call_compare_meta(sv, h, t.clone(), val).map(|o| o.reverse());
    }

    Err(DukaRuntimeError::UnsupportedOperation(
        "compare",
        val.type_of(),
    ))
}

fn add(
    sv: &mut CoState,
    h: &mut Heap,
    val: &RuntimeValue,
    other: &RuntimeValue,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(match (val, other) {
        (RuntimeValue::Int(i), RuntimeValue::Int(i2)) => RuntimeValue::Int(*i + *i2),
        (RuntimeValue::Float(i), RuntimeValue::Int(i2)) => {
            RuntimeValue::Float(*i + (*i2 as DukaFloat))
        }
        (RuntimeValue::Int(i), RuntimeValue::Float(i2)) => {
            RuntimeValue::Float((*i as DukaFloat) + *i2)
        }
        (RuntimeValue::Float(i), RuntimeValue::Float(i2)) => RuntimeValue::Float(*i + *i2),
        (a, b) if a.is_string() && b.is_string() => {
            RuntimeValue::from_string(h, format!("{}{}", a.eval_to_string(), b.eval_to_string()))
        }
        (RuntimeValue::Table(t), b)
            if let Some(v) = call_meta(sv, h, t.clone(), MetaMethod::Add, &[b.clone()])? =>
        {
            v
        }
        (a, RuntimeValue::Table(t))
            if let Some(v) = call_meta(sv, h, t.clone(), MetaMethod::Add, &[a.clone()])? =>
        {
            v
        }
        _ => {
            return Err(DukaRuntimeError::UnsupportedOperation(
                MetaMethod::Add.name(),
                val.type_of(),
            ));
        }
    })
}

fn impl_max(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let vals = sv.take_stack_many(1, ValueCount::VarArg)?;
    if vals.is_empty() {
        sv.set_stack(0, RuntimeValue::Nil)?
    } else if vals.len() == 1 {
        if let RuntimeValue::Table(t) = vals[0] {
            let tab = t.borrow();
            let mut res = RuntimeValue::Nil;
            for (_, val) in &(*tab).inner {
                if res.is_nil() {
                    if val.is_nil() {
                        continue;
                    }
                    res = val.clone();
                    continue;
                }

                if compare(sv, h, &res, val)?.is_lt() {
                    res = val.clone()
                }
            }
            sv.set_stack(0, res)?;
        } else {
            sv.set_stack(0, vals[0].clone())?;
        }
    } else {
        let mut res = RuntimeValue::Nil;
        for val in &vals {
            if res.is_nil() {
                if val.is_nil() {
                    continue;
                }
                res = val.clone();
                continue;
            }

            if compare(sv, h, &res, val)?.is_lt() {
                res = val.clone()
            }
        }
        sv.set_stack(0, res)?;
    };
    Ok(ValueCount::Exact(1))
}

fn impl_min(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let vals = sv.take_stack_many(1, ValueCount::VarArg)?;
    if vals.is_empty() {
        sv.set_stack(0, RuntimeValue::Nil)?
    } else if vals.len() == 1 {
        if let RuntimeValue::Table(t) = vals[0] {
            let tab = t.borrow();
            let mut res = RuntimeValue::Nil;
            for (_, val) in &(*tab).inner {
                if res.is_nil() {
                    if val.is_nil() {
                        continue;
                    }
                    res = val.clone();
                    continue;
                }

                if compare(sv, h, &res, val)?.is_gt() {
                    res = val.clone()
                }
            }
            sv.set_stack(0, res)?;
        } else {
            sv.set_stack(0, vals[0].clone())?;
        }
    } else {
        let mut res = RuntimeValue::Nil;
        for val in &vals {
            if res.is_nil() {
                if val.is_nil() {
                    continue;
                }
                res = val.clone();
                continue;
            }

            if compare(sv, h, &res, val)?.is_gt() {
                res = val.clone()
            }
        }
        sv.set_stack(0, res)?;
    };
    Ok(ValueCount::Exact(1))
}
fn impl_sum(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let vals = sv.take_stack_many(1, ValueCount::VarArg)?;
    if vals.is_empty() {
        sv.set_stack(0, RuntimeValue::Nil)?
    } else if vals.len() == 1 {
        if let RuntimeValue::Table(t) = vals[0] {
            let tab = t.borrow();
            let mut res = RuntimeValue::Nil;
            for (_, val) in &(*tab).inner {
                if res.is_nil() {
                    if val.is_nil() {
                        continue;
                    }
                    res = val.clone();
                    continue;
                }

                res = add(sv, h, &res, val)?
            }
            sv.set_stack(0, res)?;
        } else {
            sv.set_stack(0, vals[0].clone())?;
        }
    } else {
        let mut res = RuntimeValue::Nil;
        for val in &vals {
            if res.is_nil() {
                if val.is_nil() {
                    continue;
                }
                res = val.clone();
                continue;
            }

            res = add(sv, h, &res, val)?
        }
        sv.set_stack(0, res)?;
    };
    Ok(ValueCount::Exact(1))
}

fn impl_abs(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "abs", "value")?.clone();
    if let RuntimeValue::Int(i) = val {
        sv.set_stack(0, RuntimeValue::Int(i.abs()))?;
    } else if let RuntimeValue::Float(f) = val {
        sv.set_stack(0, RuntimeValue::Float(f.abs()))?;
    } else {
        sv.set_stack(0, val)?;
    }
    Ok(ValueCount::Exact(1))
}

fn impl_floor(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "floor", "value")?.clone();
    if let RuntimeValue::Float(f) = val {
        sv.set_stack(0, RuntimeValue::Int(f.floor() as DukaInt))?;
    } else {
        sv.set_stack(0, val)?;
    }
    Ok(ValueCount::Exact(1))
}
fn impl_ceil(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "ceil", "value")?.clone();
    if let RuntimeValue::Float(f) = val {
        sv.set_stack(0, RuntimeValue::Int(f.ceil() as DukaInt))?;
    } else {
        sv.set_stack(0, val)?;
    }
    Ok(ValueCount::Exact(1))
}
fn impl_round(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "round", "value")?.clone();
    if let RuntimeValue::Float(f) = val {
        sv.set_stack(0, RuntimeValue::Int(f.round() as DukaInt))?;
    } else {
        sv.set_stack(0, val)?;
    }
    Ok(ValueCount::Exact(1))
}

fn impl_sin(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "sin", "value")?.clone();
    ensure_num(&val, "sin", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.sin()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_cos(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "cos", "value")?.clone();
    ensure_num(&val, "cos", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.cos()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_tan(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "tan", "value")?.clone();
    ensure_num(&val, "tan", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.tan()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_arcsin(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "arcsin", "value")?.clone();
    ensure_num(&val, "arcsin", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.asin()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_arccos(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "arccos", "value")?.clone();
    ensure_num(&val, "arccos", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.acos()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_arctan(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "arctan", "value")?.clone();
    ensure_num(&val, "arctan", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.atan()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_arctan2(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "arctan2", "value")?.clone();
    ensure_num(&val, "arctan2", 0)?;
    let val2 = required(sv, 1, "arctan2", "value2")?.clone();
    ensure_num(&val2, "arctan2", 1)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.atan2(val2.eval_to_float().unwrap_or_default())))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}

fn impl_sqrt(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "sqrt", "value")?.clone();
    ensure_num(&val, "sqrt", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.sqrt()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_deg_to_rad(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "deg_to_rad", "value")?.clone();
    ensure_num(&val, "deg_to_rad", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.to_radians()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}
fn impl_rad_to_deg(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let val = required(sv, 0, "rad_to_deg", "value")?.clone();
    ensure_num(&val, "rad_to_deg", 0)?;
    sv.set_stack(
        0,
        val.eval_to_float()
            .map(|v| RuntimeValue::Float(v.to_degrees()))
            .unwrap_or(val),
    )?;
    Ok(ValueCount::Exact(1))
}

const RANDOM_FALLBACK: u32 = 0x6C8E9CF5;

fn impl_randi(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let v = rand_u32(&mut sv.rng_state);
    sv.set_stack(0, RuntimeValue::Int(v.into()))?;
    Ok(ValueCount::Exact(1))
}
fn impl_randf(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let v = rand_u32(&mut sv.rng_state);
    sv.set_stack(
        0,
        RuntimeValue::Float((v as DukaFloat) / (0xFFFFFFFFu32 as DukaFloat)),
    )?;
    Ok(ValueCount::Exact(1))
}

fn impl_set_seed(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let seed = required(sv, 0, "set_seed", "seed")?;
    ensure_num(seed, "set_seed", 0)?;
    sv.rng_state = seed
        .eval_to_int()
        .map(|v| v as u32)
        .unwrap_or(RANDOM_FALLBACK);
    Ok(ValueCount::Exact(0))
}

fn rand_u32(state: &mut u32) -> u32 {
    if *state == 0 {
        // 防止0 0是不动点
        *state = RANDOM_FALLBACK;
    }

    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}
