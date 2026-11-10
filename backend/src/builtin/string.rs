use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};
use duka_shared::value::DukaInt;

use crate::{
    builtin::normalize,
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::{CoState, NativeApi},
};

duka_builtin_def! {
    mod string
    fn {
        meta:
            impl_find,
            impl_reverse,
            impl_lower,
            impl_upper,
            impl_repeat,
            impl_trim,
            impl_trim_start,
            impl_trim_end,
            impl_len,
            impl_substr,
            impl_slice,
            impl_split,
            impl_concat co,
            impl_builder
    }
    userdata {
        meta: StringBuilder
    }
}

fn make_string(heap: &mut Heap, bytes: Vec<u8>) -> RuntimeValue {
    RuntimeValue::from_string(heap, String::from_utf8_lossy(&bytes).into_owned())
}

#[duka_builtin(
    name = "substr",
    doc = "Returns a portion of this string, starting at the specified index and extending for a given number of characters afterwards",
    params(s: string, start: int, count: int = -1),
    returns(string)
)]
fn impl_substr(
    h: &mut Heap,
    s: String,
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
    Ok(make_string(h, s[start..end].as_bytes().to_vec()))
}

#[duka_builtin(
    name = "slice",
    doc = "Extracts a section [start, end) of this string and returns it as a new string, without modifying the original string",
    params(s: bytes, start: int, end: int = s.eval_to_string().len() as DukaInt, @default = "#s"),
    returns(string)
)]
fn impl_slice(
    h: &mut Heap,
    s: RuntimeValue,
    start: DukaInt,
    end: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = s.eval_to_string();
    let len = s.len();
    let a = normalize(start, len);
    let b = normalize(end, len);
    let out = if a >= b {
        vec![]
    } else {
        s[a..b].as_bytes().to_vec()
    };
    Ok(make_string(h, out))
}

#[duka_builtin(
    name = "split",
    doc = "Splits string s by sep",
    params(s: bytes, sep: string = " ".to_owned(), @default = "\" \""),
    returns(array<string>)
)]
fn impl_split(
    h: &mut Heap,
    s: RuntimeValue,
    sep: String,
) -> Result<RuntimeValue, DukaRuntimeError> {
    if sep.is_empty() {
        return Err(DukaRuntimeError::Custom(
            "string.split: empty separator".into(),
        ));
    }
    let str = s.eval_to_string();
    let parts: Vec<Vec<u8>> = str.split(&sep).map(|p| p.as_bytes().to_vec()).collect();

    let mut table = RuntimeDukaTable::new(parts.len());
    for (idx, part) in parts.iter().enumerate() {
        table.array_set(idx, make_string(h, part.clone()));
    }
    let val = RuntimeValue::Table(h.alloc(GcCell::new(table)));
    Ok(val)
}

#[duka_builtin(
    name = "len",
    doc = "Get length of string based on characters instead of bytes",
    params(s: string),
    returns(int)
)]
fn impl_len(s: String) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(s.chars().count() as DukaInt))
}

#[duka_builtin(
    name = "upper",
    doc = "Return a string with all ASCII characters in uppercase",
    params(s: string),
    returns(string)
)]
fn impl_upper(h: &mut Heap, s: String) -> Result<RuntimeValue, DukaRuntimeError> {
    let out = s
        .as_bytes()
        .iter()
        .map(|b| b.to_ascii_uppercase())
        .collect();
    Ok(make_string(h, out))
}

#[duka_builtin(
    name = "lower",
    doc = "Return a string with all ASCII characters in lowercase",
    params(s: string),
    returns(string)
)]
fn impl_lower(h: &mut Heap, s: String) -> Result<RuntimeValue, DukaRuntimeError> {
    let out: Vec<u8> = s
        .as_bytes()
        .iter()
        .map(|b| b.to_ascii_lowercase())
        .collect();
    Ok(make_string(h, out))
}
#[duka_builtin(
    name = "trim_start",
    doc = "Removes whitespace from start of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim_start(h: &mut Heap, s: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = s.eval_to_string();
    let start = s
        .chars()
        .into_iter()
        .position(|b| !b.is_whitespace())
        .unwrap_or(s.len());
    let out = s[start..].as_bytes().to_vec();
    Ok(make_string(h, out))
}
#[duka_builtin(
    name = "trim_end",
    doc = "Removes whitespace from end of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim_end(h: &mut Heap, s: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = s.eval_to_string();
    let end = s
        .as_bytes()
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    let out = s[..end].as_bytes().to_vec();
    Ok(make_string(h, out))
}
#[duka_builtin(
    name = "trim",
    doc = "Removes whitespace from both ends of this string and returns a new string, without modifying the original string",
    params(s: bytes),
    returns(string)
)]
fn impl_trim(h: &mut Heap, s: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = s.eval_to_string();
    let start = s
        .as_bytes()
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let end = s
        .as_bytes()
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    let out = s[start..end].as_bytes().to_vec();
    Ok(make_string(h, out))
}

