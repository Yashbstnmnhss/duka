use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}
#[test]
fn a_contextual_keyword_is_usable_as_a_name() {
    let res = run_results(
        r#"
local type = 1
local where = 2
local ensure = 3
local t = { ensure = 4 }
local s = "ensure"
return type, where, ensure, t.ensure, s
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "2", "3", "4", "ensure"]);
}
#[test]
fn it_is_still_a_keyword_in_the_return_annotation() {
    let res = run_results(
        r#"
local ensure = 3
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
return not_nil(1), ensure
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "3"]);
}
#[test]
fn an_ensure_annotation_parses_and_the_function_still_works() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
return not_nil(1)
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool"]);
}
#[test]
fn an_ensure_cannot_be_written_beside_a_return_type() {
    let err = run(r#"
function not_nil<T>(t: T): int, ensure NonNil(T)
    return t ~= nil
end
return not_nil(1)
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn an_ensure_makes_the_function_return_one_bool() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
end
return not_nil(1)
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn an_ensure_checks_what_the_function_returns_against_bool() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t
end
return not_nil(1)
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn an_ensure_narrows_the_variable_the_guarantee_was_about() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if not_nil(x) then
    return takes_int(x)
end
return 0
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn the_narrowing_does_not_outlive_the_block_it_was_read_in() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if not_nil(x) then
end
return takes_int(x)
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn a_call_whose_answer_is_dropped_narrows_nothing() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
not_nil(x)
return takes_int(x)
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn an_ensure_in_a_loop_condition_holds_in_the_body() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
while not_nil(x) do
    return takes_int(x)
end
return 0
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_call_the_condition_does_not_stand_on_proves_nothing() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if x == nil or not_nil(x) then
    return takes_int(x)
end
return 0
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn an_ensure_reached_through_an_and_chain_still_holds() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if x ~= nil and not_nil(x) then
    return takes_int(x)
end
return 0
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn writing_to_a_name_ends_what_the_flow_knew_about_it() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if not_nil(x) then
    x = nil
    return takes_int(x)
end
return 0
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn the_else_reads_the_promise_the_other_way_round() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local x: int? = nil
if not_nil(x) then
    return "non-nil"
else
    local y: nil = x
    return "nil"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["nil"]);
}
#[test]
fn a_call_the_else_does_not_stand_on_proves_nothing() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local x: int? = 1
local ok = false
if not_nil(x) and ok then
    return 0
else
    local y: nil = x
    return 1
end
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn a_negated_condition_proves_the_call_failed() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local x: int? = 1
if not not_nil(x) then
    return "nil"
else
    local y: int = x
    return "int"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["int"]);
}
#[test]
fn a_later_clause_reads_the_earlier_promise_the_other_way_round() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_nil(v: nil): nil
    return v
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
local y: int? = 1
if not_nil(x) then
    takes_int(x)
elseif not_nil(y) then
    takes_int(y)
    takes_nil(x)
end
return 1
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn the_first_clause_of_a_nested_if_keeps_what_the_outer_clause_proved() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if not_nil(x) then
    if true then
        takes_int(x)
    end
