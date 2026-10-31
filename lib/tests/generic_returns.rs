//! A generic return is a call-time answer.
//!
//! The annotation is read again at each call, with the variables solved and the
//! `where` bindings of that call in scope. Reading it once at the declaration
//! would freeze the placeholders the signature was written with and lose what
//! `const` captured.

use duka_lib::harness::run;

/// A return annotation that calls a type function sees the solved variables,
/// so `const T` makes the literal part of the question.
#[test]
fn a_return_that_calls_a_type_function_is_read_at_the_call() {
    run(r##"
type function Pick(p, t)
    if SameAs(p, "#") then
        return int
    end
    return t
end

function f<const T>(x: T): Pick(T, T)
    return x
end

local n: int = f("#")
"##)
    .unwrap();
}

/// The same answer through a `where` binding, which is evaluated at the call
/// too: the name means what this call solved it to.
#[test]
fn a_where_binding_is_evaluated_at_the_call() {
    run(r##"
type function Pick(p, t)
    if SameAs(p, "#") then
        return int
    end
    return t
end

function g<const T>(x: T): R
    where type R = Pick(T, T)
    return x
end

local n: int = g("#")
"##)
    .unwrap();
}

/// The call-time reading is exact, not merely permissive: the other answer is
/// refused.
#[test]
fn the_call_time_answer_is_checked() {
    let err = run(r##"
type function Pick(p, t)
    if SameAs(p, "#") then
        return int
    end
    return t
end

function f<const T>(x: T): Pick(T, T)
    return x
end

local s: string = f("#")
return s
"##)
    .unwrap_err();
    assert!(err.to_string().contains("int"), "{err}");
}

/// A literal index counts from the tail in a type position, the way the
/// runtime does.
#[test]
fn a_type_position_indexes_a_tuple_from_the_tail() {
    run(r#"
type function At(i, vals)
    return vals[i]
end
local first: At(0, [string, int]) = "x"
local last: At(-1, [string, int]) = 1
"#)
    .unwrap();
}
