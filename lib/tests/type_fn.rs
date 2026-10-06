//! Type function (compile-time type evaluation) tests

use duka_backend::value::RuntimeValue;
use duka_lib::harness::{run, run_results};

fn strs(v: &[RuntimeValue]) -> Vec<String> {
    v.iter().map(|x| x.eval_to_string().into_owned()).collect()
}

#[test]
fn match_test() {
    run_results(
        r#"
type function C(a)
    if a == int then
        return string
    end
    return a
end
type function Match(b)
    return match b then
        C(local u) -> u;
    else
        return never
    end
end
--type CCC = Match(true)
type CCC2 = Match(C(int))
    "#,
    )
    .unwrap();
}

#[test]
fn stdlib_calls_keep_their_declared_return_types() {
    // `os.clock` returns a float, `string.upper` a string: if the standard
    // library were not in the symbol table these would all be `any`
    let res = run_results(
        r#"
local s = string.upper("ab")
local n = math.floor(1.7)
local t = os.date
return s, n, t
"#,
    )
    .unwrap();
    assert_eq!(strs(&res)[0], "AB");
    assert_eq!(strs(&res)[1], "1");
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
    assert!(
        err.to_string().contains("expected 2 arguments, got 1"),
        "{err}"
    );
}

#[test]
fn err_unknown_type_function() {
    let err = run(r#"
local z: Nope(int) = 1
return z
"#)
    .unwrap_err();
    assert!(err.to_string().contains("unknown type function"), "{err}");
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
    assert!(err.to_string().contains("max iterations"), "{err}");
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
    assert!(err.to_string().contains("never"), "{err}");
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
    assert!(err.to_string().contains("not yet supported"), "{err}");
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
    assert!(err.to_string().contains("immutable"), "{err}");
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
    assert!(err.to_string().contains("global"), "{err}");
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
    assert!(err.to_string().contains(":5:10-5:16"), "{err}");
    assert!(!err.to_string().contains(":3:"), "{err}");
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
    assert!(err.to_string().contains("function(int)"), "{err}");
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
    assert!(err.to_string().contains("'int?'"), "{err}");
    assert!(!err.to_string().contains("int | int"), "{err}");
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
    assert!(err.to_string().contains("max recursion depth"), "{err}");
}

#[test]
fn recursive_inline_type_fn() {
    run_results(
        r#"
type function List(T) = [T, List(T)?]
local xs: List(int) = [1, [2, nil]]
return xs[0]
"#,
    )
    .unwrap();
}

#[test]
fn recursive_inline_head_is_t() {
    let res = run_results(
        r#"
type function List(T) = [T, List(T)?]
local h: List(int)[0] = 42
return h
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["42"]);
}

#[test]
fn prelude_optional_works() {
    let res = run_results(
        r#"
local a: Optional(int) = 5
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["5"]);
}

#[test]
fn prelude_non_nil_works() {
    let res = run_results(
        r#"
local a: NonNil(int) = 7
return a
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["7"]);
}

#[test]
fn fn_multi_return_annotation() {
    let res = run_results(
        r#"
function f(): (int, string)
    return 1, "a"
end
local a, b = f()
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "a"]);
}

#[test]
fn fn_multi_return_rejects_mismatch() {
    let err = run(r#"
function f(): (int, string)
    return 1, 2
end
local a, b = f()
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
}

#[test]
fn fn_ret_vararg_annotation() {
    let res = run_results(
        r#"
function f(): (int, ...)
    return 1, 2, 3
end
local a, b, c = f()
return a, b, c
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["1", "2", "3"]);
}

#[test]
fn fn_bare_ret_vararg_annotation() {
    let res = run_results(
        r#"
function g(): ...
    return 4, 5
end
return g()
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["4", "5"]);
}

#[test]
fn fnlit() {
    let _ = run_results(
        r#"
type A = type fn() int
return 1
"#,
    )
    .unwrap();
}

/// A type function that is handed another type function and applies it. This
/// is the foundation a `where` predicate clause needs: `Sized(T)` is the same
/// shape, only the result is read as a boolean instead of being returned.
#[test]
fn a_type_function_can_be_applied_to_another() {
    run_results(
        r#"
type function Opt(x)
    return x?
end
type function Apply(f, t)
    return f(t)
end
local a: Apply(Opt, int) = nil
return a
"#,
    )
    .unwrap();
}

/// `and` and `or` read their operands as truth values, and the result is a
/// boolean, which the language spells `bool` rather than `true` or `false`:
/// the answer no longer depends on which operands produced it.
#[test]
fn the_logical_connectives_answer_with_a_boolean() {
    let res = run_results(
        r#"
type function both(a, b)
    return a and b
end
type function either(a, b)
    return a or b
end
local a: both(true, false) = false
local b: both(true, true) = true
local c: either(false, false) = false
local d: either(false, true) = true
return a, b, c, d
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "bool", "bool", "bool"]);
}

