//! `where` and `ensure` on a function value.

use duka_lib::harness::run;

#[test]
fn a_where_a_value_carries_is_discharged_at_the_call() {
    run(r#"
object Point
    x: int = 0
end
function shift<T>(offset: T, by: int): int?
where
    T: Point
    return offset.x + by
end
local f = shift
return f(Point.new(), 1)
"#)
    .unwrap();
}

#[test]
fn a_where_a_value_carries_still_rejects() {
    assert!(
        run(r#"
object Point
    x: int = 0
end
function shift<T>(offset: T, by: int): int?
where
    T: Point
    return offset.x + by
end
local f = shift
f(1, 1)
"#)
        .is_err()
    );
}

#[test]
fn a_function_value_meets_a_signature_that_asks_the_same() {
    run(r#"
object Point
    x: int = 0
end
function need<T>(v: T): int?
where
    T: Point
    return 1
end
local f = need
local g: type(f) = need
return g(Point.new())
"#)
    .unwrap();
}

#[test]
fn a_function_value_without_the_where_does_not_meet_it() {
    assert!(
        run(r#"
object Point
    x: int = 0
end
function need<T>(v: T): int?
where
    T: Point
    return 1
end
function plain<T>(v: T): int?
    return 1
end
local f = need
local g: type(f) = plain
return 1
"#)
        .is_err()
    );
}
