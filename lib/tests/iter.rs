//! Iter library: lazy iterator closures + `|>` pipeline chains

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_last};

#[test]
fn ipairs_walks_an_array() {
    let r = run_last(
        r#"
local sum = 0
for i, v in ipairs([10, 20, 30]) do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn ipairs_walks_a_table() {
    let r = run_last(
        r#"
local sum = 0
for i, v in ipairs({ 10, 20, 30 }) do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn a_for_over_an_array_literal_binds_the_element_type() {
    let err = run(r#"
for x in [10, 20, 30] do
    local s: string = x
    return s
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
}

#[test]
fn a_for_over_a_table_literal_binds_the_value_type() {
    let err = run(r#"
for k, v in { a = 1 } do
    local s: string = v
    return s
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
}

#[test]
fn a_for_over_ipairs_binds_the_element_type() {
    let err = run(r#"
for i, v in ipairs([1, 2, 3]) do
    local s: string = v
    return s
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
}

#[test]
fn a_for_over_a_range_binds_int() {
    let err = run(r#"
for v in iter.range(1, 3) do
    local s: string = v
    return s
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
}

#[test]
fn a_for_over_repeat_binds_the_repeated_value() {
    let err = run(r#"
for v in iter.repeat("x", 2) do
    local n: int = v
    return n
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
}

#[test]
fn pairs_refuses_an_array() {
    let err = run(r#"
for k, v in pairs([1, 2]) do print(k) end
return 0
"#)
    .unwrap_err();
    assert!(err.to_string().contains("table"), "{err}");
}

#[test]
fn iter_range_for_loop() {
    let r = run_last(
        r#"
local sum = 0
for v in iter.range(1, 5) do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn iter_range_negative() {
    let r = run_last(
        r#"
local sum = 0
for v in iter.range(5, 0, -1) do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn iter_map_array() {
    let r = run_last(
        r#"
local sum = 0
for v in iter.map([1, 2, 3], function(x) return x * 10 end) do
    sum = sum + v
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn iter_filter_array() {
    let r = run_last(
        r#"
local sum = 0
for v in iter.filter([1, 2, 3, 4, 5], function(x) return x % 2 == 1 end) do
    sum = sum + v
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(9));
}

#[test]
fn iter_take_array() {
    let r = run_last(
        r#"
local sum = 0
for v in iter.take([1, 2, 3, 4], 2) do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(3));
}

#[test]
fn iter_to_array_array() {
    let r = run_last(
        r#"
local a = [1, 2, 3]
local b = iter.to_array(iter.map(a, function(x) return x * 2 end))
return b[0] + b[1] + b[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(12));
}

#[test]
fn iter_pipeline_chain() {
    let r = run_last(
        r#"
local { map, filter, to_array } = iter
local nums = [1, 2, 3, 4, 5, 6]
local out = nums
    |> map(fn(x) x * 10)
    |> filter(fn(x) x > 30)
    |> to_array()
return out[0] + out[1] + out[2]
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(150));
}

#[test]
fn iter_map_chained_on_iterator() {
    let r = run_last(
        r#"
local sum = 0
local base = iter.range(1, 4)
for v in iter.map(iter.map(base, function(x) return x + 1 end), function(x) return x * 2 end) do
    sum = sum + v
end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(18));
}

#[test]
fn iter_for_over_array_literal() {
    let r = run_last(
        r#"
local sum = 0
for v in [1, 2, 3] do sum = sum + v end
return sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}
