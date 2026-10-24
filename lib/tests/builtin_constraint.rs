//! What a builtin says about its own call
//!
//! A builtin may write a `where` list and a set of constraints in its
//! attribute. Neither is a type, so neither travels in the signature; both are
//! read at the call that has to discharge them.

use duka_lib::harness::run;

/// A constraint is a fact about an argument, and the fact holds where the
/// call's answer is read as true.
///
/// `is_error` answers whether the second value it was given is an error
/// message, so where that answer holds, the message is a string and not
/// nothing.
#[test]
fn a_builtin_constraint_narrows_the_argument_it_is_about() {
    run(r#"
local function takes_string(v: string!): string
    return v
end
local ok = false
local err: string? = nil
if is_error(ok, err) then
    return takes_string(err)
end
return ""
"#)
    .unwrap();
}

/// The fact is about where the answer holds: outside it, nothing is known.
#[test]
fn a_builtin_constraint_does_not_reach_outside_the_answer() {
    let err = run(r#"
local function takes_string(v: string!): string
    return v
end
local ok = false
local err: string? = nil
if is_error(ok, err) then
end
return takes_string(err)
"#)
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("string"), "{message}");
}
