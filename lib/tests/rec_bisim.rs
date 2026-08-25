use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn rec_list_accepts_matching() {
    let r = run_results(
        r#"
type function List(T) = [T, List(T)?]
local xs: List(int) = [1, [2, nil]]
return xs[0]
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["1"]);
}

#[test]
fn rec_list_rejects_wrong_type() {
    assert!(
        run(r#"
type function List(T) = [T, List(T)?]
local xs: List(int) = ["hello", nil]
return 1
"#)
        .is_err(),
        "should reject string in int list"
    );
}

#[test]
fn rec_list_head_access() {
    let r = run_results(
        r#"
type function List(T) = [T, List(T)?]
local h: List(int)[0] = 42
return h
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["42"]);
}

#[test]
fn rec_nested_access() {
    let r = run_results(
        r#"
type function List(T) = [T, List(T)?]
local xs: List(int) = [1, [2, nil]]
return xs[1][0]
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["2"]);
}

#[test]
fn different_rec_types_work_independently() {
    let r = run_results(
        r#"
type function IL() = [int, IL()?]
type function SL() = [string, SL()?]
local a: IL = [1, [2, nil]]
local b: SL = ["x", ["y", nil]]
return a[0], b[0]
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["1", "x"]);
}
