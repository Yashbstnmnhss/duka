//! Builtins read as values.

use duka_lib::harness::run;

#[test]
fn a_builtin_local_is_callable() {
    run(r#"
type function Use()
    local f = Patterns.GtPat
    return f(int, 5)
end
type P = Use()
function g(x: P): int
    return 1
end
return g(10)
"#)
    .unwrap();
}

#[test]
fn a_builtin_local_still_holds_what_it_built() {
    assert!(
        run(r#"
type function Use()
    local f = Patterns.GtPat
    return f(int, 5)
end
type P = Use()
function g(x: P): int
    return 1
end
g(1)
"#)
        .is_err()
    );
}

#[test]
fn a_builtin_stored_in_a_table_keeps_its_body() {
    assert!(
        run(r#"
type function Store()
    local t = { p = Patterns.GtPat }
    return t.p
end
type Q = Store()
function h(x: Q): int
    return 1
end
h(1)
"#)
        .is_err()
    );
}
