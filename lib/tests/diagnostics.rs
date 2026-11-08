//! Diagnostics say which mistake was made, and only an error stops the program.

use duka_lib::harness::run;

#[test]
fn a_declaration_with_no_value_still_runs_when_it_is_written_first() {
    // `local x: int` says what the name will hold; the write follows, so the
    // warning is not a reason to refuse the program
    let out = run("local x: int\nx = 1\nreturn x\n").expect("a written name is fine");
    assert_eq!(out.len(), 1);
}

#[test]
fn a_read_before_the_write_is_reported() {
    let err = run("local x: int\nreturn x\n")
        .expect_err("nothing was ever put in `x`")
        .to_string();
    assert!(err.contains("read before a value"), "{err}");
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
