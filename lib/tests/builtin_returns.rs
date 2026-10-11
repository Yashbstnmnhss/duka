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
