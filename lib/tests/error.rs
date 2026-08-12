//! Error

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_results;

#[test]
fn try_success_single_result() {
    assert_eq!(
        run_results("return try(function(a, b) return a + b end, 3, 4)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)]
    );
}

#[test]
fn try_success_native_callee() {
    assert_eq!(
        run_results("return try(math.max, 3, 9, 5)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(9)]
    );
}

#[test]
fn try_success_multiple_results() {
    assert_eq!(
        run_results("return try(function(a, b) return b, a end, 1, 2)").unwrap(),
        vec![
            RuntimeValue::Bool(true),
            RuntimeValue::Int(2),
            RuntimeValue::Int(1)
        ]
    );
}

#[test]
fn try_catches_error() {
    let res = run_results("return try(function() error(\"boom\") end)").unwrap();
    assert_eq!(res[0], RuntimeValue::Bool(false));
    assert_eq!(res[1].eval_to_string(), "boom");
}

#[test]
fn try_continues_after_error() {
    let res = run_results(
        r#"
global mark = nil
try(function() error("boom") end)
mark = "alive"
return try(function() return 2 + 5 end)
        "#,
    )
    .unwrap();
    assert_eq!(res, vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)]);
}

#[test]
fn try_value_is_returned_after_error() {
    let res = run_results(
        r#"
local ok, res = try(function() return 1, 2, 3 end)
return ok, res
        "#,
    )
    .unwrap();
    assert_eq!(res[0], RuntimeValue::Bool(true));
    assert_eq!(res[1], RuntimeValue::Int(1));
}
