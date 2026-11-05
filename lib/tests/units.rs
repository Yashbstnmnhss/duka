//! Units as type tables with metatable operators.

use duka_lib::harness::run;

#[test]
fn a_unit_divides_into_a_derived_unit() {
    run(r#"
type Meter = Unit("m")
type Second = Unit("s")
type Velocity = WithUnit(float, (Meter / Second))
function speed(v: Velocity): Velocity?
    return v
end
return speed(as.<Velocity>(1.0))
"#)
    .unwrap();
}

#[test]
fn a_unit_power_scales_the_exponents() {
    run(r#"
type Meter = Unit("m")
type Area = WithUnit(float, (Meter ^ 2))
return as.<Area>(2.0)
"#)
    .unwrap();
}

#[test]
fn a_unit_power_needs_a_whole_constant() {
    assert!(
        run(r#"
type Meter = Unit("m")
type Nope = WithUnit(float, (Meter ^ 2.5))
return 1
"#)
        .is_err()
    );
}

#[test]
fn an_operator_without_a_metatable_is_refused() {
    assert!(
        run(r#"
type Nope = WithUnit(float, (1 / 2))
return 1
"#)
        .is_err()
    );
}

#[test]
fn a_derived_unit_nests() {
    run(r#"
type Meter = Unit("m")
type Second = Unit("s")
type Accel = WithUnit(float, ((Meter / Second) / Second))
local a: Accel = as.<Accel>(9.8)
return a
"#)
    .unwrap();
}

#[test]
fn a_unit_chain_keeps_combining() {
    run(r#"
type Meter = Unit("m")
type Second = Unit("s")
type Force = WithUnit(float, (((Meter / Second) ^ 2) * Second))
local a: Force = as.<Force>(1.0)
return a
"#)
    .unwrap();
}

#[test]
fn a_dimensionless_text_handed_to_with_unit_leaves_the_base() {
    run(r#"
type Plain = WithUnit(float, Unit("1"))
return as.<Plain>(1.0)
"#)
    .unwrap();
}
