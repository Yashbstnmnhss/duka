//! Linq!

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

#[test]
fn linq_from_select() {
    // 从表选出值,结果按序写入 out[1..n]
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    select x * 10
)
return out[1] + out[2] + out[3]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn linq_where_filter() {
    // where 过滤后 select 映射
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    where x > 1
    select x * 10
)
return out[1] + out[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(50));
}

#[test]
fn linq_empty_result() {
    // 全部被过滤:out 为空表,out[1] = nil
    let r = run_last(
        r#"
global arr = {1, 2, 3}
global out = linq!(
    from x in arr
    where x > 100
    select x
)
return out[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Nil);
}

#[test]
fn linq_multiple_clauses() {
    // 多 from 组合 + where 链
    let r = run_last(
        r#"
global a = {1, 2}
global b = {10, 20}
global out = linq!(
    from x in a
    from y in b
    where x + y > 15
    select x + y
)
return out[1] + out[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(43));
}
