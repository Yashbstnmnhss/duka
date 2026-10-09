//! What a call whose callee is itself a call is worth. `f()()` has no path to
//! name, so it is read off what the callee expression evaluates to.

use duka_lib::module::from_source;
use duka_shared::config::DukaConfig;
use duka_shared::types::SourceName;

fn compiles(src: &str) -> Result<(), String> {
    from_source(src, SourceName::Unnamed, DukaConfig::default())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn the_inner_call_is_what_the_outer_one_is_asked_about() {
    // `f()` hands back a function, so `f()` is a callee and the call on it
    // answers `int`. Read as unknown it would claim to answer any number of
    // values, which is what let this through with an argument it cannot take.
    let err = compiles(
        "local f = function() return function() return 7 end end\nlocal function g() return 1 end\ng(f()())",
    )
    .expect_err("g takes no argument");
    assert!(err.contains("Expected 0 argument"), "got: {err}");
}

#[test]
fn a_nested_call_reaches_its_callees_return() {
    compiles(
        "local f = function() return function(a, b) return a + b end end\nlocal r = f()(1, 2)\nassert(r == 3)",
    )
    .expect("the inner call answers a function, the outer one two arguments");
}

#[test]
fn arity_is_checked_through_a_nested_call() {
    compiles(
        "local f = function() return function() return 1 end end\nlocal r = f()()\nassert(r == 1)",
    )
    .expect("f()() needs no arguments");
    let err =
        compiles("local f = function() return function(a) return a end end\nlocal r = f()(1, 2)")
            .expect_err("f()() takes one argument");
    assert!(err.contains("Expected 1 argument"), "got: {err}");
}

#[test]
fn a_nested_call_says_nothing_when_the_callee_is_not_a_function() {
    // nothing here is callable, so the result stays unknown rather than
    // becoming a type nothing could hold
    let _ = compiles("local f = function() return 1 end\nlocal r = f()()");
}

#[test]
fn the_value_a_nested_call_answers_is_checked_against_a_parameter() {
    // `f()()` is an `int`. Read as unknown it would pass any parameter, which
    // is the only reason this went unreported.
    let err = compiles(
        "local f = function() return function() return 7 end end\nlocal function want(s: string) return s end\nwant(f()())",
    )
    .expect_err("an int is not a string");
    assert!(err.contains("string"), "got: {err}");

    // and in a position that asks for one type rather than a list of them
    let err = compiles(
        "local f = function() return function() return 7 end end\nlocal function want(a: string, b: string) return a end\nwant(f()(), \"x\")",
    )
    .expect_err("an int is not a string");
    assert!(err.contains("string"), "got: {err}");
}
