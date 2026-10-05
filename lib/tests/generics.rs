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
    let message = err.to_string();
    assert!(message.contains("does not satisfy its bound"), "{message}");
    assert!(message.contains("'T'"), "{message}");
    assert!(!message.contains("incompatible"), "{message}");
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

/// `|` and `&` are the type grammar's own operators, so a type position that
/// uses them stays a type. If this regressed into reading the whole annotation
/// as an expression, `|` would become a boolean or and the annotation would
/// stop meaning "either of these".
#[test]
fn a_union_in_a_type_position_is_still_a_union() {
    let res = run_results(
        r#"
local a: int | string = 1
local b: int | string = "x"
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "x"]);
}

#[test]
fn a_type_parameter_bound_may_be_a_union() {
    let res = run_results(
        r#"
function f<T: int | string>(x: T): T
    return x
end
local a: int = f.<int>(1)
local b: string = f.<string>("x")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "x"]);
}
