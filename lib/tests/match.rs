//! Match

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_last};

/// An `or` pattern binds only the alternative that matched. The first
/// alternative here fails, and the one that matches binds nothing, so the
/// name the body reads is not the value the failed alternative would give.
#[test]
fn an_or_pattern_binds_only_the_alternative_that_matched() {
    let r = run_last(
        r#"
local v = { 9, 2 }
return match v then
    [1, local x] or [9, 2] -> x;
    else return -1
    end
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Nil, "`x` was never bound");
}

#[test]
fn an_or_pattern_uses_the_alternative_that_matched() {
    let r = run_last(
        r#"
local v = { first = "T" }
return match v then
    { first = local head } or [local head, ...] -> head;
    else return "none"
    end
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "T");
}

/// A `match` expression answers with what its arms return, so the union is
/// what the caller has to take -- not `any`.
#[test]
fn a_match_expression_says_what_its_arms_return() {
    let err = run(
        r#"
local function f(x: string)
    return match x then
        ("a" or "b") -> "hit";
        else return 0
    end
end
local s: string = f("a")
return s
"#,
    )
    .expect_err("the else arm answers an int");
    let text = err.to_string();
    assert!(text.contains("string | int"), "{text}");
}

#[test]
fn match_basic_constant_clause() {
    let r = run_last(
        r#"
global v = match 2 then
    1 -> 1;
    2 -> 2;
    else return 0
    end
return v
        "#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(2));
}

#[test]
fn match_binding_wildcard() {
    let r = run_last(
        r#"
global v = match 42 then
    1 -> 1;
    local x -> x + 1;
    else return 0
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(43));
}

#[test]
fn match_binding_top_clause_uses_bound_var() {
    let r = run_last(
        r#"
global v = match "hello" then
    local s -> s;
    else return "unmatched"
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "hello");
}

#[test]
fn match_binding_single_layer_table_field() {
    let r = run_last(
        r#"
local t = {10, 20, 30}
global v = match t then
    { local x, ... } -> x;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn match_binding_named_field() {
    let r = run_last(
        r#"
local t = {a = 5, b = 6}
global v = match t then
    { a = local av, b = local bv } -> av + bv;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn match_table_len_check_guards_unmatched() {
    let r = run_last(
        r#"
local t = {1, 2}
global v = match t then
    { local x, local y, local z } -> x + y + z;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(-1));
}

#[test]
fn match_bind_compound_and_call() {
    let r = run_last(
        r#"
function checker(n: int)
    return n > 5
end
global v = match 7 then
    local x and |$> checker -> x;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn match_bind_typed_int_matches_and_binds() {
    let r = run_last(
        r#"
global v = match 42 then
    local x: int -> x;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn match_bind_typed_string_wont_match_int() {
    let r = run_last(
        r#"
global v = match 42 then
    local s: string -> -100;
    else return -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(-1));
}

#[test]
fn match_bind_typed_float_matches_int_and_float() {
    let r = run_last(
        r#"
global v = match 7 then
    local n: float -> n * 2;
    else return -1
end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(14));
}

#[test]
fn match_bind_typed_union_matches_either() {
    let r = run_last(
        r#"
global v = match true then
    local b: bool | string -> b;
    else return -1
end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn match_statement_form_sequential_clauses() {
    let r = run_last(
        r#"
local v = nil
match 3 then
    1 -> do v = 1 end;
    local _ -> do v = 99 end;
    else v = -1
    end
return v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(99));
}
