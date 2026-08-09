//! End-to-end math builtin tests: parse -> compile -> run.

use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
    let lexer = Lexer::new(Cursor::new(src), None);
    let stream = lexer.tokenize().map_err(|e| format!("{e}"))?;
    let chunk = Parser::parse(stream, Default::default()).map_err(|e| format!("{e}"))?;
    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(format!("{err}"));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);
    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

fn run_last(src: &str) -> Result<RuntimeValue, String> {
    Ok(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn max_of_multiple_numbers() {
    assert_eq!(
        run_last("return math.max(3, 5, 4, 2)").unwrap(),
        RuntimeValue::Int(5)
    );
}

#[test]
fn min_of_multiple_numbers() {
    assert_eq!(
        run_last("return math.min(3, 5, 4, 2)").unwrap(),
        RuntimeValue::Int(2)
    );
}

#[test]
fn max_mixed_int_float() {
    assert_eq!(
        run_last("return math.max(1, 2.5)").unwrap(),
        RuntimeValue::Float(2.5)
    );
}

#[test]
fn min_negative() {
    assert_eq!(
        run_last("return math.min(-3, 5)").unwrap(),
        RuntimeValue::Int(-3)
    );
}

#[test]
fn max_of_table() {
    assert_eq!(
        run_last("return math.max({1, 5, 3, 9, 2})").unwrap(),
        RuntimeValue::Int(9)
    );
}

#[test]
fn min_of_table() {
    assert_eq!(
        run_last("return math.min({1, 5, 3, 9, 2})").unwrap(),
        RuntimeValue::Int(1)
    );
}

#[test]
fn max_empty_returns_nil() {
    assert_eq!(run_last("return math.max()").unwrap(), RuntimeValue::Nil);
}

#[test]
fn max_single_non_table_returns_it() {
    assert_eq!(
        run_last("return math.max(42)").unwrap(),
        RuntimeValue::Int(42)
    );
}

#[test]
fn max_uses_lt_metamethod() {
    let r = run_last(
        r#"
local mt = { __lt = function(a, b) return a.v < b.v end }
local a = set_metatable({ v = 7 }, mt)
local b = set_metatable({ v = 9 }, mt)
return math.max(a, b).v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(9));
}

#[test]
fn min_uses_lt_metamethod() {
    let r = run_last(
        r#"
local mt = { __lt = function(a, b) return a.v < b.v end }
local a = set_metatable({ v = 7 }, mt)
local b = set_metatable({ v = 9 }, mt)
return math.min(a, b).v
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn sum_of_multiple() {
    assert_eq!(
        run_last("return math.sum(1, 2, 3, 4)").unwrap(),
        RuntimeValue::Int(10)
    );
}

#[test]
fn sum_of_table() {
    assert_eq!(
        run_last("return math.sum({1, 2, 3, 4})").unwrap(),
        RuntimeValue::Int(10)
    );
}

#[test]
fn sum_mixed_numbers() {
    assert_eq!(
        run_last("return math.sum(1, 0.5, 0.25)").unwrap(),
        RuntimeValue::Float(1.75)
    );
}

#[test]
fn abs_int() {
    assert_eq!(
        run_last("return math.abs(-5)").unwrap(),
        RuntimeValue::Int(5)
    );
}

#[test]
fn abs_float() {
    assert_eq!(
        run_last("return math.abs(-2.5)").unwrap(),
        RuntimeValue::Float(2.5)
    );
}

#[test]
#[should_panic]
fn abs_non_number_passthrough() {
    assert_eq!(
        run_last(r#"return math.abs("x")"#)
            .unwrap()
            .eval_to_string()
            .into_owned(),
        "x"
    );
}

#[test]
fn floor_float() {
    assert_eq!(
        run_last("return math.floor(3.7)").unwrap(),
        RuntimeValue::Int(3)
    );
}

#[test]
fn floor_negative() {
    assert_eq!(
        run_last("return math.floor(-3.2)").unwrap(),
        RuntimeValue::Int(-4)
    );
}

#[test]
fn ceil_float() {
    assert_eq!(
        run_last("return math.ceil(3.2)").unwrap(),
        RuntimeValue::Int(4)
    );
}

#[test]
fn ceil_negative() {
    assert_eq!(
        run_last("return math.ceil(-3.7)").unwrap(),
        RuntimeValue::Int(-3)
    );
}

#[test]
fn round_half_away_from_zero() {
    assert_eq!(
        run_last("return math.round(2.5)").unwrap(),
        RuntimeValue::Int(3)
    );
}

#[test]
fn round_down() {
    assert_eq!(
        run_last("return math.round(2.4)").unwrap(),
        RuntimeValue::Int(2)
    );
}

#[test]
fn floor_int_passthrough() {
    assert_eq!(
        run_last("return math.floor(5)").unwrap(),
        RuntimeValue::Int(5)
    );
}

#[test]
fn sin_zero() {
    let r = run_last("return math.sin(0)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), 0.0));
}

#[test]
fn cos_zero() {
    let r = run_last("return math.cos(0)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), 1.0));
}

#[test]
fn tan_pi_over_4() {
    let r = run_last("return math.tan(0.7853981633974483)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), 1.0));
}

#[test]
fn arcsin_one() {
    let r = run_last("return math.arcsin(1)").unwrap();
    assert!(approx(
        r.eval_to_float().unwrap(),
        std::f64::consts::FRAC_PI_2
    ));
}

#[test]
fn arctan2() {
    let r = run_last("return math.arctan2(1, 1)").unwrap();
    assert!(approx(
        r.eval_to_float().unwrap(),
        std::f64::consts::FRAC_PI_4
    ));
}

#[test]
fn sqrt_float() {
    let r = run_last("return math.sqrt(4)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), 2.0));
}

#[test]
fn deg_to_rad() {
    let r = run_last("return math.deg_to_rad(180)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), std::f64::consts::PI));
}

#[test]
fn rad_to_deg() {
    let r = run_last("return math.rad_to_deg(3.141592653589793)").unwrap();
    assert!(approx(r.eval_to_float().unwrap(), 180.0));
}

#[test]
fn trig_type_error() {
    assert!(run(r#"return math.sin("x")"#).is_err());
    assert!(run(r#"return math.sqrt("x")"#).is_err());
}

#[test]
fn pi_and_e_constants() {
    let pi = run_last("return math.PI").unwrap();
    let e = run_last("return math.E").unwrap();
    assert!(approx(pi.eval_to_float().unwrap(), std::f64::consts::PI));
    assert!(approx(e.eval_to_float().unwrap(), std::f64::consts::E));
}

#[test]
fn randf_in_unit_interval() {
    let r = run_last(
        r#"
math.set_seed(1234)
local ok = true
for i = 1, 100 do
    local v = math.randf()
    if not (v >= 0 and v < 1) then
        ok = false
    end
end
return ok
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn set_seed_reproducible() {
    let r = run_last(
        r#"
math.set_seed(42)
local a = math.randf()
math.set_seed(42)
local b = math.randf()
return a == b
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn sign_preserves_numeric_type() {
    assert_eq!(
        run_last("return math.sign(-5)").unwrap(),
        RuntimeValue::Int(-1)
    );
    assert_eq!(
        run_last("return math.sign(0)").unwrap(),
        RuntimeValue::Int(0)
    );
    assert_eq!(
        run_last("return math.sign(7)").unwrap(),
        RuntimeValue::Int(1)
    );
    assert_eq!(
        run_last("return math.sign(-2.5)").unwrap(),
        RuntimeValue::Float(-1.0)
    );
    assert_eq!(
        run_last("return math.sign(3.5)").unwrap(),
        RuntimeValue::Float(1.0)
    );

    assert_eq!(
        run_last("return math.clamp(5, 0, 3)").unwrap(),
        RuntimeValue::Float(3.0)
    );
    assert_eq!(
        run_last("return math.clamp(-1, 0, 3)").unwrap(),
        RuntimeValue::Float(0.0)
    );
    assert_eq!(
        run_last("return math.clamp(2, 0, 3)").unwrap(),
        RuntimeValue::Float(2.0)
    );
}

#[test]
fn clamp_invalid_argument() {
    let err = run(r#"return math.clamp("x", 0, 1)"#).unwrap_err();
    assert!(err.contains("not number"), "{}", err);
}

#[test]
fn modf_returns_two_values() {
    let vals = run("return math.modf(2.5)").unwrap();
    assert_eq!(vals.len(), 2);
    assert_eq!(vals[0], RuntimeValue::Int(2));
    assert_eq!(vals[1], RuntimeValue::Float(0.5));
}

#[test]
fn factors_returns_dynamic_values() {
    let vals = run("return math.factors(12)").unwrap();
    let expected: Vec<RuntimeValue> = vec![1, 2, 3, 4, 6, 12]
        .into_iter()
        .map(RuntimeValue::Int)
        .collect();
    assert_eq!(vals.len(), expected.len());
    for (a, b) in vals.iter().zip(expected.iter()) {
        assert_eq!(a, b);
    }
}

#[test]
fn randf_range_bounded() {
    let r = run_last(
        r#"
math.set_seed(99)
local ok = true
for i = 1, 50 do
    local v = math.randf_range(-2, 3)
    if not (v >= -2 and v < 3) then
        ok = false
    end
end
return ok
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn union_param_accepts_all_members() {
    assert_eq!(
        run_last("return typeof_union(42)").unwrap().eval_to_string(),
        "int"
    );
    assert_eq!(
        run_last("return typeof_union(\"hi\")").unwrap().eval_to_string(),
        "string"
    );
    assert_eq!(
        run_last("return typeof_union(true)").unwrap().eval_to_string(),
        "bool"
    );
}

#[test]
fn union_param_rejects_other_types() {
    let err = run_last("return typeof_union({})").unwrap_err();
    assert!(err.contains("int|string|bool"), "{err}");
    assert!(err.contains("got table"), "{err}");
}
