use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use duka_gc::Heap;
use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};
use duka_shared::types::ValueCount;
use duka_shared::value::DukaInt;

use crate::builtin::arg::{err, ok};
use crate::builtin::format_arg;
use crate::errors::DukaRuntimeError;
use crate::value::{RuntimeValue, RustClosure};
use crate::vm::coroutine::{CoState, InputCell, NativeApi};

duka_builtin_def! {
    mod io
    doc "File and stream I/O"
    example r#"local ok, f = io.open("a.txt", "w")
if ok then 
    f:write("hello") 
end"#
    flags(@feature(platform))
    fn {
        meta:
            impl_open,
            impl_tmpfile,
            impl_type
    }
    const {}
    init {
        stdout: IOOut::new(false).into_value(heap) meta __DUKA_IOOUT_META doc("Standard stream for output"),
        stderr: IOOut::new(true).into_value(heap) meta __DUKA_IOOUT_META doc("Standard stream for error output"),
        stdin: IOIn::new().into_value(heap) meta __DUKA_IOIN_META doc("Standard stream for input")
    }
}

fn cerr(h: &mut Heap, msg: impl Into<String>) -> Vec<RuntimeValue> {
    err(h, DukaRuntimeError::IOError(msg.into()))
}

fn open_file(path: &str, mode: &str) -> std::io::Result<File> {
    let (read, write, append, truncate, create) = match mode {
        "r" | "rb" => (true, false, false, false, false),
        "r+" | "r+b" => (true, true, false, false, false),
        "w" | "wb" => (false, true, false, true, true),
        "w+" | "w+b" => (true, true, false, true, true),
        "a" | "ab" => (false, true, true, false, true),
        "a+" | "a+b" => (true, true, true, false, true),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid mode: {mode}"),
            ));
        }
    };
    let mut o = OpenOptions::new();
    o.read(read)
        .write(write)
        .append(append)
        .truncate(truncate)
        .create(create);
    o.open(path)
}

fn open_temp_file() -> std::io::Result<File> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut path = std::env::temp_dir().join(format!(
        "duka_tmp_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => return Ok(f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                path = std::env::temp_dir().join(format!(
                    "duka_tmp_{}_{}",
                    std::process::id(),
                    COUNTER.fetch_add(1, Ordering::Relaxed)
                ));
            }
            Err(e) => return Err(e),
        }
    }
}

fn read_until_newline(f: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match f.read(&mut byte) {
            Ok(0) => return Ok(buf),
            Ok(_) => {
                buf.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(buf);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn strip_trailing_newline(buf: &mut Vec<u8>) {
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
}

fn read_line_impl(f: &mut impl Read, h: &mut Heap, keep_newline: bool) -> Vec<RuntimeValue> {
    match read_until_newline(f) {
        Ok(mut buf) => {
            if buf.is_empty() {
                return ok(RuntimeValue::Nil);
            }
            if !keep_newline {
                strip_trailing_newline(&mut buf);
            }
            ok(RuntimeValue::from_string(
                h,
                String::from_utf8_lossy(&buf).into_owned(),
            ))
        }
        Err(e) => err(h, e),
    }
}

fn read_all_impl(f: &mut impl Read, h: &mut Heap) -> Vec<RuntimeValue> {
    let mut buf = Vec::new();
    match f.read_to_end(&mut buf) {
        Ok(_) => ok(RuntimeValue::from_string(
            h,
            String::from_utf8_lossy(&buf).into_owned(),
        )),
        Err(e) => err(h, e),
    }
}

fn read_n_impl(f: &mut impl Read, h: &mut Heap, n: DukaInt) -> Vec<RuntimeValue> {
    if n < 0 {
        return cerr(h, "negative read count");
    }
    let mut buf = vec![0u8; n as usize];
    let mut read = 0usize;
    while read < buf.len() {
        match f.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(k) => read += k,
            Err(e) => return err(h, e),
        }
    }
    buf.truncate(read);
    if read == 0 {
        ok(RuntimeValue::Nil)
    } else {
        ok(RuntimeValue::from_string(
            h,
            String::from_utf8_lossy(&buf).into_owned(),
        ))
    }
}

fn read_number_impl(f: &mut impl Read, h: &mut Heap) -> Vec<RuntimeValue> {
    let mut token = String::new();
    let mut byte = [0u8; 1];
    loop {
        match f.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0].is_ascii_whitespace() {
                    if token.is_empty() {
                        continue;
                    }
                    break;
                }
                token.push(byte[0] as char);
            }
            Err(e) => return err(h, e),
        }
    }
    if token.is_empty() {
        return ok(RuntimeValue::Nil);
    }
    match token.parse::<f64>() {
        Ok(x) if x.fract() == 0.0 => ok(RuntimeValue::Int(x as i64)),
        Ok(x) => ok(RuntimeValue::Float(x)),
        Err(_) => ok(RuntimeValue::Nil),
    }
}

