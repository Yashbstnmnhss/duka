use std::{
    cmp::Ordering,
    f64::consts::{E, PI},
};

use duka_gc::{Gc, GcCell, Heap};
use duka_macros::duka_builtin;
use duka_shared::{
    constants::MetaMethod,
    value::{DukaFloat, DukaInt},
};

use crate::{
    builtin::call_meta,
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::{CoState, NativeApi},
};

define_builtins! {
    fn:
        meta:
            "clamp" => impl_clamp, __DUKA_IMPL_CLAMP_META,
            "modf" => impl_modf, __DUKA_IMPL_MODF_META,
            "factors" => impl_factors, __DUKA_IMPL_FACTORS_META,
            "randf_range" => impl_randf_range, __DUKA_IMPL_RANDF_RANGE_META,
            "max" => impl_max Co, __DUKA_IMPL_MAX_META,
            "min" => impl_min Co, __DUKA_IMPL_MIN_META,
            "sum" => impl_sum Co, __DUKA_IMPL_SUM_META,
            "abs" => impl_abs, __DUKA_IMPL_ABS_META,
            "round" => impl_round, __DUKA_IMPL_ROUND_META,
            "ceil" => impl_ceil, __DUKA_IMPL_CEIL_META,
            "floor" => impl_floor, __DUKA_IMPL_FLOOR_META,
            "sin" => impl_sin, __DUKA_IMPL_SIN_META,
            "cos" => impl_cos, __DUKA_IMPL_COS_META,
            "tan" => impl_tan, __DUKA_IMPL_TAN_META,
            "arcsin" => impl_arcsin, __DUKA_IMPL_ARCSIN_META,
            "arccos" => impl_arccos, __DUKA_IMPL_ARCCOS_META,
            "arctan" => impl_arctan, __DUKA_IMPL_ARCTAN_META,
            "arctan2" => impl_arctan2, __DUKA_IMPL_ARCTAN2_META,
            "sqrt" => impl_sqrt, __DUKA_IMPL_SQRT_META,
            "deg_to_rad" => impl_deg_to_rad, __DUKA_IMPL_DEG_TO_RAD_META,
            "rad_to_deg" => impl_rad_to_deg, __DUKA_IMPL_RAD_TO_DEG_META,
            "randf" => impl_randf, __DUKA_IMPL_RANDF_META,
            "randi" => impl_randi, __DUKA_IMPL_RANDI_META,
            "set_seed" => impl_set_seed, __DUKA_IMPL_SET_SEED_META,
            "log" => impl_log, __DUKA_IMPL_LOG_META,
            "ln" => impl_ln, __DUKA_IMPL_LN_META,
            "log2" => impl_log2, __DUKA_IMPL_LOG2_META,
            "log10" => impl_log10, __DUKA_IMPL_LOG10_META,
            "sign" => impl_sign, __DUKA_IMPL_SIGN_META;
    const:
        meta:
            "PI" => RuntimeValue::Float(DUKA_PI), __DUKA_DUKA_PI_META,
            "E" => RuntimeValue::Float(DUKA_E), __DUKA_DUKA_E_META,
            "FLOAT_MAX" => RuntimeValue::Float(DUKA_FLOAT_MAX), __DUKA_DUKA_FLOAT_MAX_META,
            "INT_MAX" => RuntimeValue::Int(DUKA_INT_MAX), __DUKA_DUKA_INT_MAX_META,
            "INF" => RuntimeValue::Float(DUKA_INF), __DUKA_DUKA_INF_META,
            "NAN" => RuntimeValue::Float(DUKA_NAN), __DUKA_DUKA_NAN_META;
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
#[duka_builtin(module = "math", name = "INF", doc = "Infinity", value = "INFINITY")]
const DUKA_INF: DukaFloat = DukaFloat::INFINITY;
#[duka_builtin(module = "math", name = "NAN", doc = "Not a number", value = "NAN")]
const DUKA_NAN: DukaFloat = DukaFloat::NAN;

fn call_compare_meta(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    t: Gc<GcCell<RuntimeDukaTable>>,
    other: &RuntimeValue,
) -> Result<Ordering, DukaRuntimeError> {
    Ok(
        if call_meta(sv, h, api, t, MetaMethod::LT, &[other.clone()])?
            .map(|t| t.eval_to_bool())
            .unwrap_or(false)
        {
            Ordering::Less
        } else if call_meta(sv, h, api, t, MetaMethod::Eq, &[other.clone()])?
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
    api: &mut NativeApi,
    val: &RuntimeValue,
    other: &RuntimeValue,
) -> Result<Ordering, DukaRuntimeError> {
    if val.is_number() && other.is_number() {
        let v = val.eval_to_float().expect("Checked");
        let o = other.eval_to_float().expect("Checked");
        return Ok(v.total_cmp(&o));
    }

    if let RuntimeValue::Table(t) = val {
        return call_compare_meta(sv, h, api, t.clone(), other);
    }

    if let RuntimeValue::Table(t) = other {
        return call_compare_meta(sv, h, api, t.clone(), val).map(|o| o.reverse());
    }

    Err(DukaRuntimeError::UnsupportedOperation(
        "compare",
        val.type_name_of(),
    ))
}

fn add(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
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
            if let Some(v) = call_meta(sv, h, api, t.clone(), MetaMethod::Add, &[b.clone()])? =>
        {
            v
        }
        (a, RuntimeValue::Table(t))
            if let Some(v) = call_meta(sv, h, api, t.clone(), MetaMethod::Add, &[a.clone()])? =>
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

#[duka_builtin(
    module = "math",
    name = "max",
    doc = "Calculate the maximum value in given values (or table)",
    params(vals: vararg),
    returns(any)
)]
fn impl_max(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    vals: Vec<RuntimeValue>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(if vals.is_empty() {
        RuntimeValue::Nil
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

                if compare(sv, h, api, &res, val)?.is_lt() {
                    res = val.clone()
                }
            }
            res
        } else {
            vals[0].clone()
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

            if compare(sv, h, api, &res, val)?.is_lt() {
                res = val.clone()
            }
        }
        res
    })
}

