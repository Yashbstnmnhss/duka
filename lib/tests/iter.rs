//! Iterator

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

#[test]
fn for_over_table_single_var_values() {
    // 单变量遍历表:绑定的是值(1+2+3=6)
    let r = run_last(
        r#"
global arr = {1, 2, 3}
local sum = 0
for x in arr do
    sum = sum + x
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn for_over_table_two_vars_key_value() {
    // 双变量遍历表:绑定 (k, v),与 pairs 一致
    let r = run_last(
        r#"
global arr = {10, 20}
local total = 0
for k, v in arr do
    total = total + v
end
return total
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(30));
}

#[test]
fn for_over_table_literal() {
    // 表字面量直接遍历
    let r = run_last(
        r#"
local sum = 0
for x in {1, 2, 3, 4} do
    sum = sum + x
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn for_over_table_empty() {
    // 空表不迭代
    let r = run_last(
        r#"
local sum = 0
for x in {} do
    sum = sum + 1
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn for_over_function_iterator_unchanged() {
    // 首值是函数时保持原协议,不受表糖影响(回归:显式 f,s,c 三参数)
    let r = run_last(
        r#"
local sum = 0
local function gen(s, i)
    if i >= 3 then return false end
    return true, i + 1
end
for k in gen, nil, 0 do
    sum = sum + k
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}