struct CellReader {
    cell: InputCell,
}
impl Read for CellReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut data = self.cell.lock().unwrap();
        if data.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(data.len());
        buf[..n].copy_from_slice(&data[..n]);
        data.drain(..n);
        Ok(n)
    }
}

enum InputSource {
    Cell(CellReader),
    Stdin(std::io::Stdin),
}
impl Read for InputSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            InputSource::Cell(c) => c.read(buf),
            InputSource::Stdin(s) => s.read(buf),
        }
    }
}

fn input_reader(co: &NativeApi) -> InputSource {
    match co.input() {
        Some(cell) => InputSource::Cell(CellReader { cell }),
        None => InputSource::Stdin(std::io::stdin()),
    }
}

#[duka_builtin(
    flags(@returns(result)),
    doc = "Opens the file `path` with `mode` (\"r\", \"w\", \"a\", \"r+\", \"w+\", \"a+\", optionally with a \"b\" suffix)",
    params(path: string, mode: string = "r".to_owned()),
    returns(vararg)
)]
fn impl_open(
    h: &mut Heap,
    path: String,
    mode: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    match open_file(&path, &mode) {
        Ok(file) => Ok(ok(FileData::new(file).into_value(h))),
        Err(e) => Ok(err(h, e)),
    }
}

#[duka_builtin(
    flags(@returns(result)),
    doc = "Creates and opens a unique temporary file for reading and writing",
    params(),
    returns(vararg)
)]
fn impl_tmpfile(h: &mut Heap) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    match open_temp_file() {
        Ok(file) => Ok(ok(FileData::new(file).into_value(h))),
        Err(e) => Ok(err(h, e)),
    }
}

#[duka_builtin(
    doc = "Returns \"file\" if `v` is an open file handle, \"closed file\" if it is closed, otherwise nil",
    params(v: any),
    returns(any)
)]
fn impl_type(h: &mut Heap, v: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
    let RuntimeValue::UserData(cell) = v else {
        return Ok(RuntimeValue::Nil);
    };
    let ud = cell.borrow();
    let any = ud.payload.as_ref() as &dyn std::any::Any;
    let Some(file) = any.downcast_ref::<FileData>() else {
        return Ok(RuntimeValue::Nil);
    };
    let name = if file.closed { "closed file" } else { "file" };
    Ok(RuntimeValue::from_string(h, name.to_owned()))
}