#[duka_builtin(
    module = "math",
    name = "min",
    doc = "Calculate the minimum value in given values (or table)",
    params(vals: vararg),
    returns(any)
)]
fn impl_min(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    vals: Vec<RuntimeValue>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(if vals.is_empty() {
        RuntimeValue::Nil
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

                if compare(sv, h, api, &res, val)?.is_gt() {
                    res = val.clone()
                }
            }
            res
        } else {
            vals[0].clone()
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

            if compare(sv, h, api, &res, val)?.is_gt() {
                res = val.clone()
            }
        }
        res
    })
}

#[duka_builtin(
    module = "math",
    name = "sum",
    doc = "Calculate sum for given values (or table)",
    params(vals: vararg),
    returns(any)
)]
fn impl_sum(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    vals: Vec<RuntimeValue>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(if vals.is_empty() {
        RuntimeValue::Nil
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

                res = add(sv, h, api, &res, val)?
            }
            res
        } else {
            vals[0].clone()
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

            res = add(sv, h, api, &res, val)?
        }
        res
    })
}

#[duka_builtin(
    module = "math", name = "abs",
    doc = "Computes the absolute value of input",
    params(val: preserve_number),
    returns(any)
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

#[duka_builtin(
    module = "math", name = "floor",
    doc = "Returns the largest integer that is less than or equal to self",
    params(val: num),
    returns(int)
)]
fn impl_floor(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.floor() as DukaInt))
}
#[duka_builtin(
    module = "math", name = "ceil",
    doc = "Returns the smallest integer that is greater than or equal to self",
    params(val: num),
    returns(int)
)]
fn impl_ceil(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.ceil() as DukaInt))
}
#[duka_builtin(
    module = "math", name = "round",
    doc = "Returns the nearest integer to self. If a value is half-way between two integers, round away from 0.0",
    params(val: num),
    returns(int)
)]
fn impl_round(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.round() as DukaInt))
}

