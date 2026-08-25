use std::{
    cmp::Ordering,
    f64::consts::{E, PI},
    ops::Rem,
};

use duka_gc::{Gc, GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::{
    constants::MetaMethod,
    value::{DukaFloat, DukaInt},
};

use crate::{
    builtin::call_meta_method,
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::{CoState, NativeApi},
};

duka_builtin_def! {
    mod math
    fn {
        meta:
            impl_clamp,
            impl_modf,
            impl_factors,
            impl_randf_range,
            impl_randi_range,
            impl_max co,
            impl_min co,
            impl_sum co,
            impl_abs,
            impl_round,
            impl_ceil,
            impl_floor,
            impl_sin,
            impl_cos,
            impl_tan,
            impl_arcsin,
            impl_arccos,
            impl_arctan,
            impl_arctan2,
            impl_sqrt,
            impl_deg_to_rad,
            impl_rad_to_deg,
            impl_randf,
            impl_randi,
            impl_set_seed,
            impl_log,
            impl_ln,
            impl_log2,
            impl_log10,
            impl_sign,
            impl_exp,
            impl_hypot,
            impl_is_nan,
            impl_is_inf,
            impl_fmod,
    }
    const {
        meta:
            DUKA_PI,
            DUKA_E,
            DUKA_FLOAT_MAX,
            DUKA_INT_MAX,
            DUKA_INF,
            DUKA_NAN
    }
}

#[duka_builtin(
    type = "float",
    name = "PI",
    doc = "Archimedes' constant (π)",
    value = "3.14159265358979323846264338327950288"
)]
const DUKA_PI: RuntimeValue = RuntimeValue::Float(PI);
#[duka_builtin(
    type = "float",
    name = "E",
    doc = "Euler's number (e)",
    value = "2.71828182845904523536028747135266250"
)]
const DUKA_E: RuntimeValue = RuntimeValue::Float(E);
#[duka_builtin(
    type = "float",
    name = "FLOAT_MAX",
    doc = "Largest finite float value",
    value = "1.7976931348623157e+308"
)]
const DUKA_FLOAT_MAX: RuntimeValue = RuntimeValue::Float(DukaFloat::MAX);
#[duka_builtin(
    type = "int",
    name = "INT_MAX",
    doc = "Largest finite int value",
    value = "9223372036854775807"
)]
const DUKA_INT_MAX: RuntimeValue = RuntimeValue::Int(DukaInt::MAX);
#[duka_builtin(type = "float", name = "INF", doc = "Infinity", value = "INFINITY")]
const DUKA_INF: RuntimeValue = RuntimeValue::Float(DukaFloat::INFINITY);
#[duka_builtin(type = "float", name = "NAN", doc = "Not a number", value = "NAN")]
const DUKA_NAN: RuntimeValue = RuntimeValue::Float(DukaFloat::NAN);

fn call_compare_meta(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    t: Gc<GcCell<RuntimeDukaTable>>,
    other: &RuntimeValue,
) -> Result<Ordering, DukaRuntimeError> {
    Ok(
        if call_meta_method(sv, h, api, t, MetaMethod::LT, std::slice::from_ref(other))?
            .map(|t| t.eval_to_bool())
            .unwrap_or(false)
        {
            Ordering::Less
        } else if call_meta_method(sv, h, api, t, MetaMethod::Eq, std::slice::from_ref(other))?
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
        return call_compare_meta(sv, h, api, *t, other);
    }

    if let RuntimeValue::Table(t) = other {
        return call_compare_meta(sv, h, api, *t, val).map(|o| o.reverse());
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
            if let Some(v) =
                call_meta_method(sv, h, api, *t, MetaMethod::Add, std::slice::from_ref(b))? =>
        {
            v
        }
        (a, RuntimeValue::Table(t))
            if let Some(v) = call_meta_method(
                sv,
                h,
                api,
                t.clone(),
                MetaMethod::Add,
                std::slice::from_ref(a),
            )? =>
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
    name = "max",
    doc = "Calculate the maximum value in given values (or table/array)",
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
        } else if let RuntimeValue::Array(a) = vals[0] {
            let arr = a.borrow();
            let mut res = RuntimeValue::Nil;
            for val in &(*arr).items {
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
    name = "min",
    doc = "Calculate the minimum value in given values (or table/array)",
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
        } else if let RuntimeValue::Array(a) = vals[0] {
            let arr = a.borrow();
            let mut res = RuntimeValue::Nil;
            for val in &(*arr).items {
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
    name = "sum",
    doc = "Calculate sum for given values (or table/array)",
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
        } else if let RuntimeValue::Array(a) = vals[0] {
            let arr = a.borrow();
            let mut res = RuntimeValue::Nil;
            for val in &(*arr).items {
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
     name = "abs",
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
     name = "floor",
    doc = "Returns the largest integer that is less than or equal to self",
    params(val: num),
    returns(int)
)]
fn impl_floor(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.floor() as DukaInt))
}
#[duka_builtin(
     name = "ceil",
    doc = "Returns the smallest integer that is greater than or equal to self",
    params(val: num),
    returns(int)
)]
fn impl_ceil(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.ceil() as DukaInt))
}
#[duka_builtin(
     name = "round",
    doc = "Returns the nearest integer to self. If a value is half-way between two integers, round away from 0.0",
    params(val: num),
    returns(int)
)]
fn impl_round(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(val.round() as DukaInt))
}

