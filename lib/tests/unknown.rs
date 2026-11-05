//! `unknown` is the top type; `any` is the dynamic one.

use duka_lib::harness::run;

/// A value may be put into `unknown` without saying what it is.
#[test]
fn unknown_takes_every_value() {
    run(r#"
local x: unknown = 1
local y: unknown = "s"
return as.<int>(x)
"#)
    .unwrap();
}

/// Nothing comes out of `unknown` without naming it first.
#[test]
fn unknown_gives_nothing_back() {
    assert!(
        run(r#"
local x: unknown = 1
local y: int = x
return y
"#)
        .is_err()
    );
}

/// `any` keeps its old job: it stands in both ways, dynamically.
#[test]
fn any_still_flows_both_ways() {
    run(r#"
local x: any = 1
local y: int = x
local z: any = y
return z
"#)
    .unwrap();
}
