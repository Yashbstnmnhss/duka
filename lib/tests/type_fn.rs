//! Type function (compile-time type evaluation) tests

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn basic_if_returns_type() {
    let res = run_results(
        r#"
type function Maybe(t)
    if t == int then
        return string
    else
        return t
    end
end
local a: Maybe(int) = "12321"
local b: Maybe(float) = 1.5
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["12321", "1.5"]);
}

#[test]
fn nested_call() {
    let res = run_results(
        r#"
type function Inner(t)
    if t == int then
        return string
    end
    return t
end
type function Outer(t)
    return Inner(t)
end
local c: Outer(int) = "1"
local d: Outer(float) = 3.25
return c, d
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "3.25"]);
}

#[test]
fn union_return() {
    let res = run_results(
        r#"
type function N(t)
    return t | nil
end
local e: N(int) = nil
return e
"#,
    )
    .unwrap();
    assert_eq!(res, vec![RuntimeValue::Nil]);
}

#[test]
fn match_bind_infer() {
    let res = run_results(
        r#"
type function Id(t)
    return match t then
        local x -> x;
        else return t
    end
end
local f: Id(string) = "ok"
return f
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["ok"]);
}

#[test]
fn multi_params() {
    let res = run_results(
        r#"
type function Chain(a, b)
    if a == b then
        return int
    else
        return float
    end
end
local g: Chain(float, float) = 7
local h: Chain(float, int) = 2.5
return g, h
"#,
    )
    .unwrap();
    assert_eq!(res, vec![RuntimeValue::Int(7), RuntimeValue::Float(2.5)]);
}

#[test]
fn cache_multi_use() {
    let res = run_results(
        r#"
type function F(t)
    if t == int then
        return string
    else
        return float
    end
end
local a: F(int) = "x"
local b: F(int) = "y"
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["x", "y"]);
}

#[test]
fn array_with_typecall() {
    let res = run_results(
        r#"
type function F(t)
    if t == int then
        return string
    end
    return float
end
local arr: array<F(int)> = ["a", "b"]
return arr[0]
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["a"]);
}

#[test]
fn literal_arg() {
    let res = run_results(
        r#"
type function Pick(t)
    if t == "a" then
        return int
    else
        return string
    end
end
local x: Pick("a") = 5
local y: Pick("b") = "hi"
return x, y
"#,
    )
    .unwrap();
    assert_eq!(res.len(), 2);
    assert_eq!(res[0], RuntimeValue::Int(5));
    assert_eq!(strs(&res[1..]), ["hi"]);
}

#[test]
fn local_type_alias_in_body() {
    let res = run_results(
        r#"
type function G(t)
    type Temp = int
    if t == Temp then
        return string
    else
        return t
    end
end
local x: G(int) = "s"
return x
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["s"]);
}

#[test]
fn global_alias_reference() {
    let res = run_results(
        r#"
type Base = int
type function H(t)
    if t == Base then
        return string
    end
    return t
end
local x: H(int) = "s"
return x
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["s"]);
}