/// Duka's truthiness, not a comparison against `bool`: a type that is not a
/// boolean at all still answers. A requirement is written over types, and
/// almost no type is a boolean, so `T: Sized` can only mean "the answer was not
/// explicitly falsy" and nothing more.
#[test]
fn a_requirement_reads_truthiness_rather_than_a_comparison() {
    let res = run_results(
        r#"
type function answer(t)
    return t
end
type function IsList(t)
    return match t then
        list() -> true;
        else return false
    end
end
-- `int` is not a boolean but it answers, so the right side is never read and
-- the badly-arityed call is harmless
type function answered(t)
    return answer(t) or IsList(t, t)
end
-- `nil` is falsy, so the right side is read, and it is the side that answers
type function unanswered(t)
    return answer(t) or IsList(t)
end
local a: answered(int) = true
local b: unanswered(nil) = false
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "bool"]);
}

/// The short circuit is not an optimisation, it is the only way to write a
/// condition over something not known yet. `array<int>` is a `list`, so the
/// left side answers and the badly-arityed right side is never called, which is
/// the point: the side that was not needed did not have to be evaluable.
#[test]
fn a_requirement_can_ask_a_question_it_cannot_yet_answer() {
    run_results(
        r#"
type function IsList(t)
    return match t then
        list() -> true;
        else return false
    end
end
type function known(t)
    return IsList(t) or IsList(t, t)
end
local a: known(array<int>) = true
return a
"#,
    )
    .unwrap();
}

