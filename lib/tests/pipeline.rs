//! Pipeline `|>` / `<|` semantics

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

#[test]
fn pipeline_forward_inserts_first_arg() {
    // 2 |> a(1) -> a(2, 1)
    let r = run_last(
        r#"
function a(x, y) return x * 100 + y end
return 2 |> a(1)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(201));
}

#[test]
fn pipeline_right_appends_last_arg() {
    // a(1) <| 2 -> a(1, 2)
    let r = run_last(
        r#"
function a(x, y) return x * 100 + y end
return a(1) <| 2
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(102));
}

#[test]
fn pipeline_mixed_chain() {
    // 3 |> a(4) <| 5 -> a(3, 4, 5)
    let r = run_last(
        r#"
function a(x, y, z) return x + y * 10 + z * 100 end
return 3 |> a(4) <| 5
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(543));
}

#[test]
fn pipeline_bare_function_name() {
    // 21 |> foo -> foo(21)
    let r = run_last(
        r#"
function foo(v) return v * 2 end
return 21 |> foo
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn pipeline_forward_chain() {
    // 3 |> a(1) |> b(2) -> b(a(3, 1), 2)
    let r = run_last(
        r#"
function a(x, y) return x * 10 + y end
function b(x, y) return x - y end
return 3 |> a(1) |> b(2)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(29));
}
