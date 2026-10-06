//! `where` clause tests.
//!
//! Three kinds of clause, told apart by what they are rather than by where they
//! are: a bound has a name and a `:`, a binding says `type`, and a concept is
//! anything else.
//!
//! The list has no terminator of its own. It sits between the signature and the
//! body and ends where the clauses stop, which is what a `where` in a query does
//! too -- so `where` is followed by the body, and the body's own `end` is the only
//! one in sight.

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

/// A binding introduces a type name. `: V` in the return annotation names it, so
/// the binding has to be read before the signature is built, not after.
#[test]
fn a_binding_names_the_return_type() {
    let res = run_results(
        r#"
function pick<T>(x: T): V
    where type V = T
    return x
end
local a: pick<int> = 1
local b: pick<string> = "s"
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

/// The binding is a type, not a synonym for a variable, so it composes: an
/// `array` of it is an array of what it was bound to.
#[test]
fn a_binding_composes_with_a_type_constructor() {
    run_results(
        r#"
function wrap<T>(x: T): Boxed
    where type Boxed = array<T>
    return x
end
local a: wrap<int> = nil
return a
"#,
    )
    .unwrap();
}

/// Later clauses see earlier ones. The list is read top to bottom and is not
/// reordered, which is the only reason this can refer to `V` at all.
#[test]
fn a_clause_sees_the_one_above_it() {
    run_results(
        r#"
function two<T>(x: T): V
where
    type V = T,
    type W = array<V>
    
    return x
end
local a: two<int> = nil
return a
"#,
    )
    .unwrap();
}

/// A concept is read for its truth, and `false` is the answer that fails.
#[test]
fn a_concept_that_holds_is_accepted() {
    run_results(
        r#"
function sized<T>(x: T): T
where
    int == int
    return x
end
local a: sized<int> = 1
return a
"#,
    )
    .unwrap();
}

/// A concept that answers `false` about types nothing is waiting on is a failure
/// here, at the declaration, rather than something to defer to the call site.
#[test]
fn a_concept_that_fails_is_reported() {
    let err = run(r#"
function sized<T>(x: T): T
where
    int == string
    return x
end
local a: sized<int> = 1
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}

/// A concept that mentions a type parameter cannot be read at the declaration --
/// nothing is known yet -- so it is an obligation and not a failure. `T == int`
/// answers `false` when `T` is still a variable, and calling that a failure would
/// reject every constraint worth writing.
#[test]
fn a_concept_about_a_type_parameter_is_deferred() {
    run_results(
        r#"
function sized<T>(x: T): T
where
    T == int
    return x
end
local a: sized<int> = 1
local b: sized<string> = "s"
return a, b
"#,
    )
    .unwrap();
}

/// A concept is read with Duka's truthiness rather than by comparing against
/// `bool`, because a concept is written over types and almost no type is a
/// boolean: `int` is not `false`, so it holds.
#[test]
fn a_concept_reads_truthiness_rather_than_a_comparison() {
    run_results(
        r#"
function anything<T>(x: T): T
where
    int
    return x
end
local a: anything<int> = 1
return a
"#,
    )
    .unwrap();
}

/// A bound is a subtype statement and the declaration accepts it: the argument
/// is not known here, so this is where it can only be recorded.
#[test]
fn a_bound_is_accepted_at_the_declaration() {
    run_results(
        r#"
object Point
end
function at<T>(x: T): T
where
    T: Point
    return x
end
local a: at<int> = 1
return a
"#,
    )
    .unwrap();
}

/// All three in one list, in the order they are read.
#[test]
fn the_three_kinds_of_clause_together() {
    run_results(
        r#"
object Point
end
function translate<T, U>(a: T, b: U): V
where
    U: Point,
    int == int,
    type V = array<T>
    return a
end
local x: translate<int, Point> = nil
return x
"#,
    )
    .unwrap();
}

/// A function with no `where` is unaffected.
#[test]
fn a_function_without_a_where_is_untouched() {
    let res = run_results(
        r#"
function id<T>(x: T): T
    return x
end
local a: id<int> = 1
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
