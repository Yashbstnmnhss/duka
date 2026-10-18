//! `break ::name::` and `continue ::name::`: a name picks the body to leave
//! instead of the innermost one.
//!
//! The label is written after the keyword -- `for ::l::`, `while ::l::`,
//! `do ::l::` -- and the same name on the `break` or `continue` reaches it,
//! however many bodies sit in between.

use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_last;

fn count(src: &str) -> i64 {
    match run_last(src).unwrap() {
        RuntimeValue::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

#[test]
fn named_break_leaves_an_outer_for() {
    assert_eq!(
        count(
            r#"
            local n = 0
            for ::outer:: i = 1, 3 do
                for j = 1, 3 do
                    n = n + 1
                    break ::outer::
                end
            end
            return n
            "#
        ),
        1
    );
}

#[test]
fn named_break_leaves_an_outer_while() {
    assert_eq!(
        count(
            r#"
            local n = 0
            while ::outer:: n < 5 do
                n = n + 1
                local k = 0
                while k < 3 do
                    k = k + 1
                    break ::outer::
                end
            end
            return n
            "#
        ),
        1
    );
}

#[test]
fn named_continue_restarts_an_outer_for() {
    // a plain `continue` here would only skip the inner iteration, so the sum
    // would reach 6
    assert_eq!(
        count(
            r#"
            local sum = 0
            for ::c:: i = 1, 3 do
                for j = 1, 3 do
                    continue ::c::
                end
                sum = sum + i
            end
            return sum
            "#
        ),
        0
    );
}

#[test]
fn an_unnamed_break_still_takes_the_innermost_body() {
    assert_eq!(
        count(
            r#"
            local n = 0
            for ::outer:: i = 1, 3 do
                for j = 1, 3 do
                    n = n + 1
                    break
                end
            end
            return n
            "#
        ),
        3
    );
}

#[test]
fn a_named_do_still_takes_an_unnamed_break() {
    // the name decides which bodies a `break` may reach past, not whether an
    // unnamed one is still its own
    assert_eq!(
        run_last("return do ::outer:: break 42 end").unwrap(),
        RuntimeValue::Int(42)
    );
}

#[test]
fn a_named_break_carries_a_value_out_of_a_nested_do() {
    assert_eq!(
        run_last("return do ::o:: do ::i:: break ::o:: 42 end break 0 end").unwrap(),
        RuntimeValue::Int(42)
    );
}

#[test]
fn a_named_break_carries_a_value_out_of_a_loop() {
    assert_eq!(
        run_last(
            r#"
            return do ::o::
                for i = 1, 3 do
                    break ::o:: i * 10
                end
                break 0
            end
            "#
        )
        .unwrap(),
        RuntimeValue::Int(10)
    );
}

#[test]
fn a_named_break_carries_several_values() {
    let got = run_last(
        "local a, b = do ::o:: do ::i:: break ::o:: 7, 8 end break 0, 0 end return a + b * 10",
    )
    .unwrap();
    assert_eq!(got, RuntimeValue::Int(87));
}

#[test]
fn the_widest_break_fixes_the_slots_whatever_it_is_named() {
    // the two-value `break ::o::` is what makes the block worth two
    assert_eq!(
        run_last(
            r#"
            local a, b = do ::o::
                if 1 < 2 then
                    break ::o:: 5, 6
                end
                break 1
            end
            return a * 100 + b
            "#,
        )
        .unwrap(),
        RuntimeValue::Int(506)
    );
}

#[test]
fn a_lone_break_pads_the_slots_a_wider_named_break_declares() {
    // the same wider `break` never runs, but it still declares the arity, so
    // the one-value `break` has to leave nil in the second slot
    assert_eq!(
        run_last(
            r#"
            local a, b = do ::o::
                if 1 > 2 then
                    break ::o:: 5, 6
                end
                break 1
            end
            return a
            "#,
        )
        .unwrap(),
        RuntimeValue::Int(1)
    );
}

#[test]
fn a_padded_slot_reads_nil_rather_than_a_leftover() {
    assert_eq!(
        run_last(
            r#"
            local a, b = do ::o::
                if 1 > 2 then
                    break ::o:: 5, 6
                end
                break 1
            end
            return b
            "#,
        )
        .unwrap(),
        RuntimeValue::Nil
    );
}

#[test]
fn the_value_of_a_named_break_is_what_the_block_is_worth() {
    assert_eq!(
        run_last(r#"local x: int = do ::o:: break ::o:: 5 end return x"#).unwrap(),
        RuntimeValue::Int(5)
    );
}

#[test]
fn the_value_of_a_named_break_is_typed() {
    assert!(run_last(r#"local x: string = do ::o:: break ::o:: 5 end return x"#).is_err());
}

#[test]
fn an_unknown_label_is_an_error() {
    assert!(run_last("for i = 1, 2 do break ::no_such_label:: end").is_err());
}

#[test]
fn a_label_out_of_scope_is_an_error() {
    // the `do` ends before the `break` is reached, so its name is gone by then
    assert!(run_last("for i = 1, 2 do do ::gone:: end break ::gone:: end").is_err());
}

#[test]
fn continue_cannot_target_a_do() {
    // restarting a `do` would re-enter it
    assert!(run_last("do ::d:: while true do continue ::d:: end end").is_err());
}

#[test]
fn a_break_value_cannot_target_a_loop() {
    // a loop is a statement, so it has nothing to hand the value back to
    assert!(run_last("for ::l:: i = 1, 2 do break ::l:: 1 end end").is_err());
}

#[test]
fn a_label_cannot_be_reached_from_a_nested_function() {
    assert!(run_last("for ::l:: i = 1, 2 do local f = function() break ::l:: end end").is_err());
}
