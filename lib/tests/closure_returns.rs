//! Inferring what a closure returns: a closure has no name to file its return
//! types under, so they are keyed by where it was written instead.

use duka_lib::module::from_source;
use duka_shared::config::DukaConfig;
use duka_shared::types::SourceName;

fn compiles(src: &str) -> Result<(), String> {
    from_source(src, SourceName::Unnamed, DukaConfig::default())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn a_closures_return_is_inferred() {
    assert!(compiles("local f = function() return 1 end\nassert(f())").is_ok());
    assert!(compiles("local f = function() return 1, 2 end\nassert(f())").is_ok());
}

#[test]
fn the_inferred_value_reaches_a_nested_call() {
    // the arity of `g` cannot be read at all unless `f()` is known to hand
    // something back
    compiles(
        "local g = function(a) return a end\nlocal f = function() return 1 end\nassert(g(f()))",
    )
    .expect("g(f()) should compile");
}

#[test]
fn a_closure_that_returns_nothing_still_hands_over_nothing() {
    // this one is right: a call in the last argument position that yields no
    // values is no argument at all, and `assert` needs one
    let err = compiles("local f = function() end\nassert(f())")
        .expect_err("assert(f()) has no value to assert");
    assert!(err.contains("Expected 1 argument"), "got: {err}");
}

#[test]
fn a_closure_reached_through_a_table_keeps_its_return() {
    compiles("local t = { f = function() return 5 end }\nassert(t.f())")
        .expect("t.f() should compile");
    compiles("local m = {}\nm.k = function() return 9 end\nassert(m.k())")
        .expect("m.k() should compile");
}

#[test]
fn several_closures_are_told_apart_by_where_they_were_written() {
    compiles("local v = function() end\nlocal w = function() return 1 end\nassert(w())")
        .expect("the void one must not be confused for the other");
}

#[test]
fn a_closure_returning_a_closure_keeps_both_returns() {
    compiles("local function mk() return function() return 7 end end\nassert(mk()())")
        .expect("both closures' returns should be inferred");
    compiles("local outer = function() return function() return 3 end end\nassert(outer()())")
        .expect("a nested closure's returns should be inferred");
}

#[test]
fn an_annotated_closure_still_wins_over_what_it_returns() {
    // the annotation is the whole answer; inference must not override it
    assert!(compiles("local f = function(): int return \"s\" end").is_err());
    assert!(compiles("local f = function(): int return 1 end\nassert(f())").is_ok());
}