duka_user_data! {
    #[duka_builtin(name = "File", doc = "An open file handle", example = "local f = io.open(\"a.txt\")")]
    struct FileData {
        inner: Option<Arc<Mutex<File>>>,
        closed: bool
    }
    constructor fn new(file: File) -> Self {
        Self {
            inner: Some(Arc::new(Mutex::new(file))),
            closed: false
        }
    }
    #[duka_builtin(
        doc = "Closes the file",
        params(self: userdata),
        returns(vararg)
    )]
    fn close(&mut self, h: &mut Heap) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to close a closed file"));
        }
        self.closed = true;
        self.inner = None;
        Ok(ok(RuntimeValue::Nil))
    },
    #[duka_builtin(
        doc = "Returns whether the file handle is still open",
        params(self: userdata),
        returns(bool)
    )]
    fn is_open(&self) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Bool(!self.closed))
    },
    #[duka_builtin(
        doc = "Reads from the file. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: \"a\" reads all, \"l\"/\"L\" reads a line, \"n\" reads a number. Returns [true, data] on success, [true, nil] at end of file, [false, msg] on error",
        params(self: userdata, what: vararg),
        returns(vararg)
    )]
    fn read(&mut self, h: &mut Heap, what: Vec<RuntimeValue>) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to read a closed file"));
        }
        Ok(match what.first() {
            None => self.read_line(h, false),
            Some(RuntimeValue::Int(n)) => self.read_n(h, *n),
            Some(v) if v.is_string() => match v.eval_to_string().as_ref() {
                "a" | "*a" => self.read_all(h),
                "l" | "*l" => self.read_line(h, false),
                "L" | "*L" => self.read_line(h, true),
                "n" | "*n" => self.read_number(h),
                other => cerr(h, format!("invalid read format: {other}")),
            },
            Some(v) => cerr(h, format!("bad read argument of type {}", v.type_name_of())),
        })
    },
    #[duka_builtin(
        doc = "Writes each argument as a string to the file; nil is written as \"nil\". Returns [true, count] on success, [false, msg] on error",
        params(self: userdata, data: vararg),
        returns(vararg)
    )]
    fn write(&mut self, h: &mut Heap, data: Vec<RuntimeValue>) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to write a closed file"));
        }
        match self.borrow_file() {
            Ok(mut f) => {
                let mut total = 0usize;
                for v in &data {
                    let s = v.eval_to_string().into_owned();
                    match f.write_all(s.as_bytes()) {
                        Ok(()) => total += s.len(),
                        Err(e) => return Ok(err(h, e)),
                    }
                }
                Ok(ok(RuntimeValue::Int(total as DukaInt)))
            }
            Err(e) => Ok(err(h, e)),
        }
    },
    #[duka_builtin(
        doc = "Sets and gets the file position; `whence` is \"set\", \"cur\" or \"end\". Returns [true, pos] on success, [false, msg] on error",
        params(self: userdata, whence: string = "cur".to_owned(), offset: int = 0),
        returns(vararg)
    )]
    fn seek(&mut self, h: &mut Heap, whence: String, offset: DukaInt) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to seek a closed file"));
        }
        let from = match whence.as_str() {
            "set" => SeekFrom::Start(offset as u64),
            "cur" => SeekFrom::Current(offset),
            "end" => SeekFrom::End(offset),
            other => return Ok(cerr(h, format!("invalid seek whence: {other}"))),
        };
        match self.borrow_file() {
            Ok(mut f) => match f.seek(from) {
                Ok(pos) => Ok(ok(RuntimeValue::Int(pos as DukaInt))),
                Err(e) => Ok(err(h, e)),
            },
            Err(e) => Ok(err(h, e)),
        }
    },
    #[duka_builtin(
        doc = "Flushes any buffered data to the file",
        params(self: userdata),
        returns(vararg)
    )]
    fn flush(&mut self, h: &mut Heap) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to flush a closed file"));
        }
        match self.borrow_file() {
            Ok(mut f) => match f.flush() {
                Ok(()) => Ok(ok(RuntimeValue::Nil)),
                Err(e) => Ok(err(h, e)),
            },
            Err(e) => Ok(err(h, e)),
        }
    },
    #[duka_builtin(
        doc = "Returns an iterator that yields one line per iteration",
        params(self: userdata),
        returns(vararg)
    )]
    fn lines(&self, h: &mut Heap) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        if self.closed {
            return Ok(cerr(h, "attempt to iterate a closed file"));
        }
        let inner = self.inner.clone();
        let func = RustClosure::returns_with_captures(
            move |c, h, _api| {
                let Some(arc) = inner.as_ref() else {
                    c.set_stack(0, RuntimeValue::Bool(false))?;
                    return Ok(ValueCount::Exact(1));
                };
                let mut f = arc.lock().unwrap();
                match read_until_newline(&mut *f) {
                    Ok(mut buf) => {
                        if buf.is_empty() {
                            c.set_stack(0, RuntimeValue::Bool(false))?;
                            return Ok(ValueCount::Exact(1));
                        }
                        strip_trailing_newline(&mut buf);
                        c.set_stack(0, RuntimeValue::Bool(true))?;
                        c.set_stack(
                            1,
                            RuntimeValue::from_string(h, String::from_utf8_lossy(&buf).into_owned()),
                        )?;
                        Ok(ValueCount::Exact(2))
                    }
                    Err(_) => {
                        c.set_stack(0, RuntimeValue::Bool(false))?;
                        Ok(ValueCount::Exact(1))
                    }
                }
            },
            vec![],
            Some("__file_lines".into()),
        );
        Ok(vec![RuntimeValue::from_rust_closure(h, func)])
    },
}

impl FileData {
    fn borrow_file(&self) -> Result<MutexGuard<'_, File>, DukaRuntimeError> {
        self.inner
            .as_ref()
            .map(|i| i.lock().unwrap())
            .ok_or_else(|| DukaRuntimeError::IOError("attempt to use a closed file".into()))
    }

    fn read_line(&mut self, h: &mut Heap, keep_newline: bool) -> Vec<RuntimeValue> {
        match self.borrow_file() {
            Ok(mut f) => read_line_impl(&mut *f, h, keep_newline),
            Err(e) => err(h, e),
        }
    }

    fn read_all(&mut self, h: &mut Heap) -> Vec<RuntimeValue> {
        match self.borrow_file() {
            Ok(mut f) => read_all_impl(&mut *f, h),
            Err(e) => err(h, e),
        }
    }

    fn read_n(&mut self, h: &mut Heap, n: DukaInt) -> Vec<RuntimeValue> {
        if n < 0 {
            return cerr(h, "negative read count");
        }
        match self.borrow_file() {
            Ok(mut f) => read_n_impl(&mut *f, h, n),
            Err(e) => err(h, e),
        }
    }

    fn read_number(&mut self, h: &mut Heap) -> Vec<RuntimeValue> {
        match self.borrow_file() {
            Ok(mut f) => read_number_impl(&mut *f, h),
            Err(e) => err(h, e),
        }
    }
}