end
return 1
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_local_of_an_else_belongs_to_the_else() {
    let res = run_results(
        r#"
local function takes_int(v: int!): int
    return v
end
local y: int! = 1
if true then
else
    local y = "s"
end
return takes_int(y)
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_promise_read_through_not_comes_back_the_other_way_round() {
    let res = run_results(
        r#"
function is_nil<T>(t: T): ensure Not(T, NonNil(T))
    return t == nil
end
local function takes_nil(v: nil): nil
    return v
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if is_nil(x) then
    takes_nil(x)
    return "nil"
else
    takes_int(x)
    return "int"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["int"]);
}
#[test]
fn any_hands_over_what_the_side_that_did_not_fail_promised() {
    let res = run_results(
        r#"
function int_or_not_nil<T>(t: T): ensure Any(NotIn(T, int), NonNil(T))
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if int_or_not_nil(x) then
    return takes_int(x)
end
return 0
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn all_hands_over_what_the_side_that_did_not_fail_promised() {
    let res = run_results(
        r#"
function int_and_not_nil<T>(t: T): ensure All(NotIn(T, int), NonNil(T))
    return t ~= nil
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if int_and_not_nil(x) then
    return takes_int(x)
end
return 0
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_check_against_nil_says_which_side_it_fell_on() {
    let res = run_results(
        r#"
local function takes_nil(v: nil): nil
    return v
end
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
if x == nil then
    takes_nil(x)
    return "nil"
else
    takes_int(x)
    return "int"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["int"]);
}
#[test]
fn a_check_against_nil_read_the_other_way_round_says_the_same() {
    let res = run_results(
        r#"
local function takes_nil(v: nil): nil
    return v
end
local function takes_int(v: int!): int
    return v
end
local x: int? = nil
if x ~= nil then
    return takes_int(x)
else
    takes_nil(x)
    return "nil"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["nil"]);
}
#[test]
fn a_local_initialized_in_the_else_holds_the_nil_it_read() {
    let res = run_results(
        r#"
local c: int? = nil
if c ~= nil then
    return 1
else
    local a = c
    local b: nil = a
    return 0
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["0"]);
}
#[test]
fn a_local_initialized_in_the_else_does_not_become_any() {
    let err = run(r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local x: int? = nil
if not_nil(x) then
    return 1
else
    local y = x
    local z: int! = y
    return z
end
"#)
    .unwrap_err();
    assert!(err.to_string().contains("nil"), "{err}");
}
#[test]
fn a_local_initialized_with_nil_is_any() {
    let res = run_results(
        r#"
local a = nil
local b: int! = a
local c
local d: int! = c
return 1
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_literal_check_narrows_the_union_it_was_read_against() {
    let res = run_results(
        r#"
local function pick(c: "1" | "2"): "1" | "2"
    if c == "1" then
        local ok = c
        local only: "1" = ok
        return only
    else
        local err = c
        local only: "2" = err
        return only
    end
end
return pick("1"), pick("2")
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "2"]);
}
#[test]
fn a_boolean_check_excludes_one_literal_at_a_time() {
    let res = run_results(
        r#"
local function only_false(c: bool?): int
    if c ~= true and c ~= nil then
        local narrowed = c
        local only: false = narrowed
        if only then
            return 1
        end
        return 0
    end
    return 1
end
local function only_true(c: bool?): int
    if c ~= false and c ~= nil then
        local narrowed = c
        local only: true = narrowed
        if only then
            return 0
        end
        return 1
    end
    return 1
end
return only_false(false), only_false(true), only_false(nil),
    only_true(true), only_true(false), only_true(nil)
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["0", "1", "1", "0", "1", "1"]);
}
#[test]
fn a_check_against_nil_says_nothing_when_a_metatable_could_answer_it() {
    let err = run(r#"
local function takes_nil(v: nil): nil
    return v
end
local t: table? = {}
if t == nil then
    return takes_nil(t)
end
return "table"
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
#[test]
fn the_else_of_an_or_reads_both_sides_the_other_way_round() {
    let res = run_results(
        r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local x: int? = nil
local y: int? = nil
if not_nil(x) or not_nil(y) then
    return 1
else
    local a: nil = x
    local b: nil = y
    return 0
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["0"]);
}
#[test]
fn a_nil_check_or_read_the_other_way_round_narrows_both() {
    let res = run_results(
        r#"
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
local y: int? = 1
if x == nil or y == nil then
    return "nil"
else
    local a: int! = x
    local b: int! = y
    return "int"
end
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["int"]);
}
#[test]
fn an_or_that_holds_proves_neither_side() {
    let err = run(r#"
local function takes_int(v: int!): int
    return v
end
local x: int? = 1
local y: int? = 1
if x == nil or y == nil then
    return takes_int(x)
end
return 0
"#)
    .unwrap_err();
    assert!(!err.to_string().is_empty(), "{err}");
}
