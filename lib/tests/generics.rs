//! Generic function type inference (L2) tests

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn explicit_typeargs_infers_return() {
    let res = run_results(
        r#"
function id<T>(x: T): T
    return x
end
local a: int = id.<int>(1)
local b: string = id.<string>("s")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

#[test]
fn inferred_from_args() {
    let res = run_results(
        r#"
function pick<T>(x: T): T
    return x
end
local a: int = pick(1)
local b: string = pick("s")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

#[test]
fn bound_accepts_valid() {
    let res = run_results(
        r#"
function bnd<T: int>(x: T): T
    return x
end
local a: int = bnd.<int>(1)
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn bound_rejects_invalid() {
    let err = run(r#"
function bnd<T: int>(x: T): T
    return x
end
local a: string = bnd.<string>("x")
return a
"#)
    .unwrap_err();
    assert!(err.contains("incompatible"), "{err}");
}

#[test]
fn explicit_typeargs_array_element() {
    let res = run_results(
        r#"
function first<T>(x: array<T>): T
    return x[0]
end
local a: int = first.<int>([1, 2])
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}