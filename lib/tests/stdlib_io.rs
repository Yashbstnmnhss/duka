//! Stdlib.io

use std::sync::atomic::{AtomicU64, Ordering};

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_last, run_with_input};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempFile {
    path: String,
}

impl TempFile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "duka_io_test_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = path.to_string_lossy().replace('\\', "/");
        TempFile { path }
    }

    fn script(&self, body: &str) -> String {
        format!("local path = \"{}\"\n{body}", self.path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn str(v: &RuntimeValue) -> String {
    v.eval_to_string().into_owned()
}

#[test]
fn write_then_read_all() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok, "open failed: " .. f)
f:write("hello world")
f:close()
local ok2, f2 = io.open(path, "r")
assert(ok2, "reopen failed: " .. f2)
local ok3, data = f2:read("*a")
f2:close()
return data
"#,
    );
    assert_eq!(str(&run_last(&src).unwrap()), "hello world");
}

#[test]
fn write_returns_byte_count() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
local ok2, n = f:write("hello")
f:close()
return n
"#,
    );
    assert_eq!(run_last(&src).unwrap(), RuntimeValue::Int(5));
}

#[test]
fn write_nil_formats_as_string() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
local ok2, n = f:write(nil)
f:close()
local ok3, f2 = io.open(path, "r")
assert(ok3)
local ok4, data = f2:read("*a")
f2:close()
return ok2, n, data
"#,
    );
    let res = run(&src).unwrap();
    let last3 = &res[res.len() - 3..];
    assert_eq!(last3[0], RuntimeValue::Bool(true));
    assert_eq!(last3[1], RuntimeValue::Int(3));
    assert_eq!(str(&last3[2]), "nil");
}

#[test]
fn read_line_default_strips_newline() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
f:write("first\nsecond")
f:close()
local ok2, f2 = io.open(path, "r")
assert(ok2)
local ok1, l1 = f2:read()
local ok2b, l2 = f2:read()
local ok3, l3 = f2:read()
f2:close()
return l1, l2, ok3, l3
"#,
    );
    let res = run(&src).unwrap();
    let last4 = &res[res.len() - 4..];
    assert_eq!(str(&last4[0]), "first");
    assert_eq!(str(&last4[1]), "second");
    assert_eq!(last4[2], RuntimeValue::Bool(true));
    assert_eq!(last4[3], RuntimeValue::Nil);
}

#[test]
fn read_n_bytes() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w+")
assert(ok)
f:write("hello world")
f:seek("set", 0)
local ok3, five = f:read(5)
local ok4, rest = f:read(6)
f:close()
return five, rest
"#,
    );
    let res = run(&src).unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(str(&last2[0]), "hello");
    assert_eq!(str(&last2[1]), " world");
}

#[test]
fn seek_positions() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w+")
assert(ok)
f:write("abcdef")
local ok2, at_end = f:seek("end", 0)
f:seek("set", 0)
local ok3, abc = f:read(3)
local ok4, pos = f:seek("cur", 1)
local ok5, ef = f:read(2)
f:close()
return at_end, pos, abc, ef
"#,
    );
    let res = run(&src).unwrap();
    let last4 = &res[res.len() - 4..];
    assert_eq!(last4[0], RuntimeValue::Int(6), "seek end -> 6");
    assert_eq!(last4[1], RuntimeValue::Int(4), "seek cur 3+1 -> 4");
    assert_eq!(str(&last4[2]), "abc");
    assert_eq!(str(&last4[3]), "ef");
}

#[test]
fn lines_iterator() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
f:write("one\ntwo\nthree\n")
f:close()
local ok2, f2 = io.open(path, "r")
assert(ok2)
local total = 0
local longest = ""
for line in f2:lines() do
    total = total + 1
    if #line > #longest then longest = line end
end
f2:close()
return total, longest
"#,
    );
    let res = run(&src).unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(last2[0], RuntimeValue::Int(3));
    assert_eq!(str(&last2[1]), "three");
}

#[test]
fn read_number() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w+")
assert(ok)
f:write("42 3.5")
f:seek("set", 0)
local ok3, a = f:read("n")
local ok4, b = f:read("n")
f:close()
return a, b
"#,
    );
    let res = run(&src).unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(last2[0], RuntimeValue::Int(42));
    assert_eq!(last2[1], RuntimeValue::Float(3.5));
}

#[test]
fn tmpfile_is_writable() {
    let src = r#"
local ok, f = io.tmpfile()
assert(ok, "tmpfile failed: " .. f)
f:write("tmp data")
f:seek("set", 0)
local ok2, data = f:read("*a")
f:close()
return data
"#;
    assert_eq!(str(&run_last(src).unwrap()), "tmp data");
}

