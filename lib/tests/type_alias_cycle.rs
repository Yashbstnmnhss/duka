//! A type alias is its definition. When the definition leads back to the name,
//! there is no type to arrive at: say so instead of degrading to `any` or
//! expanding forever.

use duka_lib::harness::run;

#[test]
fn an_alias_that_refers_to_itself_is_reported() {
    let err = run("type A = A\nlocal x: A = 1\nreturn x\n").unwrap_err();
    assert!(err.to_string().contains("refers to itself"), "{err}");
}

#[test]
fn mutually_recursive_aliases_are_reported() {
    let err = run("type A = B\ntype B = A\nlocal x: A = 1\nreturn x\n").unwrap_err();
    assert!(err.to_string().contains("refers to itself"), "{err}");
}

#[test]
fn a_recursive_record_alias_is_reported_without_exploding() {
    let err = run("type A = { x: A }\nlocal a: A = nil\nreturn 0\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("refers to itself"), "{err}");
    assert!(
        err.len() < 1500,
        "the message stays readable instead of expanding the record: {} bytes",
        err.len()
    );
}

#[test]
fn an_ordinary_alias_still_works() {
    run("type N = int\nlocal n: N = 1\nreturn n\n").expect("a plain alias resolves");
}
