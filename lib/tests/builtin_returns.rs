//! A builtin that returns a value must say so with `returns(...)`.
//!
//! `MetaInfo::get_type` reads the declared return list verbatim, so a builtin
//! written as `Result<RuntimeValue, _>` without one is typed as producing *no*
//! values. That is invisible until the call is used as an argument, where the
//! enclosing call counts what the inner one produced and finds nothing there:
//! `assert(instanceof(c, Counter))` read as a call to `assert` with zero
//! arguments.

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run_last, run_results};

/// The builtins that answer a question and return the answer, in a spot where
/// the checker has to count the value they hand back.
const PREDICATES: &[(&str, &str)] = &[
    ("instanceof", "instanceof(1, int)"),
    ("typeof", "typeof(1)"),
    ("is_error", "is_error(1)"),
];

#[test]
fn an_iterator_builtin_answers_with_a_function_not_any() {
    let err = duka_lib::harness::run(
        r#"
local it = ipairs([1, 2, 3])
local n: int = it
return n
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("function"), "{err}");
}

#[test]
fn a_pairs_iterator_is_typed_by_the_table() {
    let err = duka_lib::harness::run(
        r#"
local t: table<string, int> = { a = 1 }
local it = pairs(t)
local n: int = it
return n
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("function"), "{err}");
}

#[test]
fn the_iterator_element_type_comes_from_the_callee() {
    duka_lib::harness::run(
        r#"
local it = ipairs([1, 2, 3])
local f: function() -> (bool, int, int) = it
return f
"#,
    )
    .unwrap();
}

#[test]
fn a_predicate_used_as_an_argument_is_still_one_argument() {
    for (name, call) in PREDICATES {
        let src = format!("return to_string({call})");
        assert!(
            duka_lib::module::from_source(
                &src,
                duka_shared::types::SourceName::Unnamed,
                Default::default()
            )
            .is_ok(),
            "`{name}` must hand a value back to the call it is written inside"
        );
    }
}

#[test]
fn a_predicate_answers_for_real() {
    assert_eq!(
        run_last("return typeof(1)").unwrap().eval_to_string(),
        "int"
    );
    assert_eq!(
        run_last("return to_string(12)").unwrap().eval_to_string(),
        "12"
    );
    assert_eq!(
        run_last("return to_number('42')").unwrap(),
        RuntimeValue::Int(42)
    );
    assert_eq!(
        run_last("return to_string(is_error(1))")
            .unwrap()
            .eval_to_string(),
        "false"
    );
    // `instanceof` reads a class table, so the target has to be one
    assert_eq!(
        run_last(
            r#"
            object Counter end
            local c = Counter.new()
            return to_string(instanceof(c, Counter))
            "#
        )
        .unwrap()
        .eval_to_string(),
        "true"
    );
}

#[test]
fn a_builtin_pack_binds_its_vararg_tail_like_a_written_one() {
    let err = duka_lib::harness::run(
        r#"
local packed = array.pack(1, 2, 3)
local not_a_string: string = packed
return not_a_string
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("[int, int, int]"), "{err}");
}

#[test]
fn unpacking_an_array_gives_each_slot_its_element_type_and_may_be_short() {
    let err = duka_lib::harness::run(
        r#"
function mk(): array<int>
    return [1, 2]
end
function f(x: string) return x end
local a, b = array.unpack(mk())
return f(a)
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("int?"), "{err}");
}

#[test]
fn unpacking_a_tuple_keeps_the_slots_it_knows() {
    let err = duka_lib::harness::run(
        r#"
function f(x: string) return x end
local a, b = array.unpack([1, "x"])
return f(a)
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("'int'"), "{err}");
}

#[test]
fn a_repeated_tail_type_is_written_with_dots() {
    duka_lib::harness::run(
        r#"
function takes(x: [...int]) return #x end
takes([1, 2, 3])
"#,
    )
    .unwrap();
    let err = duka_lib::harness::run(
        r#"
function takes(x: [...int]) return #x end
takes([1, "x"])
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("[int, string]"), "{err}");
}

#[test]
fn stdlib_returns_say_what_they_are() {
    let cases = [
        ("local x: string = math.max(1, 2, 3)", "int"),
        ("local x: string = math.max(1, 2.5)", "float"),
        ("local x: string = math.max({1, 5, 3})", "int?"),
        ("local x: string = math.min(1, 2.5)", "float"),
        ("local x: string = math.sum(1, 2, 3)", "int"),
        ("local x: string = math.factors(12)", "int?"),
        ("local x: string = math.clamp(2, 0, 3)", "float"),
        ("local x: string = string.len('abc')", "int"),
        ("local x: string = string.split('a b')", "array<string>"),
        ("local x: string = iter.to_array([1, 2])", "array<int>"),
        (
            "function f(t: table<string, int>)\n    local x: string = table.raw_get(t, 'a')\nend",
            "int",
        ),
        ("local d = os.date()\nlocal x: string = d.year", "int"),
        ("local x: int = os.get_env('DUKA_TEST_ENV')", "string?"),
        (
            "local ok, v = json.parse('{}')\nlocal x: string = ok",
            "bool",
        ),
    ];
    for (src, want) in cases {
        let err = duka_lib::harness::run(src).unwrap_err();
        assert!(err.to_string().contains(want), "{src}: {err}");
    }
}

#[test]
fn a_builtin_with_no_returns_really_does_produce_nothing() {
    // the other half of the rule: `print` returns nothing, so a call to it is
    // no argument at all and `assert` has nothing to check
    assert!(
        duka_lib::module::from_source(
            "return to_string(print('x'))",
            duka_shared::types::SourceName::Unnamed,
            Default::default(),
        )
        .is_err(),
        "`print` must not be counted as producing a value"
    );
}

#[test]
fn the_argument_count_of_an_enclosing_call_is_not_disturbed() {
    // one value in, one value out -- the shape `assert` measures
    assert_eq!(
        run_results("return try(function(a, b) return b end, typeof(1), 4)").unwrap(),
        vec![RuntimeValue::Bool(true), RuntimeValue::Int(4)],
    );
}