#[duka_builtin(
    module = "math", name = "sin",
    doc = "Computes the sine of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_sin(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.sin()))
}
#[duka_builtin(
    module = "math", name = "cos",
    doc = "Computes the cosine of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_cos(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.cos()))
}
#[duka_builtin(
    module = "math", name = "tan",
    doc = "Computes the tangent of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_tan(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.tan()))
}
#[duka_builtin(
    module = "math", name = "arcsin",
    doc = "Computes the arcsine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1",
    params(val: num),
    returns(float)
)]
fn impl_arcsin(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.asin()))
}
#[duka_builtin(
    module = "math", name = "arccos",
    doc = "Computes the arccosine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1",
    params(val: num),
    returns(float)
)]
fn impl_arccos(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.acos()))
}
#[duka_builtin(
    module = "math", name = "arctan",
    doc = "Computes the arctangent of a number. Return value is in radians in the range -pi/2, pi/2",
    params(val: num),
    returns(float)
)]
fn impl_arctan(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.atan()))
}

#[duka_builtin(
    module = "math", name = "arctan2",
    doc = "Computes the four quadrant arctangent of val and val2 in radians",
    params(val: num, val2: num),
    returns(float)
)]
fn impl_arctan2(val: DukaFloat, val2: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.atan2(val2)))
}

#[duka_builtin(
    module = "math", name = "sqrt",
    doc = "Returns the square root of a number. Returns NaN if self is a negative number other than -0.0",
    params(val: num),
    returns(float)
)]
fn impl_sqrt(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.sqrt()))
}
#[duka_builtin(
    module = "math", name = "deg_to_rad",
    doc = "Converts degrees to radians",
    params(val: num),
    returns(float)
)]
fn impl_deg_to_rad(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.to_radians()))
}
#[duka_builtin(
    module = "math", name = "rad_to_deg",
    doc = "Converts radians to degrees",
    params(val: num),
    returns(float)
)]
fn impl_rad_to_deg(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.to_degrees()))
}

const RANDOM_FALLBACK: u32 = 0x6C8E9CF5;

#[duka_builtin(
    module = "math",
    name = "randi",
    doc = "Generate random integer, from 0 to MAX",
    params(),
    returns(int)
)]
fn impl_randi(sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(rand_u32(&mut sv.rng_state).into()))
}
#[duka_builtin(
    module = "math",
    name = "randf",
    doc = "Generate random float, from 0 to 1 (exclusive)",
    params(),
    returns(float)
)]
fn impl_randf(sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(
        (rand_u32(&mut sv.rng_state) as DukaFloat) / (0xFFFFFFFFu32 as DukaFloat),
    ))
}

#[duka_builtin(
    module = "math",
    name = "set_seed",
    doc = "Set seed for random generation (only accepts integer)",
    params(seed: num),
)]
fn impl_set_seed(sv: &mut CoState, seed: DukaFloat) -> Result<(), DukaRuntimeError> {
    sv.rng_state = seed as u32;
    Ok(())
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
    returns(float)
)]
fn impl_log(x: DukaFloat, y: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.log(y)))
}
#[duka_builtin(
    module = "math", name = "ln",
    doc = "Returns the base natural logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_ln(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.ln()))
}
#[duka_builtin(
    module = "math", name = "log10",
    doc = "Returns the base 10 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_log10(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log10()))
}

#[duka_builtin(
    module = "math", name = "log2",
    doc = "Returns the base 2 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_log2(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log2()))
}

#[duka_builtin(
    module = "math", name = "sign",
    doc = "Returns a number that represents the sign of it",
    params(val: preserve_number),
    returns(int)
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
    returns(num),
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
    returns(int, float),
)]
fn impl_modf(x: DukaFloat) -> Result<(DukaInt, DukaFloat), DukaRuntimeError> {
    Ok((x.trunc() as DukaInt, x.fract()))
}

#[duka_builtin(
    module = "math", name = "factors",
    doc = "Return all factors of n",
    params(n: int),
    returns(vararg),
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
    returns(float),
)]
fn impl_randf_range(
    sv: &mut CoState,
    lo: DukaFloat,
    hi: DukaFloat,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let t = rand_u32(&mut sv.rng_state) as DukaFloat / (0xFFFFFFFFu32 as DukaFloat);
    Ok(RuntimeValue::Float(lo + (hi - lo) * t))
}
