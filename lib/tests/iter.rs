//! Iter library: lazy iterator closures + `|>` pipeline chains

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

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
