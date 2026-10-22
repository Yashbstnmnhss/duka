//! Generic function type inference (L2) tests

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn explicit_typeargs_infers_return() {
    let res = run_results(
        r#"
function id<T>(x: T): T
    return x
end
local a: int = id.<int>(1)
local b: string = id.<string>("s")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

#[test]
fn inferred_from_args() {
    let res = run_results(
        r#"
function pick<T>(x: T): T
    return x
end
local a: int = pick(1)
local b: string = pick("s")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

#[test]
fn bound_accepts_valid() {
    let res = run_results(
        r#"
function bnd<T: int>(x: T): T
    return x
end
local a: int = bnd.<int>(1)
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn bound_rejects_invalid() {
    let err = run(r#"
function bnd<T: int>(x: T): T
    return x
end
local a: string = bnd.<string>("x")
return a
"#)
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("does not satisfy its bound"), "{message}");
    assert!(message.contains("'T'"), "{message}");
    assert!(!message.contains("incompatible"), "{message}");
}

#[test]
fn explicit_typeargs_array_element() {
    let res = run_results(
        r#"
function first<T>(x: array<T>): T
    return x[0]
end
local a: int = first.<int>([1, 2])
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

/// `|` and `&` are the type grammar's own operators, so a type position that
/// uses them stays a type. If this regressed into reading the whole annotation
/// as an expression, `|` would become a boolean or and the annotation would
/// stop meaning "either of these".
#[test]
fn a_union_in_a_type_position_is_still_a_union() {
    let res = run_results(
        r#"
local a: int | string = 1
local b: int | string = "x"
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "x"]);
}

#[test]
fn a_type_parameter_bound_may_be_a_union() {
    let res = run_results(
        r#"
function f<T: int | string>(x: T): T
    return x
end
local a: int = f.<int>(1)
local b: string = f.<string>("x")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "x"]);
}

/// A pack parameter takes the remaining type arguments as a list, and that list
/// is an ordinary tuple of types.
///
/// Two arguments go in and the tuple has both, so `Ts` is `[string, int]` and not
/// `string` -- the pack does not pick the first one and leave the rest. This is
/// also what makes the arity ceiling matter: with two arguments against one
/// declared parameter, a solver that counted the declaration as the maximum
/// rejected the call outright.
#[test]
fn a_pack_parameter_takes_the_rest_of_the_type_arguments_as_a_list() {
    // The annotation is the assertion: the two annotations below are the exact
    // tuple the call produces, and neither of them names a single element, so a
    // pack that kept only the first argument would be rejected here.
    //
    // What the function returns at run time is beside the point -- `x[0]` on the
    // empty array it is handed is nil -- so the check is that the program
    // type-checks, which is what a pack changes.
    run_results(
        r#"
function first<...Ts>(x: array<Ts>): Ts
    return x[0]
end
local a: [string, int] = first.<string, int>([])
return a
"#,
    )
    .unwrap();

    // And the shape that must not type-check: the pack is not the first argument.
    let err = run(r#"
function first<...Ts>(x: array<Ts>): Ts
    return x[0]
end
local a: string = first.<string, int>([])
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("[string, int]"), "{err}");
}

/// A pack that is handed nothing is an empty list, not a one-element list and not
/// an error. `<...Ts>` alone means "all of the type arguments", and there being
/// none is the ordinary case for a call that writes none.
#[test]
fn a_pack_parameter_with_no_type_arguments_is_an_empty_list() {
    run_results(
        r#"
function first<...Ts>(x: array<Ts>): Ts
    return x[0]
end
local a: [] = first([])
return a
"#,
    )
    .unwrap();
}

/// A pack after a fixed parameter takes the rest, so `<T, ...Ts>` binds one type
/// to `T` and the remainder to `Ts`.
///
/// The fixed count is the floor and does not move when a pack is present: the pack
/// relaxes the ceiling, not the requirement.
#[test]
fn a_fixed_parameter_before_a_pack_binds_one_and_the_rest_to_the_pack() {
    run_results(
        r#"
function pick<T, ...Ts>(x: T, y: array<Ts>): [T, Ts]
    return x, y
end
local a: [string, [int, float]] = pick.<string, int, float>("s", [])
return a
"#,
    )
    .unwrap();
}

/// Without the `...`, `Ts` names one type and the same call has a second argument
/// it cannot place. This is what the previous test is measured against.
#[test]
fn a_single_parameter_does_not_take_a_list_of_type_arguments() {
    run(r#"
function first<Ts>(x: array<Ts>): Ts
    return x[0]
end
local a = first.<int, string>(nil)
return a
"#)
    .unwrap_err();
}

/// A builtin that declares a type parameter solves for it at the call site,
/// even though it is reached through a module and has no name of its own a
/// call site could look it up by.
#[test]
fn a_builtin_solves_its_type_parameter_from_the_argument() {
    let res = run_results(
        r#"
local a: array<int> = [1, 2, 3]
local x = array.pop(a)
local y: int = x
return y
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["3"]);
}

/// The parameter is nilable by default, and the `nil` is not an element type:
/// the array says what it holds, not that it may be nothing.
#[test]
fn a_nilable_argument_still_says_what_it_holds() {
    run_results(
        r#"
local a: array<string> = ["x"]
local x = array.pop(a)
local y: string = x
return y
"#,
    )
    .unwrap();
}
