use std::{
    cmp::Ordering,
    f64::consts::{E, PI},
};

use duka_gc::{Gc, GcCell, Heap};
use duka_macros::duka_builtin;
use duka_shared::{
    constants::{MetaMethod, ctype},
    types::ValueCount,
    value::{DukaFloat, DukaInt},
};

use crate::{
    builtin::{call_meta, required},
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
            input.type_name_of(),
        ))
    }
}

define_builtins! {
    fn:
        plain:
            "max" => impl_max,
            "min" => impl_min,
            "sum" => impl_sum,
            "abs" => impl_abs,
            "round" => impl_round,
            "ceil" => impl_ceil,
            "floor" => impl_floor,
            "sin" => impl_sin,
            "cos" => impl_cos,
            "tan" => impl_tan,
            "arcsin" => impl_arcsin,
            "arccos" => impl_arccos,
            "arctan" => impl_arctan,
            "arctan2" => impl_arctan2,
            "sqrt" => impl_sqrt,
            "deg_to_rad" => impl_deg_to_rad,
            "rad_to_deg" => impl_rad_to_deg,
            "randf" => impl_randf,
            "randi" => impl_randi,
            "set_seed" => impl_set_seed,
            "log" => impl_log,
            "ln" => impl_ln,
            "log2" => impl_log2,
            "log10" => impl_log10,
            "sign" => impl_sign;
        meta:
            "clamp" => impl_clamp, __DUKA_IMPL_CLAMP_META,
            "modf" => impl_modf, __DUKA_IMPL_MODF_META,
            "factors" => impl_factors, __DUKA_IMPL_FACTORS_META,
            "randf_range" => impl_randf_range, __DUKA_IMPL_RANDF_RANGE_META;
    const:
        meta:
            "PI" => RuntimeValue::Float(DUKA_PI), __DUKA_DUKA_PI_META,
            "E" => RuntimeValue::Float(DUKA_E), __DUKA_DUKA_E_META,
            "FLOAT_MAX" => RuntimeValue::Float(DUKA_FLOAT_MAX), __DUKA_DUKA_FLOAT_MAX_META,
            "INT_MAX" => RuntimeValue::Int(DUKA_INT_MAX), __DUKA_DUKA_INT_MAX_META;
}

#[duka_builtin(
    module = "math",
    name = "PI",
    doc = "Archimedes' constant (π)",
    value = "3.14159265358979323846264338327950288"
)]
const DUKA_PI: DukaFloat = PI;
#[duka_builtin(
    module = "math",
    name = "E",
    doc = "Euler's number (e)",
    value = "2.71828182845904523536028747135266250"
)]
const DUKA_E: DukaFloat = E;
#[duka_builtin(
    module = "math",
    name = "FLOAT_MAX",
    doc = "Largest finite float value",
    value = "1.7976931348623157e+308"
)]
const DUKA_FLOAT_MAX: DukaFloat = DukaFloat::MAX;
#[duka_builtin(
    module = "math",
    name = "INT_MAX",
    doc = "Largest finite int value",
    value = "9223372036854775807"
)]
const DUKA_INT_MAX: DukaInt = DukaInt::MAX;

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
        val.type_name_of(),
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
                val.type_name_of(),
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

#[duka_builtin(
    module = "math", name = "abs",
    doc = "Computes the absolute value of input",
    params(val: preserve_number),
    returns = "The absolute value"
)]
fn impl_abs(val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(if let RuntimeValue::Int(i) = val {
        RuntimeValue::Int(i.abs())
    } else if let RuntimeValue::Float(f) = val {
        RuntimeValue::Float(f.abs())
    } else {
        val
    })
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

#[duka_builtin(
    module = "math", name = "log",
    doc = "Returns the base y logarithm of the x number.",
    params(x: num, y: num),
    returns = "The result"
)]
fn impl_log(x: DukaFloat, y: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.log(y)))
}
#[duka_builtin(
    module = "math", name = "ln",
    doc = "Returns the base natural logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns = "The result"
)]
fn impl_ln(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.ln()))
}
#[duka_builtin(
    module = "math", name = "log10",
    doc = "Returns the base 10 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns = "The result"
)]
fn impl_log10(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log10()))
}

#[duka_builtin(
    module = "math", name = "log2",
    doc = "Returns the base 2 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns = "The result"
)]
fn impl_log2(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log2()))
}

#[duka_builtin(
    module = "math", name = "sign",
    doc = "Returns a number that represents the sign of it",
    params(val: preserve_number),
    returns = "The sign of it"
)]
fn impl_sign(val: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    if let RuntimeValue::Int(i) = val {
        Ok(RuntimeValue::Int(i.signum()))
    } else if let RuntimeValue::Float(f) = val {
        Ok(RuntimeValue::Float(f.signum()))
    } else {
        unreachable!()
    }
}

#[duka_builtin(
    module = "math", name = "clamp",
    doc = "Clamp a number into [lo, hi]",
    params(x: num, lo: num, hi: num),
    returns = "The clamped value",
)]
fn impl_clamp(
    x: DukaFloat,
    lo: DukaFloat,
    hi: DukaFloat,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.max(lo).min(hi)))
}

#[duka_builtin(
    module = "math", name = "modf",
    doc = "Split x into integer and fractional parts",
    params(x: num),
    returns = "two values: the integer part and the fractional part",
)]
fn impl_modf(x: DukaFloat) -> Result<(DukaInt, DukaFloat), DukaRuntimeError> {
    Ok((x.trunc() as DukaInt, x.fract()))
}

#[duka_builtin(
    module = "math", name = "factors",
    doc = "Return all factors of n",
    params(n: int),
    returns = "0..n values, one per factor",
)]
fn impl_factors(n: DukaInt) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    Ok((1..=n)
        .filter(|d| n % d == 0)
        .map(RuntimeValue::Int)
        .collect())
}

#[duka_builtin(
    module = "math", name = "randf_range",
    doc = "Random float in [lo, hi)",
    params(lo: num, hi: num),
    returns = "the random float",
)]
fn impl_randf_range(
    sv: &mut CoState,
    lo: DukaFloat,
    hi: DukaFloat,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let t = rand_u32(&mut sv.rng_state) as DukaFloat / (0xFFFFFFFFu32 as DukaFloat);
    Ok(RuntimeValue::Float(lo + (hi - lo) * t))
}
