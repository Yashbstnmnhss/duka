use duka_gc::{GcCell, Heap};
use duka_macros::duka_builtin;
use duka_shared::value::DukaInt;

use crate::{
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
};

define_builtins! {
    fn:
        meta:
            "find" => impl_find, __DUKA_IMPL_FIND_META,
            "reverse" => impl_reverse, __DUKA_IMPL_REVERSE_META,
            "lower" => impl_lower, __DUKA_IMPL_LOWER_META,
            "upper" => impl_upper, __DUKA_IMPL_UPPER_META,
            "repeat" => impl_repeat, __DUKA_IMPL_REPEAT_META,
            "trim" => impl_trim, __DUKA_IMPL_TRIM_META,
            "trim_start" => impl_trim_start, __DUKA_IMPL_TRIM_START_META,
            "trim_end" => impl_trim_end, __DUKA_IMPL_TRIM_END_META,
            "len" => impl_len, __DUKA_IMPL_LEN_META,
            "substr" => impl_substr, __DUKA_IMPL_SUBSTR_META,
            "slice" => impl_slice, __DUKA_IMPL_SLICE_META,
            "split" => impl_split, __DUKA_IMPL_SPLIT_META;
    const:
}

/// 规范化索引:非负 clamp 到 len;负值按尾部回绕(小于 len 的负数即 `len+i`)
///
/// See docs/stdlib.md
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

#[duka_builtin(
    module = "string",
    name = "substr",
    doc = "Returns a portion of this string, starting at the specified index and extending for a given number of characters afterwards",
    params(s: bytes, start: int, count: int = -1),
    returns(string)
)]
fn impl_substr(
    h: &mut Heap,
    s: Vec<u8>,
    start: DukaInt,
    count: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let len = s.len();
    let start = normalize(start, len);
    let count = if count < 0 {
        len.saturating_sub(start)
    } else {
        count as usize
    };
    let end = (start + count).min(len);
    let out = s[start..end].to_vec();
    Ok(make_string(h, out))
}

#[duka_builtin(
    module = "string",
    name = "slice",
    doc = "Extracts a section [start, end) of this string and returns it as a new string, without modifying the original string",
    params(s: bytes, start: int, end: int = s.len() as DukaInt),
    returns(string)
)]
fn impl_slice(
    h: &mut Heap,
    s: Vec<u8>,
    start: DukaInt,
    end: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let len = s.len();
    let a = normalize(start, len);
    let b = normalize(end, len);
    let out = if a >= b { vec![] } else { s[a..b].to_vec() };
    Ok(make_string(h, out))
}

#[duka_builtin(
    module = "string",
    name = "split",
    doc = "Splits string s by sep",
    params(s: bytes, sep: bytes = vec![b' ']),
    returns(table)
)]
fn impl_split(h: &mut Heap, s: Vec<u8>, sep: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    if sep.is_empty() {
        return Err(DukaRuntimeError::Custom(
            "string.split: empty separator".into(),
        ));
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
    Ok(val)
}

#[duka_builtin(
    module = "string",
    name = "len",
    doc = "Get length of string, same as #",
    params(s: bytes),
    returns(string)
)]
fn impl_len(s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(s.len() as DukaInt))
}

#[duka_builtin(
    module = "string",
    name = "upper",
    doc = "Return a string with all ASCII characters in uppercase",
    params(s: bytes),
    returns(string)
)]
fn impl_upper(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let out: Vec<u8> = s.into_iter().map(|b| b.to_ascii_uppercase()).collect();
    Ok(make_string(h, out))
}

#[duka_builtin(
    module = "string",
    name = "upper",
    doc = "Return a string with all ASCII characters in lowercase",
    params(s: bytes),
    returns(string)
)]
fn impl_lower(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let out: Vec<u8> = s.into_iter().map(|b| b.to_ascii_lowercase()).collect();
    Ok(make_string(h, out))
}
#[duka_builtin(
    module = "string",
    name = "trim_start",
    doc = "Removes whitespace from start of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim_start(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = s.iter().position(|b| !is_ws(*b)).unwrap_or(s.len());
    let out = s[start..].to_vec();
    Ok(make_string(h, out))
}
#[duka_builtin(
    module = "string",
    name = "trim_end",
    doc = "Removes whitespace from end of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim_end(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let end = s.iter().rposition(|b| !is_ws(*b)).map_or(0, |i| i + 1);
    let out = s[..end].to_vec();
    Ok(make_string(h, out))
}
#[duka_builtin(
    module = "string",
    name = "trim",
    doc = "Removes whitespace from both ends of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = s.iter().position(|b| !is_ws(*b)).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !is_ws(*b)).map_or(start, |i| i + 1);
    let out = s[start..end].to_vec();
    Ok(make_string(h, out))
}

#[duka_builtin(
    module = "string",
    name = "find",
    doc = "Finds a substring in string (from given start index), returns its start index or nil when not found",
    params(s: bytes, sub: bytes, from: int = 0),
    returns(int | nil)
)]
fn impl_find(s: Vec<u8>, sub: Vec<u8>, from: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
    let len = s.len();
    let start = normalize(from, len);
    if sub.is_empty() {
        return Ok(RuntimeValue::Int(start as DukaInt));
    }
    let pos = if start + sub.len() <= len {
        s[start..]
            .windows(sub.len())
            .position(|w| w == sub)
            .map(|p| start + p)
    } else {
        None
    };
    Ok(match pos {
        Some(p) => RuntimeValue::Int(p as DukaInt),
        None => RuntimeValue::Nil,
    })
}

#[duka_builtin(
    module = "string",
    name = "reverse",
    doc = "Reverses string",
    params(s: bytes),
    returns(string)
)]
fn impl_reverse(h: &mut Heap, s: Vec<u8>) -> Result<RuntimeValue, DukaRuntimeError> {
    let out: Vec<u8> = s.into_iter().rev().collect();
    Ok(make_string(h, out))
}

#[duka_builtin(
    module = "string", name = "repeat",
    doc = "Repeat s n times, separated by sep",
    params(s: bytes, n: int, sep: bytes = Vec::new()),
    returns(string),
)]
fn impl_repeat(
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
