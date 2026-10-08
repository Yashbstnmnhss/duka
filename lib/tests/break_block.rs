//! `break` as a block exit: `break` yields the value of a `do`-expression,
//! `return` always leaves the nearest function.

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run_last, to_ir};

#[test]
fn break_is_the_block_value() {
    assert_eq!(
        run_last("return do break 1 end").unwrap(),
        RuntimeValue::Int(1)
    );
}

#[test]
fn break_midway_wins_over_the_tail() {
    assert_eq!(
        run_last("return do if 1 < 2 then break 1 end break 2 end").unwrap(),
        RuntimeValue::Int(1)
    );
}

#[test]
fn unvisited_break_slot_reads_nil() {
    assert_eq!(
        run_last("return do if 1 > 2 then break 1 end end").unwrap(),
        RuntimeValue::Nil
    );
}

#[test]
fn unvisited_break_slot_is_not_a_leaked_register() {
    // `__blk*` is initialized to nil, so a path that never breaks cannot pick
    // up whatever the dead expression left behind in the slot.
    assert_eq!(
        run_last(
            r#"
            local x = "dirty"
            return do if 1 > 2 then break x end end
            "#
        )
        .unwrap(),
        RuntimeValue::Nil
    );
}

#[test]
fn return_inside_a_do_block_leaves_the_function() {
    assert_eq!(
        run_last("return do return 5 end").unwrap(),
        RuntimeValue::Int(5)
    );
}

#[test]
fn return_midway_skips_the_rest_of_the_do_block() {
    let r = run_last(
        r#"
        global log = ""
        local function f()
            do
                log = log .. "A"
                if true then return 42 end
                log = log .. "B"
            end
            return 7
        end
        local v = f()
        return log .. "/" .. to_string(v)
        "#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "A/42");
}

#[test]
fn a_do_block_without_break_is_an_error() {
    // an *empty* `do` is still folded to `nil` before codegen
    assert_eq!(run_last("return do end").unwrap(), RuntimeValue::Nil);
    assert!(run_last("return do local x = 1 end").is_err());
}

#[test]
fn break_with_a_value_inside_a_loop_is_an_error() {
    assert!(run_last("for i = 1, 3 do break i end").is_err());
    assert!(run_last("while true do break 1 end").is_err());
    assert!(
        run_last(
            r#"
            for k, v in pairs({1}) do
                break k
            end
            "#
        )
        .is_err()
    );
}

#[test]
fn bare_break_still_leaves_a_loop() {
    let r = run_last(
        r#"
        local total = 0
        for i = 1, 100 do
            if i > 3 then break end
            total = total + i
        end
        return total
        "#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn break_carries_several_values() {
    let r = run_last(
        r#"
        local a, b, c = do break 1, 2, 3 end
        return a + b * 10 + c * 100
        "#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(321));
}

#[test]
fn nested_do_blocks_keep_their_own_slots() {
    assert_eq!(
        run_last("return do local x = do break 5 end break x + 1 end").unwrap(),
        RuntimeValue::Int(6)
    );
}

#[test]
fn a_do_expression_inside_a_loop_still_takes_the_break() {
    // the `for` is not what the `break` belongs to, the `do` nested in it is
    let r = run_last(
        r#"
        local sum = 0
        for i = 1, 3 do
            sum = sum + (do break i end)
        end
        return sum
        "#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn a_loop_inside_a_do_block_keeps_the_bare_break() {
    // the `break` with no value leaves the `for`, the one after it leaves the `do`
    assert_eq!(
        run_last("return do for i = 1, 3 do break end break 9 end").unwrap(),
        RuntimeValue::Int(9)
    );
}

#[test]
fn continue_in_a_do_expression_is_an_error() {
    assert!(run_last("return do continue end").is_err());
    // but a loop around it makes the same `continue` fine
    assert_eq!(
        run_last("local n = 0\nfor i = 1, 4 do do continue end n = n + 1 end\nreturn n").unwrap(),
        RuntimeValue::Int(0)
    );
}

#[test]
fn the_widest_break_fixes_the_slot_count() {
    // the two `break`s disagree, the wider one decides: the narrower leaves the
    // second slot as the nil it was initialized to
    assert_eq!(
        run_last("local a, b = do if true then break 1, 2, 3 end break 7 end\nreturn a * 100 + b")
            .unwrap(),
        RuntimeValue::Int(102)
    );
}

#[test]
fn a_closure_inherits_no_body_to_leave() {
    // a closure is its own function: what it sits inside cannot lend it a
    // `break` target, or a `continue`
    for src in [
        "return do local f = function() break end break 1 end",
        "return do local f = function() break 1 end break 1 end",
        "for i = 1, 3 do local f = function() break end end",
        "for i = 1, 3 do local f = function() continue end end",
    ] {
        assert!(run_last(src).is_err(), "{src} should not compile");
    }
    // a `return` inside the same closure is of course fine
    assert_eq!(
        run_last("for i = 1, 3 do local f = function() return 1 end if i > 2 then break end end\nreturn 2")
            .unwrap(),
        RuntimeValue::Int(2)
    );
}

#[test]
fn a_do_block_in_a_loop_does_not_grow_the_frame() {
    // the slots are locals of the block, so the second pass through the loop
    // reuses the same registers instead of stacking a new set every time
    let src = r#"
        local total = 0
        for i = 1, 40 do
            local a, b = do break i, i + 1 end
            total = total + a + b
        end
        return total
        "#;
    assert_eq!(to_ir(src).unwrap().reg_lifetime.count, 11);
    assert_eq!(run_last(src).unwrap(), RuntimeValue::Int(1680));
}

#[test]
fn nested_do_blocks_inside_a_loop_reuse_their_registers() {
    let src = r#"
        local total = 0
        for i = 1, 40 do
            local ten = do break i * 10 end
            local outer = do
                if ten > 100 then break ten end
                break ten + 1
            end
            total = total + outer
        end
        return total
        "#;
    assert_eq!(to_ir(src).unwrap().reg_lifetime.count, 12);
    assert_eq!(run_last(src).unwrap(), RuntimeValue::Int(8210));
}

#[test]
fn match_as_an_expression_brings_its_own_block() {
    // no `do` written here: a `match` in expression position is already a
    // block, and its arms hand it their value with `return`
    let r = run_last(
        r#"
        local function pick(t)
            return match t then
                "string" -> "s";
                "int" -> "i";
                else return "?"
            end
        end
        return pick("int") .. pick("string") .. pick(true)
        "#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "is?");
}
