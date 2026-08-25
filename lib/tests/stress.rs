//! Type system stress tests
use duka_lib::harness::{run, run_results};

#[test]
fn infinite_recursion_hits_fuel() {
    let err = run(r#"
type function Loop(n)
    return Loop(n + 1)
end
local x: Loop(0) = 1
return x
"#)
    .unwrap_err();
    assert!(err.contains("fuel") || err.contains("max"), "{err}");
}

#[test]
fn mutual_recursion_hits_limit() {
    let err = run(r#"
type function Even(n)
    if n == 0 then return true end
    return Odd(n - 1)
end
type function Odd(n)
    if n == 0 then return false end
    return Even(n - 1)
end
local x: Even(9999) = true
return x
"#)
    .unwrap_err();
    assert!(err.contains("max") || err.contains("fuel"), "{err}");
}

#[test]
fn deep_but_bounded_recursion_works() {
    let res = run_results(
        r#"
type function Count(n)
    if n <= 0 then return 0 end
    return Count(n - 1) + 1
end
local x: Count(15) = 15
return x
"#,
    )
    .unwrap();
    assert!(!res.is_empty());
}

#[test]
fn self_referential_alias_no_hang() {
    let r = run(r#"
type A = B
type B = A
local x: A = 1
return x
"#);
    assert!(r.is_ok() || r.is_err());
}

#[test]
fn unknown_type_fn_is_error() {
    let err = run("local x: NoSuchFn(int) = 1").unwrap_err();
    assert!(err.contains("unknown") || err.contains("not"), "{err}");
}

#[test]
fn wrong_arg_count_errors() {
    let err = run(r#"
type function F(a, b)
    return a
end
local x: F(1) = 1
"#)
    .unwrap_err();
    assert!(err.contains("expected 2 arguments"), "{err}");
}

#[test]
fn assign_to_non_table_errors() {
    let err = run(r#"
type function F()
    local x = 42
    x.field = int
    return x
end
local v: F() = 1
return v
"#)
    .unwrap_err();
    assert!(err.contains("not a table"), "{err}");
}

#[test]
fn index_oob_errors() {
    let err = run(r#"
type function F()
    local t = [int, string]
    t[99] = bool
    return t
end
local v: F() = 1
return v
"#)
    .unwrap_err();
    assert!(err.contains("out of bounds"), "{err}");
}

#[test]
fn empty_source_no_panic() {
    let _ = run("");
}

#[test]
fn comments_only_no_panic() {
    let _ = run("-- nothing\n-- more");
}