#[duka_builtin(
    name = "find",
    doc = "Finds a substring in string (from given start index), returns its start index or nil when not found",
    params(s: bytes, sub: bytes, from: int = 0),
    returns(int | nil)
)]
fn impl_find(
    s: RuntimeValue,
    sub: RuntimeValue,
    from: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let s = s.eval_to_string();
    let len = s.len();
    let start = normalize(from, len);
    let sub = sub.eval_to_string();
    if sub.is_empty() {
        return Ok(RuntimeValue::Int(start as DukaInt));
    }
    let pos = if start + sub.len() <= len {
        s[start..]
            .as_bytes()
            .windows(sub.len())
            .position(|w| w == sub.as_bytes())
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
    name = "reverse",
    doc = "Reverses string",
    params(s: bytes),
    returns(string)
)]
fn impl_reverse(h: &mut Heap, s: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let out: Vec<u8> = s.eval_to_string().bytes().rev().collect();
    Ok(make_string(h, out))
}

#[duka_builtin(
    name = "repeat",
    doc = "Repeat s n times, separated by sep",
    params(s: bytes, n: int, sep: string = String::new(), @default = "\"\""),
    returns(string),
)]
fn impl_repeat(
    h: &mut Heap,
    s: RuntimeValue,
    n: DukaInt,
    sep: String,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let mut out = vec![];
    let str = s.eval_to_string();
    for i in 0..n.max(0) {
        if i > 0 {
            out.extend_from_slice(sep.as_bytes());
        }
        out.extend_from_slice(str.as_bytes());
    }
    Ok(RuntimeValue::from_string(
        h,
        String::from_utf8_lossy(&out).into_owned(),
    ))
}

#[duka_builtin(
    name = "concat",
    doc = "Concat all arguments into one string",
    params(vals: vararg),
    returns(string),
)]
fn impl_concat(
    sv: &mut CoState,
    h: &mut Heap,
    api: &mut NativeApi,
    vals: Vec<RuntimeValue>,
) -> Result<RuntimeValue, DukaRuntimeError> {
    use super::get_string;
    let v = vals
        .into_iter()
        .map(|val| get_string(sv, h, api, val))
        .collect::<Result<Vec<_>, _>>()?
        .join("");
    Ok(RuntimeValue::from_string(h, v))
}

#[duka_builtin(
    name = "builder",
    doc = "Create a builder",
    params(),
    returns(userdata)
)]
fn impl_builder(h: &mut Heap) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(StringBuilder::new(String::new()).into_value(h))
}

duka_user_data! {
    #[duka_builtin(
        name = "Builder",
        doc = "String builder",
        example = r#"local b = string.builder()
for i = 1, 1000 do
    b:append(i, ",")
end
local s = b:build()  -- "1,2,3,...,1000,""#
    )]
    struct StringBuilder {
        buf: String
    }

    constructor fn new(init: String) -> Self {
        Self { buf: init }
    }

    metamethod {
        #[duka_builtin(
            doc = "Returns the current character length of the builder.",
            params(self: userdata),
            returns(int)
        )]
        fn len(&self) -> Result<RuntimeValue, DukaRuntimeError> {
            Ok(RuntimeValue::Int(self.buf.chars().count() as DukaInt))
        },
        #[duka_builtin(
            doc = "**Clone** a result from this builder.",
            params(self: userdata),
            returns(int)
        )]
        fn tostring(&self, h: &mut Heap) -> Result<RuntimeValue, DukaRuntimeError> {
            let s = self.buf.clone();
            Ok(RuntimeValue::from_string(h, s))
        },
    }

    chain #[duka_builtin(
        doc = "Appends each value to the builder. Returns self for chaining.",
        params(self: userdata, vals: vararg),
        returns(userdata)
    )]
    fn append(&mut self, vals: Vec<RuntimeValue>) -> Result<(), DukaRuntimeError> {
        for v in &vals {
            self.buf.push_str(&v.eval_to_string());
        }
        Ok(())
    },
    chain #[duka_builtin(
        doc = "Alias of `append`. Appends each value to the builder. Returns self for chaining.",
        params(self: userdata, vals: vararg),
        returns(userdata)
    )]
    fn write(&mut self, vals: Vec<RuntimeValue>) -> Result<(), DukaRuntimeError> {
        for v in &vals {
            self.buf.push_str(&v.eval_to_string());
        }
        Ok(())
    },
    #[duka_builtin(
        doc = "Returns the built string and CLEARS the builder. Zero-copy (takes the underlying buffer).",
        params(self: userdata),
        returns(string)
    )]
    fn build(&mut self, h: &mut Heap) -> Result<RuntimeValue, DukaRuntimeError> {
        let s = std::mem::take(&mut self.buf);
        Ok(RuntimeValue::from_string(h, s))
    },
    #[duka_builtin(
        doc = "Returns a copy of the current content WITHOUT clearing the builder.",
        params(self: userdata),
        returns(string)
    )]
    fn to_string(&self, h: &mut Heap) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::from_string(h, self.buf.clone()))
    },
    #[duka_builtin(
        doc = "Returns true if the builder contains no bytes.",
        params(self: userdata),
        returns(bool)
    )]
    fn is_empty(&self) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Bool(self.buf.is_empty()))
    },
    #[duka_builtin(
        doc = "Clears the builder's content, keeping its allocated capacity.",
        params(self: userdata)
    )]
    fn clear(&mut self) -> Result<(), DukaRuntimeError> {
        self.buf.clear();
        Ok(())
    },
    chain #[duka_builtin(
        doc = "Reserves capacity for at least `n` more bytes to avoid reallocations.",
        params(self: userdata, n: int),
        returns(userdata)
    )]
    fn reserve(&mut self, n: DukaInt) -> Result<(), DukaRuntimeError> {
        if n > 0 {
            self.buf.reserve(n as usize);
        }
        Ok(())
    }
}
