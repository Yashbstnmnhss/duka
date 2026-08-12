//! Array literals `[a, b, c]`

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

#[test]
fn array_literal_basic_index() {
    let r = run_last(
        r#"
local a = [1, 2, 3]
return a[0] + a[1] + a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn array_literal_empty() {
    let r = run_last(
        r#"
local a = []
return #a
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn array_literal_nested() {
    let r = run_last(
        r#"
local m = [[1, 2], [3, 4]]
return m[0][0] + m[0][1] + m[1][0] + m[1][1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn array_literal_three_level_nested() {
    let r = run_last(
        r#"
local a = [[[1], [2]], [[3]]]
return a[0][1][0] + a[1][0][0]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn array_literal_variables_and_expr() {
    let r = run_last(
        r#"
local x = 10
local y = 20
local a = [x, y, x + y]
return a[0] + a[1] + a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn array_literal_trailing_comma() {
    let r = run_last(
        r#"
local a = [1, 2, 3,]
return a[0] + a[1] + a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn array_literal_semicolon_separator() {
    let r = run_last(
        r#"
local a = [1; 2; 3]
return a[0] + a[1] + a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn array_literal_call_arg() {
    let r = run_last(
        r#"
function sum(t)
    return t[0] + t[1] + t[2]
end
return sum([4, 5, 6])
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn array_literal_chained_index() {
    let r = run_last(
        r#"
return [10, 20, 30][1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(20));
}

#[test]
fn array_literal_negative_index_element() {
    let r = run_last(
        r#"
local a = [0, -1, -2]
return a[1] + a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(-3));
}

#[test]
fn array_literal_mixed_types() {
    let r = run_last(
        r#"
local a = ["hello", 42, true]
local s = a[0] .. " " .. a[1] .. " " .. a[2]
return s
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "hello 42 bool");
}

#[test]
fn array_literal_mutation_persists() {
    let r = run_last(
        r#"
local a = [0, 0]
a[0] = 7
a[1] = 8
return a[0] * 10 + a[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(78));
}

#[test]
fn array_literal_each_call_fresh() {
    let r = run_last(
        r#"
function make()
    return [0]
end
local a = make()
local b = make()
a[0] = 5
return b[0]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn array_literal_iterate() {
    let r = run_last(
        r#"
local a = [1, 2, 3]
local total = 0
for i = 0, #a - 1 do
    total = total + a[i]
end
return total
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn long_string_still_works_after_array() {
    let r = run_last(
        r#"
local a = [1, 2]
local s = [=[line1
line2]=]
local t = [==[x]==]
return a[0] + a[1] + #s + #t
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(3 + 11 + 1));
}

#[test]
fn array_in_table_field() {
    let r = run_last(
        r#"
local t = { head = [1, 2], tail = [3, 4] }
return t.head[0] + t.tail[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(1 + 4));
}

#[test]
fn table_in_array() {
    let r = run_last(
        r#"
local a = [{x = 1}, {x = 2}]
return a[0].x + a[1].x
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(3));
}

#[test]
fn array_literal_returned() {
    let r = run_last(
        r#"
function two()
    return [8, 9]
end
local a = two()
return a[0] + a[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(17));
}

#[test]
fn array_type_is_distinct_from_table() {
    let r = run_last(r#"return type([1, 2])"#).unwrap();
    assert_eq!(r.eval_to_string(), "array");
    let r = run_last(r#"return type({1, 2})"#).unwrap();
    assert_eq!(r.eval_to_string(), "table");
}

#[test]
fn array_oob_read_is_nil() {
    let r = run_last(
        r#"
local a = [1, 2]
return a[5]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Nil);
}

#[test]
fn array_index_grows_automatically() {
    let r = run_last(
        r#"
local a = [0]
a[3] = 9
return #a * 10 + a[3]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(49));
}

#[test]
fn array_grown_slot_is_nil() {
    let r = run_last(
        r#"
local a = [0]
a[3] = 9
return a[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Nil);
}

#[test]
fn array_set_float_key_ignored() {
    let r = run_last(
        r#"
local a = [1, 2]
a[0.5] = 99
return a[0] + a[1]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(3));
}
