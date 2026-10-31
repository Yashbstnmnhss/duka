//! `select` runtime: a count, one value, or the values an array of indices
//! names. Return typing is the type system's business, not this file's.

use duka_lib::harness::run;

#[test]
fn the_count_pattern_answers_with_how_many_values_there_are() {
    run(r##"
assert(select("#") == 0)
assert(select("#", 1) == 1)
assert(select("#", 1, 2, 3) == 3)
"##)
    .unwrap();
}

#[test]
fn an_index_picks_the_value_it_names() {
    run(r#"
assert(select(0, "a", "b") == "a")
assert(select(1, "a", "b") == "b")
assert(select(-1, "a", "b") == "b")
"#)
    .unwrap();
}

#[test]
fn an_array_of_indices_picks_them_in_order() {
    run(r#"
local a, b = select([1, 0], "x", "y")
assert(a == "y" and b == "x")
"#)
    .unwrap();
}

#[test]
fn an_index_past_the_end_is_nil() {
    run(r#"
    assert(select(9, "a", "b") == nil)
"#)
    .unwrap();
}

/// The pattern decides the answer's type: a count is an `int`, a literal index
/// is that element, an array of indices is the tail's members.
#[test]
fn the_pattern_decides_the_return_type() {
    run(r##"
local n: int = select("#", 1, 2, 3)
local s: string = select(0, "a", "b")
local last: string = select(-1, "a", "b")
local x, y = select([0, 1], "a", 9)
local xs: string = x
local yi: int = y
local c, d = select([1, 0], "a", 9)
local ci: int = c
local ds: string = d
local picked: int = select(1, 1, 2)
assert(n == 3 and s == "a" and last == "b" and x == "a" and y == 9)
assert(c == 9 and d == "a" and picked == 2)
"##)
    .unwrap();

    let err = run(r##"
local n: string = select("#", 1, 2)
return n
"##)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");

    let err = run(r#"
local n: int = select(0, "a", "b")
return n
"#)
    .unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");

    // the array pattern gives one slot per index, in the order written
    let err = run(r#"
local a, b = select([0, 1], "a", 9)
local n: int = a
return n
"#)
    .unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
}