duka_user_data! {
    struct IOOut {
        err: bool,
        cache: Vec<u8>
    }
    constructor fn new(stderr: bool) -> Self {
        Self {
            err: stderr,
            cache: vec![]
        }
    }
    #[duka_builtin(
        flags(@returns(result)),
        doc = "Write content to this stream; nil is written as \"nil\". Returns [true, count] on success, [false, msg] on error",
        params(self: userdata, vals: vararg),
        returns(vararg)
    )]
    fn write(&mut self, sv: &mut CoState, h: &mut Heap, co: &mut NativeApi, vals: Vec<RuntimeValue>) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let mut len: usize = 0;
        for v in vals {
            let s = match format_arg(sv, h, co, &v) {
                Ok(v) => v,
                Err(e) => return Ok(cerr(h, e.to_string()))
            };
            let b = s.as_bytes();
            len += b.len();
            self.cache.extend_from_slice(b);
        }
        Ok(ok(RuntimeValue::Int(len as DukaInt)))
    },
    #[duka_builtin(
        flags(@returns(result)),
        doc = "Flush content to this stream",
        params(self: userdata),
        returns(vararg)
    )]
    fn flush(&mut self, h: &mut Heap, co: &mut NativeApi) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let r = if self.err{
            co.write_err_bytes(&std::mem::take(&mut self.cache))
        } else {
            co.write_bytes(&std::mem::take(&mut self.cache))
        };
        let len = match r {
            Ok(len) => len,
            Err(e) => return Ok(cerr(h, e.to_string()))
        };
        Ok(ok(RuntimeValue::Int(len as DukaInt)))
    },
}

duka_user_data! {
    struct IOIn {}
    constructor fn new() -> Self {
        Self {}
    }
    #[duka_builtin(
        doc = r#"Reads from standard input. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: "a" reads all, "l"/"L" reads a line, "n" reads a number. Returns [true, data] on success, [true, nil] at end of input, [false, msg] on error"#,
        params(self: userdata, what: vararg),
        returns(vararg)
    )]
    fn read(&mut self, h: &mut Heap, co: &mut NativeApi, what: Vec<RuntimeValue>) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let mut reader = input_reader(co);
        Ok(match what.first() {
            None => read_line_impl(&mut reader, h, false),
            Some(RuntimeValue::Int(n)) => read_n_impl(&mut reader, h, *n),
            Some(v) if v.is_string() => match v.eval_to_string().as_ref() {
                "a" | "*a" => read_all_impl(&mut reader, h),
                "l" | "*l" => read_line_impl(&mut reader, h, false),
                "L" | "*L" => read_line_impl(&mut reader, h, true),
                "n" | "*n" => read_number_impl(&mut reader, h),
                other => cerr(h, format!("invalid read format: {other}")),
            },
            Some(v) => cerr(h, format!("bad read argument of type {}", v.type_name_of())),
        })
    },
    #[duka_builtin(
        doc = "Returns an iterator that yields one line from standard input per iteration",
        params(self: userdata),
        returns(vararg)
    )]
    fn lines(&self, h: &mut Heap) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let func = RustClosure::returns_with_captures(
            move |c, h, api| {
                let mut reader = input_reader(api);
                match read_until_newline(&mut reader) {
                    Ok(mut buf) => {
                        if buf.is_empty() {
                            c.set_stack(0, RuntimeValue::Bool(false))?;
                            return Ok(ValueCount::Exact(1));
                        }
                        strip_trailing_newline(&mut buf);
                        c.set_stack(0, RuntimeValue::Bool(true))?;
                        c.set_stack(
                            1,
                            RuntimeValue::from_string(
                                h,
                                String::from_utf8_lossy(&buf).into_owned(),
                            ),
                        )?;
                        Ok(ValueCount::Exact(2))
                    }
                    Err(_) => {
                        c.set_stack(0, RuntimeValue::Bool(false))?;
                        Ok(ValueCount::Exact(1))
                    }
                }
            },
            vec![],
            Some("__stdin_lines".into()),
        );
        Ok(vec![RuntimeValue::from_rust_closure(h, func)])
    },
}
