//! Labels and value patterns.

use duka_lib::harness::run;

/// A pattern is checked where the value is known.
#[test]
fn a_pattern_rejects_a_literal_it_cannot_hold() {
    assert!(
        run(r#"
type Port = AndPat(GtPat(int, 0), LePat(int, 65535))
function open(p: Port): int
    return 1
end
open(70000)
"#)
        .is_err()
    );
}

#[test]
fn a_pattern_takes_a_literal_it_can_hold() {
    run(r#"
type Port = AndPat(GtPat(int, 0), LePat(int, 65535))
function open(p: Port): int
    return 1
end
return open(80)
"#)
    .unwrap();
}

/// A promise is about values, so a value nobody knows is let through.
#[test]
fn a_pattern_lets_an_unknown_value_through() {
    run(r#"
type Port = GtPat(int, 65535)
function open(p: Port): int
    return 1
end
local n = as.<int>(1)
return open(n)
"#)
    .unwrap();
}

/// Two labels are two types: an id is not a quantity.
#[test]
fn two_labels_do_not_mix() {
    assert!(
        run(r#"
type Len = WithBrand(int, "len")
type Time = WithBrand(int, "time")
function mix(a: Len, b: Time): Len
    return a
end
mix(as.<Len>(1), as.<Len>(2))
"#)
        .is_err()
    );
}

/// A label is a subtype of what it labels, so a branded value is a number.
#[test]
fn a_label_is_its_base() {
    run(r#"
type Len = WithBrand(int, "len")
function takes(n: int): int
    return 1
end
return takes(as.<Len>(1))
"#)
    .unwrap();
}

/// A nominal label has no algebra, so it does not combine.
#[test]
fn a_nominal_label_refuses_to_multiply() {
    assert!(
        run(r#"
type Len = WithBrand(int, "len")
return as.<Len>(2) * as.<Len>(3)
"#)
        .is_err()
    );
}

/// A literal takes the label of the place it is written in.
#[test]
fn a_literal_is_read_as_the_label_it_is_written_in() {
    run(r#"
type X = Unit(float, "m")
type Tt = Unit(float, "s")
local x: X = 1.0
local t: Tt = 2.0
return 1
"#)
    .unwrap();
}

/// A nominal label has no identity element, so a constant cannot become one.
#[test]
fn a_literal_does_not_become_a_nominal_brand() {
    assert!(
        run(r#"
type Id = WithBrand(int, "id")
local a: Id = 5
return 1
"#)
        .is_err()
    );
}

/// A value nobody wrote down keeps its own type: only a constant is read as
/// the label of the place it lands in.
#[test]
fn a_variable_still_needs_its_own_label() {
    assert!(
        run(r#"
type X = Unit(float, "m")
function takes(v: X): int
    return 1
end
local n = as.<float>(1.0)
return takes(n)
"#)
        .is_err()
    );
}

/// A square root halves the exponents, and only whole ones.
#[test]
fn sqrt_halves_the_unit() {
    run(r#"
type Side = Unit(float, "m")
type Area = Unit(float, "m^2")
function takesLen(a: Side): int
    return 1
end
return takesLen(math.sqrt(as.<Area>(9.0)))
"#)
    .unwrap();
}

#[test]
fn sqrt_of_a_root_unit_is_refused() {
    assert!(
        run(r#"
type Side = Unit(float, "m")
return math.sqrt(as.<Side>(9.0))
"#)
        .is_err()
    );
}

/// `floor` answers an integer with the unit it was handed.
#[test]
fn floor_keeps_the_unit() {
    run(r#"
type Side = Unit(float, "m")
type Whole = Unit(int, "m")
function takesWhole(a: Whole): int
    return 1
end
return takesWhole(math.floor(as.<Side>(2.7)))
"#)
    .unwrap();
}

/// Two units do not share a `max`.
#[test]
fn max_refuses_two_different_units() {
    assert!(
        run(r#"
type Side = Unit(float, "m")
type Time = Unit(float, "s")
return math.max(as.<Side>(1.0), as.<Time>(2.0))
"#)
        .is_err()
    );
}

/// A power multiplies the exponents, so `(m/s)^2` is `m^2/s^2`.
#[test]
fn a_power_scales_the_exponents() {
    run(r#"
type V = Unit(float, "m/s")
type Acc = Unit(float, "m^2/s^2")
function takesAcc(a: Acc): int
    return 1
end
return takesAcc(as.<V>(2.0) ^ 2)
"#)
    .unwrap();
}

/// An exponent has to be a whole constant: the group has integer exponents.
#[test]
fn a_power_needs_a_whole_constant() {
    let sources = [
        "type V = Unit(float, \"m/s\")\nreturn as.<V>(2.0) ^ as.<V>(3.0)\n",
        "type V = Unit(float, \"m/s\")\nreturn as.<V>(2.0) ^ 2.5\n",
        "type V = Unit(float, \"m/s\")\nlocal n = as.<int>(2)\nreturn as.<V>(2.0) ^ n\n",
    ];
    for src in sources {
        assert!(run(src).is_err(), "{src}");
    }
}

/// The zeroth power is dimensionless, which is the same type as no label.
#[test]
fn the_zeroth_power_is_dimensionless() {
    assert!(
        run(r#"
type V = Unit(float, "m/s")
function takesV(v: V): int
    return 1
end
return takesV(as.<V>(2.0) ^ 0)
"#)
        .is_err()
    );
}

/// A local is nilable by default, and the unit still has to survive the `nil`.
#[test]
fn a_nilable_local_keeps_its_unit() {
    let err = run(r#"
type V = Unit(float, "m/s")
local v: V = 2.0
local x: string = v ^ 2
"#)
    .unwrap_err();
    assert!(err.to_string().contains("m^2/s^2"), "{err}");
}

/// A unit combines, and `m * m` is `m^2`, not `m`.
#[test]
fn a_unit_combines() {
    run(r#"
type Side = Unit(int, "m")
type Area = Unit(int, "m^2")
function square(s: Area): int
    return 1
end
return square(as.<Side>(3) * as.<Side>(3))
"#)
    .unwrap();
}
