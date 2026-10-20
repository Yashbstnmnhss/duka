use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}
#[test]
fn a_loop_inside_a_type_function_does_not_see_the_callers_type_parameters() {
    let iterated = run(r#"
type function Head(Cs)
    for C in Cs do return C end
    return false
end
function f<T>(x: T): T where Head([SameAs(T, int)]) return x end
return f(1)
"#);
    assert!(iterated.is_err(), "iterating should not have answered");

    let passed = run_results(
        r#"
type function Head(C)
    return C
end
function f<T>(x: T): T where Head(SameAs(T, int)) return x end
return f(1)
"#,
    )
    .unwrap();
    assert_eq!(strs(&passed), ["1"]);
}
#[test]
fn a_requirement_reads_a_concept_that_answers() {
    let res = run_results(
        r#"
function as_int<T>(x: T): T
    where SameAs(T, int)
    return x
end
local a = as_int(1)
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}
#[test]
fn a_requirement_refuses_a_call_that_breaks_the_concept() {
    let err = run(r#"
function as_int<T>(x: T): T
    where SameAs(T, int)
    return x
end
local a = as_int("s")
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
#[test]
fn not_inverts_a_concept() {
    let res = run_results(
        r#"
function not_int<T>(x: T): T
    where Not(T, SameAs(T, int))
    return x
end
local a = not_int("s")
local b = not_int(1.5)
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["s", "1.5"]);
}

#[test]
fn not_refuses_what_it_inverts() {
    let err = run(r#"
function not_int<T>(x: T): T
    where Not(T, SameAs(T, int))
    return x
end
local a = not_int(1)
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
#[test]
fn any_is_satisfied_by_one_of_its_concepts() {
    let res = run_results(
        r#"
function int_or_string<T>(x: T): T
    where Any(SameAs(T, int), SameAs(T, string))
    return x
end
local a = int_or_string(1)
local b = int_or_string("s")
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "s"]);
}

#[test]
fn any_is_not_satisfied_by_none_of_them() {
    let err = run(r#"
function int_or_string<T>(x: T): T
    where Any(SameAs(T, int), SameAs(T, string))
    return x
end
local a = int_or_string(true)
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
#[test]
fn all_is_satisfied_by_every_concept() {
    let res = run_results(
        r#"
function integral_only<T>(x: T): T
    where All(Integral(T), Not(T, SameAs(T, float)))
    return x
end
local a = integral_only(1)
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn all_is_not_satisfied_by_one_of_them_failing() {
    let err = run(r#"
function integral_only<T>(x: T): T
    where All(Integral(T), Not(T, SameAs(T, int)))
    return x
end
local a = integral_only(1)
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
#[test]
fn combinators_nest() {
    let res = run_results(
        r#"
function neither_int_nor_string<T>(x: T): T
    where Not(T, Any(SameAs(T, int), SameAs(T, string)))
    return x
end
local a = neither_int_nor_string(true)
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool"]);
}

#[test]
fn a_nested_combination_still_refuses() {
    let err = run(r#"
function neither_int_nor_string<T>(x: T): T
    where Not(T, Any(SameAs(T, int), SameAs(T, string)))
    return x
end
local a = neither_int_nor_string("s")
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
#[test]
fn is_sub_of_reads_the_argument_before_the_bound() {
    run_results(
        r#"
object Animal
end
object Dog extends Animal
end
object Rock
end

function as_animal<T>(x: T): T
    where IsSubOf(T, Animal)
    return x
end
local a = as_animal(Dog.new())
return 1
"#,
    )
    .unwrap();
}

#[test]
fn has_reads_a_member_of_a_class() {
    let res = run_results(
        r#"
object A
    x = 1
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local a = as_has_x(A.new())
return a.x
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn has_walks_up_the_class_chain() {
    let res = run_results(
        r#"
object Animal
    x = 1
end
object Dog extends Animal
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local d = as_has_x(Dog.new())
return d.x
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn has_reads_a_method() {
    let res = run_results(
        r#"
object A
    function :x(): int
        return 1
    end
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local a = as_has_x(A.new())
return a:x()
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn has_holds_when_one_arm_of_a_union_has_it() {
    let res = run_results(
        r#"
object Animal
    x = 1
end
object Rock
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local v: Animal | Rock = Animal.new()
return as_has_x(v)
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["table"]);
}

#[test]
fn has_refuses_a_union_without_it() {
    let err = run(r#"
object Rock
end
object Pebble
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local v: Pebble | Rock = Rock.new()
return as_has_x(v)
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}

#[test]
fn has_refuses_a_class_without_the_member() {
    let err = run(r#"
object Rock
end
function as_has_x<T>(v: T): T
    where Has(T, "x")
    return v
end
local a = as_has_x(Rock.new())
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}

#[test]
fn is_sub_of_refuses_a_class_outside_the_hierarchy() {
    let err = run(r#"
object Animal
end
object Dog extends Animal
end
object Rock
end

function as_animal<T>(x: T): T
    where IsSubOf(T, Animal)
    return x
end
local a = as_animal(Rock.new())
return 1
"#)
    .unwrap_err();
    assert!(err.to_string().contains("requirement"), "{err}");
}