#[duka_builtin(
     name = "sin",
    doc = "Computes the sine of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_sin(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.sin()))
}
#[duka_builtin(
     name = "cos",
    doc = "Computes the cosine of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_cos(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.cos()))
}
#[duka_builtin(
    name = "tan",
    doc = "Computes the tangent of a number (in radians)",
    params(val: num),
    returns(float)
)]
fn impl_tan(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.tan()))
}
#[duka_builtin(
     name = "arcsin",
    doc = "Computes the arcsine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1",
    params(val: num),
    returns(float)
)]
fn impl_arcsin(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.asin()))
}
#[duka_builtin(
     name = "arccos",
    doc = "Computes the arccosine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1",
    params(val: num),
    returns(float)
)]
fn impl_arccos(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.acos()))
}
#[duka_builtin(
     name = "arctan",
    doc = "Computes the arctangent of a number. Return value is in radians in the range -pi/2, pi/2",
    params(val: num),
    returns(float)
)]
fn impl_arctan(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.atan()))
}

#[duka_builtin(
     name = "arctan2",
    doc = "Computes the four quadrant arctangent of val and val2 in radians",
    params(val: num, val2: num),
    returns(float)
)]
fn impl_arctan2(val: DukaFloat, val2: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.atan2(val2)))
}

#[duka_builtin(
     name = "sqrt",
    doc = "Returns the square root of a number. Returns NaN if self is a negative number other than -0.0",
    params(val: num),
    returns(float)
)]
fn impl_sqrt(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.sqrt()))
}
#[duka_builtin(
     name = "deg_to_rad",
    doc = "Converts degrees to radians",
    params(val: num),
    returns(float)
)]
fn impl_deg_to_rad(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.to_radians()))
}
#[duka_builtin(
     name = "rad_to_deg",
    doc = "Converts radians to degrees",
    params(val: num),
    returns(float)
)]
fn impl_rad_to_deg(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.to_degrees()))
}

const RANDOM_FALLBACK: u32 = 0x6C8E9CF5;

#[duka_builtin(
    name = "randi",
    doc = "Generate random integer, from 0 to MAX",
    params(),
    returns(int)
)]
fn impl_randi(sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(rand_u32(&mut sv.rng_state).into()))
}
#[duka_builtin(
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
     name = "log",
    doc = "Returns the base y logarithm of the x number.",
    params(x: num, y: num),
    returns(float)
)]
fn impl_log(x: DukaFloat, y: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.log(y)))
}
#[duka_builtin(
     name = "ln",
    doc = "Returns the base natural logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_ln(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.ln()))
}
#[duka_builtin(
     name = "log10",
    doc = "Returns the base 10 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_log10(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log10()))
}

#[duka_builtin(
     name = "log2",
    doc = "Returns the base 2 logarithm of the number.\nThis returns NaN when the number is negative, and negative infinity when number is zero.",
    params(val: num),
    returns(float)
)]
fn impl_log2(val: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(val.log2()))
}

#[duka_builtin(
     name = "sign",
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
     name = "clamp",
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
    name = "fmod",
    doc = "Computes remainder of the floating point division operation",
    params(x: num, y: num),
    returns(float),
)]
fn impl_fmod(x: DukaFloat, y: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.rem(y)))
}
#[duka_builtin(
    name = "modf",
    doc = "Split x into integer and fractional parts",
    params(x: num),
    returns(int, float),
)]
fn impl_modf(x: DukaFloat) -> Result<(DukaInt, DukaFloat), DukaRuntimeError> {
    Ok((x.trunc() as DukaInt, x.fract()))
}

#[duka_builtin(
     name = "factors",
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
    name = "randf_range",
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

#[duka_builtin(
    name = "randi_range",
    doc = "Random integer in [lo, hi)",
    params(lo: int, hi: int),
    returns(int),
)]
fn impl_randi_range(
    sv: &mut CoState,
    lo: DukaInt,
    hi: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let lo = lo as DukaFloat;
    let hi = hi as DukaFloat;
    let t = rand_u32(&mut sv.rng_state) as DukaFloat / (0xFFFFFFFFu32 as DukaFloat);
    Ok(RuntimeValue::Int((lo + (hi - lo) * t).floor() as DukaInt))
}

#[duka_builtin(
    name = "hypot",
    doc = "Computes the euclidean norm",
    params(x: num, y: num),
    returns(float)
)]
fn impl_hypot(x: DukaFloat, y: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.hypot(y)))
}
#[duka_builtin(
    name = "exp",
    doc = "Computes exponential function",
    params(x: num),
    returns(float)
)]
fn impl_exp(x: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Float(x.exp()))
}

#[duka_builtin(
    name = "is_nan",
    doc = "Whether x is `NAN`",
    params(x: num),
    returns(float)
)]
fn impl_is_nan(x: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Bool(x.is_nan()))
}

#[duka_builtin(
    name = "is_inf",
    doc = "Whether x is `INFINITY`",
    params(x: num),
    returns(float)
)]
fn impl_is_inf(x: DukaFloat) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Bool(x.is_infinite()))
}
