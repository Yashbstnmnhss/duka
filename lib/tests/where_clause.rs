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

/// A concept about a type parameter is an obligation, and the call site is where
/// it is discharged. `T == int` answers `false` when `T` is still a variable, so
/// a call that solves `T` to `string` is the first place the question can be put.
#[test]
fn a_concept_is_discharged_at_the_call_that_satisfies_it() {
    run_results(
        r#"
function only_int<T>(x: T): T
where
    T == int
    return x
end
local a = only_int(1)
return a
"#,
    )
    .unwrap();
}

/// The other side of the same obligation: the call solves `T` to something the
/// concept rejects, and that is an error here rather than a silent pass.
///
/// It has to be a real call. `local a: only_int<string>` is an annotation, and an
/// annotation is not a call site, so nothing is solved and there is nothing to
/// discharge against.
#[test]
fn a_concept_is_discharged_against_the_type_argument() {
    let err = run(r#"
function only_int<T>(x: T): T
where
    T == int
    return x
end
local a = only_int("s")
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}

/// A concept that is still undecided after substitution is left alone: refusing a
/// call over a variable nobody passed would be worse than letting it through.
#[test]
fn a_concept_that_is_still_undecided_is_left_alone() {
    run_results(
        r#"
function only_in_first<T, U>(a: T, b: U): T
where
    T == int
    return a
end
local a = only_in_first(1, "s")
return a
"#,
    )
    .unwrap();
}

/// A bound is checked against the type argument at the call, which is the first
/// place there is an argument to be wrong about.
#[test]
fn a_bound_is_discharged_against_the_type_argument() {
    let err = run(r#"
function num<T>(x: T): T
where
    T: int | string
    return x
end
local a = num(true)
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("does not satisfy"), "{err}");
}

/// The same bound, satisfied. A bound is read without `?` -- `T: int | string`
/// means exactly that and not "that or nil" -- so `int` answers it.
#[test]
fn a_bound_that_holds_is_accepted() {
    run_results(
        r#"
function num<T>(x: T): T
where
    T: int | string
    return x
end
local a = num(1)
local b = num("s")
return a, b
"#,
    )
    .unwrap();
}

/// The point of a bound in a generic body.
///
/// `offset.x` is a member read on a type parameter. Without the bound, `T` has no
/// shape to read from and the read is `any`, which every return annotation
/// accepts. With the bound, the read is resolved against `Point` and is a
/// `string`, which `: int` does not accept.
///
/// The two tests below are the same program apart from the `where`, so the error
/// can only have come from the bound. An annotated property is what makes this
/// work: `x = "str"` with no annotation leaves the member `any`, and then nothing
/// would disagree with anything.
#[test]
/// The point of a bound in a generic body.
///
/// `offset.x` is a member read on a type parameter. Without the bound, `T` has no
/// shape to read from and the read is `any`, which every return annotation
/// accepts. With the bound, the read is resolved against `Point` and is a
/// `string`, which `: int` does not accept.
///
/// The two tests below are the same program apart from the `where`, so the error
/// can only have come from the bound. An annotated property is what makes this
/// work: `x = "str"` with no annotation leaves the member `any`, and then nothing
/// would disagree with anything.
#[test]
fn a_bound_is_what_a_member_read_in_the_body_is_checked_against() {
    let err = run(r#"
object Point
    x: string = "not a number"
end
function read<T>(offset: T): int
where
    T: Point
    return offset.x
end
local a: read<Point> = nil
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
}

/// The same program with the `where` removed. The member read is on a bare type
/// parameter, so it is `any` and the return annotation has nothing to disagree
/// with. This is what makes the previous test a proof rather than a coincidence.
#[test]
fn without_a_bound_the_same_member_read_checks_nothing() {
    run_results(
        r#"
object Point
    x: string = "not a number"
end
function read<T>(offset: T): int
    return offset.x
end
local a: read<Point> = nil
return a
"#,
    )
    .unwrap();
}

/// The shape a bound is normally written for, where the two agree.
#[test]
fn a_bound_lets_a_member_read_in_the_body_resolve() {
    run_results(
        r#"
object Point
    x: int = 0
end
function shift<T>(offset: T, by: int): int
where
    T: Point
    return offset.x + by
end
local a = shift(Point.new(), 1)
return a
"#,
    )
    .unwrap();
}

/// The same read of a member the bound does not have is still `any` rather than an
/// error: a bound is a subtype statement, not a closed record, and the language
/// does not have a "no such member" error for a value whose type it only knows
/// partially.
#[test]
fn a_bound_does_not_turn_into_a_closed_record() {
    run_results(
        r#"
object Point
    x = 0
end
function probe<T>(offset: T): int
where
    T: Point
    return offset.y
end
local a: probe<Point> = nil
return a
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
