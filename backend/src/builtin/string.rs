use duka_gc::{GcCell, Heap};
use duka_macros::duka_builtin;
use duka_shared::{constants::ctype, types::ValueCount, value::DukaInt};

use crate::{
    builtin::{ensure_type, required},
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::CoState,
};

define_builtins! {
    fn:
        plain:
            "substr" => impl_substr,
            "slice" => impl_slice,
            "split" => impl_split,
            "len" => impl_len,
            "upper" => impl_upper,
            "lower" => impl_lower,
            "trim" => impl_trim,
            "repeat" => impl_repeat,
            "find" => impl_find,
            "reverse" => impl_reverse;
        meta:
            "repeatn" => impl_repeatn, __DUKA_IMPL_REPEATN_META;
    const:
}

/// 取第 idx 个参数为字符串,返回其字节
fn get_str(sv: &mut CoState, idx: usize, func: &str) -> Result<Vec<u8>, DukaRuntimeError> {
    let val = required(sv, idx, func, "string")?.clone();
    ensure_type(&val, ctype::STR, func, idx)?;
    Ok(val.eval_to_string().as_bytes().to_vec())
}

/// 取第 idx 个参数为整数(Int/Float/Bool 均可,其余类型报错)
fn get_int(sv: &mut CoState, idx: usize, func: &str) -> Result<DukaInt, DukaRuntimeError> {
    let val = required(sv, idx, func, "number")?.clone();
    val.eval_to_int().ok_or_else(|| {
        DukaRuntimeError::ArgumentInvalidType(idx, func.into(), "number", val.type_name_of())
    })
}

/// 规范化索引:非负 clamp 到 len;负值按尾部回绕(小于 len 的负数即 `len+i`)
fn normalize(i: DukaInt, len: usize) -> usize {
    if i >= 0 {
        (i as usize).min(len)
    } else {
        len.saturating_sub(i.unsigned_abs() as usize)
    }
}

fn make_string(heap: &mut Heap, bytes: Vec<u8>) -> RuntimeValue {
    RuntimeValue::from_string(heap, String::from_utf8_lossy(&bytes).into_owned())
}

fn impl_substr(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "substr")?;
    let start = get_int(sv, 1, "substr")?;
    let count = if sv.ensure_address(3) {
        get_int(sv, 2, "substr")?
    } else {
        -1
    };

    let len = s.len();
    let start = normalize(start, len);
    let count = if count < 0 {
        len.saturating_sub(start)
    } else {
        count as usize
    };
    let end = (start + count).min(len);
    let out = s[start..end].to_vec();
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_slice(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "slice")?;
    let start = get_int(sv, 1, "slice")?;
    let end = if sv.ensure_address(3) {
        get_int(sv, 2, "slice")?
    } else {
        s.len() as DukaInt
    };

    let len = s.len();
    let a = normalize(start, len);
    let b = normalize(end, len);
    let out = if a >= b { vec![] } else { s[a..b].to_vec() };
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_split(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "split")?;
    let sep = get_str(sv, 1, "split")?;
    if sep.is_empty() {
        return Err(DukaRuntimeError::Custom("split: empty separator".into()));
    }

    let mut parts: Vec<Vec<u8>> = vec![];
    let mut cur: Vec<u8> = vec![];
    let mut i = 0;
    let sl = sep.len();
    let n = s.len();
    while i < n {
        if i + sl <= n && s[i..i + sl] == sep[..] {
            parts.push(std::mem::take(&mut cur));
            i += sl;
        } else {
            cur.push(s[i]);
            i += 1;
        }
    }
    parts.push(cur);

    let mut table = RuntimeDukaTable::new(parts.len());
    for (idx, part) in parts.iter().enumerate() {
        table.array_set(idx, make_string(h, part.clone()));
    }
    let val = RuntimeValue::Table(h.alloc(GcCell::new(table)));
    sv.set_stack(0, val)?;
    Ok(ValueCount::Exact(1))
}

fn impl_len(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "len")?;
    sv.set_stack(0, RuntimeValue::Int(s.len() as DukaInt))?;
    Ok(ValueCount::Exact(1))
}

fn impl_upper(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "upper")?;
    let out: Vec<u8> = s.into_iter().map(|b| b.to_ascii_uppercase()).collect();
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_lower(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "lower")?;
    let out: Vec<u8> = s.into_iter().map(|b| b.to_ascii_lowercase()).collect();
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_trim(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "trim")?;
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = s.iter().position(|b| !is_ws(*b)).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !is_ws(*b)).map_or(start, |i| i + 1);
    let out = s[start..end].to_vec();
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_repeat(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "repeat")?;
    let n = get_int(sv, 1, "repeat")?;
    if n <= 0 {
        sv.set_stack(0, make_string(h, vec![]))?;
        return Ok(ValueCount::Exact(1));
    }
    let out = s.repeat(n as usize);
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

fn impl_find(sv: &mut CoState, _h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "find")?;
    let sub = get_str(sv, 1, "find")?;
    let from = if sv.ensure_address(3) {
        get_int(sv, 2, "find")?
    } else {
        0
    };

    let len = s.len();
    let start = normalize(from, len);
    if sub.is_empty() {
        sv.set_stack(0, RuntimeValue::Int(start as DukaInt))?;
        return Ok(ValueCount::Exact(1));
    }
    let pos = if start + sub.len() <= len {
        s[start..]
            .windows(sub.len())
            .position(|w| w == sub)
            .map(|p| start + p)
    } else {
        None
    };
    match pos {
        Some(p) => sv.set_stack(0, RuntimeValue::Int(p as DukaInt))?,
        None => sv.set_stack(0, RuntimeValue::Nil)?,
    }
    Ok(ValueCount::Exact(1))
}

fn impl_reverse(sv: &mut CoState, h: &mut Heap) -> Result<ValueCount, DukaRuntimeError> {
    let s = get_str(sv, 0, "reverse")?;
    let out: Vec<u8> = s.into_iter().rev().collect();
    sv.set_stack(0, make_string(h, out))?;
    Ok(ValueCount::Exact(1))
}

#[duka_builtin(
    module = "string", name = "repeatn",
    doc = "Repeat s n times, separated by sep",
    params(s: bytes, n: int, sep: bytes = Vec::new()),
    returns(string),
)]
fn impl_repeatn(
    h: &mut Heap,
    s: Vec<u8>,
    n: DukaInt,
    sep: Vec<u8>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut out = Vec::new();
    for i in 0..n.max(0) {
        if i > 0 {
            out.extend_from_slice(&sep);
        }
        out.extend_from_slice(&s);
    }
    Ok(RuntimeValue::from_string(
        h,
        String::from_utf8_lossy(&out).into_owned(),
    ))
}