/// Without the short circuit the right side is reached and its arity is
/// checked. `string` is not a list, so the left side fails and the right side
/// runs, and the run is an error: the previous test only means something because
/// this one is a failure.
#[test]
fn a_reached_side_is_still_checked() {
    let err = run(r#"
type function IsList(t)
    return match t then
        list() -> true;
        else return false
    end
end
type function unknown(t)
    return IsList(t) or IsList(t, t)
end
local a: unknown(string) = true
return a
"#)
    .unwrap_err();
    assert!(err.to_string().contains("arguments"), "{err}");
}

/// `and` short circuits the other way round: the left side failing is enough to
/// answer, so the right side is not read. The right side here is a call with the
/// wrong arity, which is only acceptable because it is never reached.
#[test]
fn and_stops_at_a_failing_left_side() {
    run_results(
        r#"
type function IsList(t)
    return match t then
        list() -> true;
        else return false
    end
end
type function known(t)
    return IsList(t) and IsList(t, t)
end
-- `string` is not a list, so the left side fails and answers the whole thing
local a: known(string) = false
return a
"#,
    )
    .unwrap();
}

/// `xor` reads both sides by definition, so it cannot short circuit and the
/// arity of both calls is checked even though the left side already answers.
#[test]
fn xor_reads_both_sides() {
    let res = run_results(
        r#"
type function IsList(t)
    return match t then
        list() -> true;
        else return false
    end
end
type function differs(a, b)
    return IsList(a) xor IsList(b)
end
local a: differs(array<int>, string) = true
local b: differs(array<int>, list<int>) = false
return a, b
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["bool", "bool"]);
}

/// A type function held in a field is still a type function. The callee used to
/// have to be a bare name, so a value reached by a path was unreachable.
#[test]
fn a_type_function_reached_by_a_path_can_be_called() {
    run_results(
        r#"
type function deep(t)
    return array<array<array<t>>>
end
type Fns = { deep: type fn(t) array<array<array<t>>> }
type function Use(t)
    return Fns.deep(t)
end
local a: Use(int) = nil
return a
"#,
    )
    .unwrap();
}

/// The same, but the callee arrives as an anonymous literal rather than a
/// named type function.
#[test]
fn a_type_function_can_be_applied_to_a_literal() {
    run_results(
        r#"
type function Apply(f, t)
    return f(t)
end
local a: Apply(type fn(x) x?, int) = nil
return a
"#,
    )
    .unwrap();
}

#[test]
fn generic_inline_type_fn() {
    let _ = run_results(
        r#"
type function Pick<T>(a: T, b: T) = a
local x: Pick(int, int) = 3
return x
"#,
    )
    .unwrap();
}

#[test]
fn generic_named_type_fn() {
    let _ = run_results(
        r#"
type function PickA<T>(a: T, b: T)
    return a
end
local x: PickA(int, int) = 3
return x
"#,
    )
    .unwrap();
}

#[test]
fn unclosed_paren_in_type_fn_no_hang() {
    let err = run(r#"
type function F(a
return 1
"#);
    assert!(err.is_err(), "expected a parse error, got ok");
}

#[test]
fn unclosed_generic_in_type_fn_no_hang() {
    let err = run(r#"
type function F<T(a: int)
    return a
end
return 1
"#);
    assert!(err.is_err(), "expected a parse error, got ok");
}

#[test]
fn inline_fn_arity_mismatch_is_error_not_silent() {
    let err = run(r#"
type function Pair(a, b) = [a, b]
local x: Pair(int) = [1]
return 1
"#)
    .unwrap_err();
    assert!(err.to_string().contains("expected 2 arguments"), "{err}");
}

#[test]
fn type_of_local_var_accepts_match() {
    let res = run_results(
        r#"
local x = 123
local y: type(x) = 456
return y
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["456"]);
}

#[test]
fn type_of_local_var_rejects_mismatch() {
    let err = run(r#"
local x = 123
local y: type(x) = "hello"
return y
"#)
    .unwrap_err();
    assert!(err.to_string().contains("'int'"), "{err}");
    assert!(err.to_string().contains("'string'"), "{err}");
}

#[test]
fn type_of_literal() {
    let res = run_results(
        r#"
local y: type("str") = "abc"
return y
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["abc"]);
}

#[test]
fn type_of_nil() {
    let res = run_results(
        r#"
local y: type(nil) = nil
return y
"#,
    )
    .unwrap();
    assert_eq!(res[0], RuntimeValue::Nil);
}

#[test]
fn type_of_union_with_nil() {
    let res = run_results(
        r#"
local x = 1
local y: type(x) | nil = nil
return y
"#,
    )
    .unwrap();
    assert_eq!(res[0], RuntimeValue::Nil);
}

#[test]
fn type_of_unknown_var_is_any() {
    let res = run_results(
        r#"
local y: type(undefined) = "anything"
return y
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["anything"]);
}

#[test]
fn type_keyword_rejected_in_expression_context() {
    let err = run(r#"
local x = 1
local t = type(x)
return t
"#)
    .unwrap_err();
    assert!(err.to_string().contains("Unexpected token type"), "{err}");
}

#[test]
fn type_of_arg_type_function_accepts_match() {
    let res = run_results(
        r#"
type function Maybe(t)
    if t == int then
        return string
    else
        return t
    end
end
local x = 123
local y: Maybe(type(x)) = "ok"
return y
"#,
    )
    .unwrap();
    assert_eq!(strs(&res), ["ok"]);
}

/// A type function has to survive being stored in a type, not only being passed
/// straight to a call. A record field holds a `Type`, and a closure put into a
/// `Type` used to become `any` on the way, so the field was `any` by the time it
/// was read back and applying it answered `any` as well.
#[test]
fn a_type_function_survives_being_stored_in_a_type() {
    run_results(
        r#"
type function Apply(f, t)
    return f(t)
end
type Fns = { id: type fn(t) t? }
type function Use(t)
    return Apply(Fns.id, t)
end
local a: Use(int) = nil
return a
"#,
    )
    .unwrap();
}

/// The same, one level deeper: the closure is the element of a type
/// constructor rather than a field of one.
#[test]
fn a_type_function_survives_being_an_element_of_a_type() {
    run_results(
        r#"
type function Apply(f, t)
    return f(t)
end
type Fns = array<type fn(t) t?>
type function Use(t)
    return Apply(Fns[0], t)
end
local a: Use(int) = nil
return a
"#,
    )
    .unwrap();
}

#[test]
fn type_of_arg_type_function_rejects_mismatch() {
    let err = run(r#"
type function Maybe(t)
    if t == int then
        return string
    else
        return t
    end
end
local x = 123
local y: Maybe(type(x)) = 456
return y
"#)
    .unwrap_err();
    assert!(err.to_string().contains("'string'"), "{err}");
    assert!(err.to_string().contains("'int'"), "{err}");
}