#[test]
fn io_type_and_is_open() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
local t_open = io.type(f)
local open_flag = f:is_open()
f:close()
local t_closed = io.type(f)
local open_after = f:is_open()
return t_open, open_flag, t_closed, open_after
"#,
    );
    let res = run(&src).unwrap();
    let last4 = &res[res.len() - 4..];
    assert_eq!(str(&last4[0]), "file");
    assert_eq!(last4[1], RuntimeValue::Bool(true));
    assert_eq!(str(&last4[2]), "closed file");
    assert_eq!(last4[3], RuntimeValue::Bool(false));
}

#[test]
fn open_missing_file_returns_error() {
    let src = r#"
local ok, msg = io.open("/no/such/duka_file.txt", "r")
return ok, msg
"#;
    let res = run(&src).unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(last2[0], RuntimeValue::Bool(false));
    assert!(last2[1].is_string());
}

#[test]
fn write_to_closed_file_returns_error() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
f:close()
local _, msg = f:write("nope")
return msg
"#,
    );
    let res = run_last(&src).unwrap();
    assert!(res.is_string());
    assert!(res.to_string().contains("closed"), "{res}");
}

#[test]
fn read_all_empty_file_returns_empty_string() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
f:close()
local ok2, f2 = io.open(path, "r")
assert(ok2)
local ok3, data = f2:read("*a")
f2:close()
return data
"#,
    );
    assert_eq!(str(&run_last(&src).unwrap()), "");
}

#[test]
fn lines_on_file_without_trailing_newline() {
    let t = TempFile::new();
    let src = t.script(
        r#"
local ok, f = io.open(path, "w")
assert(ok)
f:write("one\ntwo")
f:close()
local ok2, f2 = io.open(path, "r")
assert(ok2)
local lines = {}
local n = 0
for line in f2:lines() do
    n = n + 1
    lines[n] = line
end
f2:close()
return n, lines[1], lines[2]
"#,
    );
    let res = run(&src).unwrap();
    let last3 = &res[res.len() - 3..];
    assert_eq!(last3[0], RuntimeValue::Int(2));
    assert_eq!(str(&last3[1]), "one");
    assert_eq!(str(&last3[2]), "two");
}

#[test]
fn stdin_read_line() {
    let src = r#"
local ok, line = io.stdin:read()
assert(ok, "read failed: " .. to_string(line))
return line
"#;
    let res = run_with_input(src, b"hello world\n").unwrap();
    assert_eq!(str(res.last().unwrap()), "hello world");
}

#[test]
fn stdin_read_all() {
    let src = r#"
local ok, data = io.stdin:read("*a")
assert(ok)
return data
"#;
    let res = run_with_input(src, b"line1\nline2\n").unwrap();
    assert_eq!(str(res.last().unwrap()), "line1\nline2\n");
}

#[test]
fn stdin_read_number() {
    let src = r#"
local ok, n = io.stdin:read("*n")
assert(ok)
return n
"#;
    let res = run_with_input(src, b"42 rest\n").unwrap();
    assert_eq!(res.last().unwrap(), &RuntimeValue::Int(42));
}

#[test]
fn stdin_read_n_bytes() {
    let src = r#"
local ok, data = io.stdin:read(3)
assert(ok)
return data
"#;
    let res = run_with_input(src, b"abcdef").unwrap();
    assert_eq!(str(res.last().unwrap()), "abc");
}

#[test]
fn stdin_eof_is_nil() {
    let src = r#"
local ok, line = io.stdin:read()
return ok, line
"#;
    let res = run_with_input(src, b"").unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(last2[0], RuntimeValue::Bool(true));
    assert_eq!(last2[1], RuntimeValue::Nil);
}

#[test]
fn stdin_lines_iter() {
    let src = r#"
local lines = {}
local n = 0
for line in io.stdin:lines() do
    n = n + 1
    lines[n] = line
end
return n, lines[1], lines[2]
"#;
    let res = run_with_input(src, b"one\ntwo\nthree").unwrap();
    let last3 = &res[res.len() - 3..];
    assert_eq!(last3[0], RuntimeValue::Int(3));
    assert_eq!(str(&last3[1]), "one");
    assert_eq!(str(&last3[2]), "two");
}

#[test]
fn stdin_reads_consume_shared_buffer() {
    let src = r#"
local ok1, line1 = io.stdin:read()
local ok2, line2 = io.stdin:read()
assert(ok1)
assert(ok2)
return line1, line2
"#;
    let res = run_with_input(src, b"first\nsecond\n").unwrap();
    let last2 = &res[res.len() - 2..];
    assert_eq!(str(&last2[0]), "first");
    assert_eq!(str(&last2[1]), "second");
}
