//! Match

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

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
function checker(n)
    return n > 5
end
global v = match 7 then
    local x and |> checker -> x;
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
local v
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