#[test]
fn err_wrong_arity() {
    let err = run(r#"
type function Bad(a, b)
    return int
end
local x: Bad(int) = 1
return x
"#)
    .unwrap_err();
    assert!(err.contains("expected 2 arguments, got 1"), "{err}");
}

#[test]
fn err_unknown_type_function() {
    let err = run(r#"
local z: Nope(int) = 1
return z
"#)
    .unwrap_err();
    assert!(err.contains("unknown type function"), "{err}");
}

#[test]
fn err_recursion_depth() {
    let err = run(r#"
type function Recur(t)
    return Recur(t)
end
local y: Recur(int) = 1
return y
"#)
    .unwrap_err();
    assert!(err.contains("max iterations"), "{err}");
}

#[test]
fn err_no_return() {
    let err = run(r#"
type function Empty(t)
end
local w: Empty(int) = 1
return w
"#)
    .unwrap_err();
    assert!(err.contains("never"), "{err}");
}

#[test]
fn err_table_match_unsupported() {
    let err = run(r#"
type function S(t)
    return match t then
        { local x, ... } -> x;
        else return t
    end
end
local v: S(int) = 1

return v
"#)
    .unwrap_err();
    assert!(err.contains("not yet supported"), "{err}");
}

#[test]
fn type_local_mutable_assign() {
    let res = run_results(
        r#"
type function Acc(t)
    local result = int
    result = t
    if result == string then
        result = float
    end
    return result
end
local a: Acc(int) = 1
local b: Acc(string) = 1.5
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "1.5"]);
}

#[test]
fn type_local_immutable_rejects_assign() {
    let err = run(r#"
type function G(t)
    type X = int
    X = string
    return X
end
local v: G(int) = 1
return v
"#)
    .unwrap_err();
    assert!(err.contains("immutable"), "{err}");
}

#[test]
fn while_loop_reassigns() {
    let res = run_results(
        r#"
type function Repeat(t)
    local cur = int
    while t ~= cur do
        cur = t
    end
    return cur
end
local a: Repeat(int) = 1
local b: Repeat(float) = 1.5
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "1.5"]);
}

#[test]
fn for_numeric_binds_count() {
    let res = run_results(
        r#"
type function Count(t)
    local acc = int
    for i = 1, 3 do
        acc = acc | string
    end
    if t == int then
        return acc
    end
    return string
end
local a: Count(int) = 1
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn match_typector_array_and_table() {
    let res = run_results(
        r#"
type function Pick(t)
    return match t then
        array(local inner) -> inner;
        table(local k, local v) -> v;
        else return string
    end
end
local a: Pick(array<float>) = 1.5
local b: Pick(table<bool, int>) = 1
local c: Pick(int) = "x"
return a, b, c
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1.5", "1", "x"]);
}

#[test]
fn match_typector_shape_only() {
    let res = run_results(
        r#"
type function IsList(t)
    return match t then
        list() -> bool;
        else return string
    end
end
local a: IsList(array<int>) = true
local b: IsList(int) = "x"
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "x"]);
}

#[test]
fn plain_local_declares_type() {
    let res = run_results(
        r#"
type function Wrap(t)
    local x = t
    return x
end
local a: Wrap(int) = 1
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1"]);
}

#[test]
fn plain_local_is_mutable() {
    let res = run_results(
        r#"
type function Swap(t)
    local x = int
    x = t
    return x
end
local a: Swap(string) = "s"
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["s"]);
}

#[test]
fn plain_global_rejected_in_type_fn() {
    let err = run(r#"
type function Bad(t)
    global x = int
    return x
end
local a: Bad(int) = 1
return a
"#)
    .unwrap_err();
    assert!(err.contains("global"), "{err}");
}

#[test]
fn err_reports_at_call_site_span() {
    let err = run(r#"
type function concat(a, b)
    return a..b
end
local x: concat(int, int) = 123
return x
"#)
    .unwrap_err();
    assert!(err.contains(":5:10-5:16"), "{err}");
    assert!(!err.contains(":3:"), "{err}");
}

#[test]
fn function_type_params_resolve_against_args() {
    let err = run(r#"
type function wrap(c)
    return function(c)
end
local f: wrap(int) = 123
return f
"#)
    .unwrap_err();
    assert!(err.contains("function(int)"), "{err}");
}

#[test]
fn alias_ref_is_exact_not_nilable() {
    let err = run(r#"
type function Concat(a, b)
    return a..b
end
type A = Concat("", "")!
local c: A = 123
return c
"#)
    .unwrap_err();
    assert!(
        err.contains("incompatible with initializer of type 'int'"),
        "{err}"
    );
    assert!(err.contains("'\"\"'"), "{err}");
    assert!(!err.contains("nil"), "{err}");
}

#[test]
fn alias_ref_still_accepts_nil_when_alias_nilable() {
    let res = run_results(
        r#"
type A = "x"?
local c: A = nil
return c
"#,
    )
    .unwrap();
    assert_eq!(res[0], RuntimeValue::Nil);
}

#[test]
fn union_annotation_dedups() {
    let err = run(r#"
local x: int | int = "a"
return x
"#)
    .unwrap_err();
    assert!(err.contains("'int | nil'"), "{err}");
    assert!(!err.contains("int | int"), "{err}");
}

#[test]
fn tail_recursive_bypasses_depth_limit() {
    let res = run_results(
        r#"
type function Down(n)
    if n == 0 then
        return "done"
    else
        return Down(n - 1)
    end
end
local f: Down(100) = "done"
return f
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["done"]);
}

#[test]
fn mutual_tail_recursion_works() {
    let res = run_results(
        r#"
type function Even(n)
    if n == 0 then
        return true
    else
        return Odd(n - 1)
    end
end
type function Odd(n)
    if n == 0 then
        return false
    else
        return Even(n - 1)
    end
end
local f: Even(100) = true
local g: Odd(99) = true
return f, g
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "bool"]);
}

#[test]
fn non_tail_recursion_still_depth_limited() {
    let err = run(r#"
type function Fib(n)
    if n < 2 then
        return n
    else
        return Fib(n - 1) + Fib(n - 2)
end
end
local f: Fib(50) = 1
return f
"#)
    .unwrap_err();
    assert!(err.contains("max recursion depth"), "{err}");
}
