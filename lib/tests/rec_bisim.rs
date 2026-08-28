use duka_lib::harness::{run, run_results};

fn strs(v: &[duka_backend::value::RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn tuple_type_single_index() {
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
fn chained_type_index_via_alias() {
    let r = run_results(
        r#"
type function List(T) = [T, List(T)?]
type A = List(int)
type B = A[1]
return 1
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["1"]);
}

#[test]
fn chained_type_index_index_into_union() {
    let r = run_results(
        r#"
type function List(T) = [T, List(T)?]
type A = List(int)
type C = A[1][0]
return 1
"#,
    )
    .unwrap();
    assert_eq!(strs(&r), ["1"]);
}

#[test]
fn reject_wrong_tail_element() {
    assert!(
        run(r#"
type function List(T) = [T, List(T)?]
local c: List(int) = [1, true]
return 1
"#)
        .is_err(),
        "[1, true] should not be a List(int)"
    );
}

#[test]
fn accept_valid_nested_list() {
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
