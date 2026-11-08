//! Diagnostics say which mistake was made, and only an error stops the program.

use duka_lib::harness::run;

#[test]
fn a_declaration_with_nothing_in_it_must_allow_nil() {
    // it holds `nil` until written, and `int` is not that
    let err = run("local x: int\nreturn 0\n")
        .expect_err("an int declaration cannot start nil")
        .to_string();
    assert!(err.contains("starts with no value"), "{err}");
    assert!(err.contains("nil is not 'int'"), "{err}");
}

#[test]
fn a_nilable_declaration_starts_nil_and_takes_a_value_later() {
    let out = run("local x: int?\nx = 1\nreturn x\n").expect("the write fills it in");
    assert_eq!(out.len(), 1);
}

#[test]
fn a_read_of_a_nilable_name_is_nil_until_written() {
    let err = run("local x: int?\nlocal n: int = x\nreturn n\n")
        .expect_err("nothing was ever put in `x`")
        .to_string();
    assert!(err.contains("nil"), "{err}");
}

#[test]
fn an_assignment_is_reported_as_an_assignment() {
    let err = run("local x = 1\nx = \"s\"\nreturn x\n")
        .expect_err("a string is not an int")
        .to_string();
    assert!(err.contains("Cannot assign a value of type 'string'"), "{err}");
    assert!(!err.contains("initializer"), "{err}");
}

#[test]
fn a_wrong_argument_count_says_how_many() {
    let err = run("local function f(a: int) return a end\nreturn f(1, 2)\n")
        .expect_err("one parameter, two arguments")
        .to_string();
    assert!(
        err.contains("Expected 1 argument, but got 2 arguments"),
        "{err}"
    );
}
