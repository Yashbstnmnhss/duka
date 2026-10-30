//! FFI
//!
//! A call is a promise from the runtime that the declaration and the library
//! agree; these tests only use what every build links against.

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_results;

#[test]
fn a_declaration_that_is_not_a_header_is_refused() {
    let results = run_results(
        r#"
local ok = ffi.cdef("int ;")
return ok
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Bool(false)]);
}

#[test]
fn a_name_that_was_never_declared_is_an_error_not_a_panic() {
    let results = run_results(
        r#"
local ok, err = ffi.call("duka_no_such_function")
if ok then return "called" end
return #err > 0
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Bool(true)]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_declared_function_is_called_and_its_answer_read() {
    let results = run_results(
        r#"
local ok, err = ffi.cdef("int abs(int n);")
if not ok then return "cdef: " .. err end
local loaded, lerr = ffi.load("libc.so.6")
if not loaded then return "load: " .. lerr end
local called, answer = ffi.call("abs", -3)
if not called then return "call: " .. answer end
return answer
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Int(3)]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_string_argument_is_handed_over_as_a_c_string() {
    let results = run_results(
        r#"
ffi.cdef("typedef unsigned long size_t; size_t strlen(const char *s);")
ffi.load("libc.so.6")
local ok, n = ffi.call("strlen", "hello")
if not ok then return -1 end
return n
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Int(5)]);
}

#[cfg(target_os = "linux")]
#[test]
fn a_struct_returned_by_value_is_refused_rather_than_unwrapped() {
    let results = run_results(
        r#"
ffi.cdef("struct div_t { int quot; int rem; }; struct div_t div(int n, int d);")
ffi.load("libc.so.6")
local ok = ffi.call("div", 7, 2)
return ok
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Bool(false)]);
}

#[cfg(target_os = "windows")]
#[test]
fn a_declared_function_is_called_and_its_answer_read() {
    let results = run_results(
        r#"
local ok, err = ffi.cdef("unsigned long GetCurrentProcessId(void);")
if not ok then return "cdef: " .. err end
local loaded, lerr = ffi.load("kernel32.dll")
if not loaded then return "load: " .. lerr end
local called, answer = ffi.call("GetCurrentProcessId")
if not called then return "call: " .. answer end
return answer > 0
"#,
    )
    .unwrap();
    assert_eq!(results, vec![RuntimeValue::Bool(true)]);
}
