//! `?` guards one step of a chain: a nil left side gives nil, it does not call.

use duka_lib::harness::run;

#[test]
fn a_nil_safe_call_returns_nil_and_skips_the_arguments() {
    run(r#"
local called = false
local side = fn() do
    called = true
    return 1
end
local f = nil
assert(f?() == nil)
assert(f?(side()) == nil)
assert(called == false)
"#)
    .unwrap();
}

#[test]
fn a_nil_safe_call_still_calls_a_function() {
    run(r#"
local add = fn(a, b) a + b
assert(add?(1, 2) == 3)
"#)
    .unwrap();
}

#[test]
fn a_nil_safe_method_call_returns_nil() {
    run(r#"
object Box
    function :get()
        return "boxed"
    end
end
local b = Box.new()
assert(b?:get() == "boxed")
local nothing = nil
assert(nothing?:get() == nil)
"#)
    .unwrap();
}

#[test]
fn a_nil_safe_index_and_field_return_nil() {
    run(r#"
local a = [1, 2]
assert(a?[0] == 1)
assert(a?.size == nil)
local nothing = nil
assert(nothing?[0] == nil)
assert(nothing?.deep == nil)
"#)
    .unwrap();
}

#[test]
fn a_nil_safe_call_works_as_a_statement() {
    run(r#"
local f = nil
f?("never")
local g = fn(s) s
g?("fine")
"#)
    .unwrap();
}

/// `a?.b()` guards the field read only: reading gives nil, so the call is a
/// call on nil and fails, exactly like writing `local f = a?.b; f()`.
#[test]
fn a_guard_before_a_field_does_not_guard_the_call() {
    let err = run(r#"
local a = nil
return a?.b()
"#)
    .unwrap_err();
    assert!(err.to_string().contains("function"), "{err}");
}
