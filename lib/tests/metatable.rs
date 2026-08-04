//! End-to-end metatable functional tests: parse -> compile -> run.

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
    Ok(dbg!(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil)))
}

#[test]
fn arith_meta_add() {
    let r = run_last(
        r#"
local mt = {}
mt.__add = function(a, b)
    return a.value + b.value
end
local a = setmetatable({ value = 10 }, mt)
local b = setmetatable({ value = 5 }, mt)
return a + b
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn arith_meta_mul() {
    let r = run_last(
        r#"
local mt = {}
mt.__mul = function(a, b)
    return a.value * b.value
end
local a = setmetatable({ value = 6 }, mt)
local b = setmetatable({ value = 7 }, mt)
return a * b
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn call_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__call = function(self, x, y)
    return x * y
end
local obj = setmetatable({}, mt)
return obj(6, 7)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn index_meta_table_fallback() {
    let r = run_last(
        r#"
local base = { name = "foo" }
local mt = { __index = base }
local obj = setmetatable({}, mt)
return obj.name
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "foo");
}

#[test]
fn index_meta_function() {
    let r = run_last(
        r#"
local mt = {}
mt.__index = function(self, key)
    print("I HATE YOU")
    return key .. "_virtual"
end
local obj = setmetatable({}, mt)
return obj.x
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "x_virtual");
}

#[test]
fn newindex_meta() {
    let r = run_last(
        r#"
local store = {}
local mt = {}
mt.__newindex = function(self, key, value)
    store[key] = value * 2
end
local obj = setmetatable({}, mt)
obj["x"] = 21
return store.x
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn tostring_meta_in_concat() {
    let r = run_last(
        r#"
local mt = {}
mt.__tostring = function(self)
    return "obj"
end
local obj = setmetatable({}, mt)
return obj .. "!"
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "obj!");
}

#[test]
fn eq_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__eq = function(a, b)
    return a.id == b.id
end
local x = setmetatable({ id = 1 }, mt)
local y = setmetatable({ id = 1 }, mt)
local z = setmetatable({ id = 2 }, mt)
return x == y and not (x == z)
"#,
    )
    .unwrap();
    assert!(r.eval_to_bool());
}

#[test]
fn lt_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__lt = function(a, b)
    return a.rank < b.rank
end
local a = setmetatable({ rank = 1 }, mt)
local b = setmetatable({ rank = 2 }, mt)
return a < b
"#,
    )
    .unwrap();
    assert!(r.eval_to_bool());
}

#[test]
fn getset_metatable_roundtrip() {
    let r = run_last(
        r#"
local mt = { tag = "special" }
local obj = setmetatable({}, mt)
local got = getmetatable(obj)
return got.tag
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "special");
}

#[test]
fn set_metatable_nil_removes() {
    let r = run_last(
        r#"
local mt = { __index = { name = "hidden" } }
local obj = setmetatable({}, mt)
setmetatable(obj, nil)
return obj.name
"#,
    )
    .unwrap();
    assert!(r.is_nil());
}

#[test]
fn newindex_direct_field_no_recursion() {
    let r = run_last(
        r#"
local out = {}
local mt = {}
mt.__newindex = function(self, key, value)
    out[key] = value
end
local obj = setmetatable({}, mt)
obj["x"] = 21
return out.x
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(21));
}

#[test]
fn newindex_reassign_no_recursion() {
    let r = run_last(
        r#"
local out = {}
local out2 = {}
local mt = {}
mt.__newindex = function(self, key, value)
    out[key] = value
end
mt.__newindex = function(self, key, value)
    out2[key] = value
end
local obj = setmetatable({}, mt)
obj["y"] = 5
return out2.y
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn tostring_meta_direct_field() {
    let r = run_last(
        r#"
local obj = {}
obj.__tostring = function(self)
    return "obj"
end
return obj .. "!"
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "obj!");
}

fn run_cmp(expr: &str) -> RuntimeValue {
    run_last(&format!(
        r#"
local mt = {{}}
mt.__lt = function(a, b)
    if type(a) == "int" or type(a) == "float" then
        return a < b.value
    end
    return a.value < b
end
mt.__le = function(a, b)
    if type(a) == "int" or type(a) == "float" then
        return a <= b.value
    end
    return a.value <= b
end
mt.__eq = function(a, b)
    if type(a) == "int" or type(a) == "float" then
        return a == b.value
    end
    return a.value == b
end
local t = setmetatable({{ value = 10 }}, mt)
return {expr}
"#
    ))
    .unwrap()
}

#[test]
fn lt_meta_const_both_sides() {
    assert!(!run_cmp("t < 5").eval_to_bool());
    assert!(run_cmp("5 < t").eval_to_bool());
    assert!(run_cmp("t > 5").eval_to_bool());
    assert!(!run_cmp("5 > t").eval_to_bool());
}

#[test]
fn le_meta_const_both_sides() {
    assert!(run_cmp("t <= 10").eval_to_bool());
    assert!(run_cmp("10 <= t").eval_to_bool());
    assert!(run_cmp("t >= 5").eval_to_bool());
    assert!(!run_cmp("5 >= t").eval_to_bool());
}

#[test]
fn eq_meta_const_both_sides() {
    assert!(run_cmp("t == 10").eval_to_bool());
    assert!(run_cmp("10 == t").eval_to_bool());
    assert!(!run_cmp("t == 5").eval_to_bool());
    assert!(!run_cmp("5 == t").eval_to_bool());
}

#[test]
fn sub_meta_with_imm() {
    let r = run_last(
        r#"
local mt = {}
mt.__sub = function(a, b)
    return a.value - b
end
local t = setmetatable({ value = 10 }, mt)
return t - 5
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn add_meta_swapped_operands() {
    let r = run_last(
        r#"
local mt = {}
mt.__add = function(a, b)
    return a.value + b
end
local t = setmetatable({ value = 10 }, mt)
return 5 + t
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn unm_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__unm = function(a)
    return -a.value
end
local t = setmetatable({ value = 10 }, mt)
return -t
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(-10));
}

#[test]
fn band_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__band = function(a, b)
    return a.value & b
end
local t = setmetatable({ value = 6 }, mt)
return t & 3
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(2));
}

#[test]
fn bor_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__bor = function(a, b)
    return a.value | b
end
local t = setmetatable({ value = 6 }, mt)
return t | 1
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn bxor_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__bxor = function(a, b)
    return a.value ~ b
end
local t = setmetatable({ value = 6 }, mt)
return t ~ 3
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn shl_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__shl = function(a, b)
    return a.value << b
end
local t = setmetatable({ value = 1 }, mt)
return t << 3
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(8));
}

#[test]
fn shr_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__shr = function(a, b)
    return a.value >> b
end
local t = setmetatable({ value = 8 }, mt)
return t >> 3
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(1));
}

#[test]
fn concat_meta_mixed_chain() {
    let r = run_last(
        r#"
local mt = {}
mt.__concat = function(a, b)
    if type(a) == "string" then
        return a .. "[" .. tostring(b) .. "]"
    end
    return tostring(a) .. "<>" .. b
end
local t = setmetatable({}, mt)
return "x" .. t .. "y" .. t
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "x[table]y[table]");
}

#[test]
fn tostring_builtin_uses_meta() {
    let r = run_last(
        r#"
local mt = {}
mt.__tostring = function(self)
    return "obj"
end
local obj = setmetatable({}, mt)
return tostring(obj)
"#,
    )
    .unwrap();
    assert_eq!(r.eval_to_string(), "obj");
}

#[test]
fn generic_for_single_var() {
    let r = run_last(
        r#"
local count = 0
local sum = 0
local function iter(s, k)
    if k == 3 then return nil end
    return k + 1
end
for k in iter, nil, 0 do
    count = count + 1
    sum = sum + k
end
return count, sum
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn generic_for_two_vars_with_state() {
    let r = run_last(
        r#"
local arr = {10, 20, 30}
local n = 0
local total = 0
local function nexti(t, i)
    if i >= 2 then return nil end
    return i + 1, t[i + 1]
end
for k, v in nexti, arr, -1 do
    n = n + 1
    total = total + v
end
return n, total
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(60));
}

#[test]
fn generic_for_call_explist() {
    let r = run_last(
        r#"
local arr = {5, 7, 9}
local function iter(t, i)
    if i >= 2 then return nil end
    return i + 1, t[i + 1]
end
local function triple()
    return iter, arr, -1
end
local total = 0
for k, v in triple() do
    total = total + v
end
return total
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(21));
}

#[test]
fn generic_for_continue_and_break() {
    let r = run_last(
        r#"
local sum = 0
local hits = 0
local function gen(s, i)
    if i >= 5 then return nil end
    return i + 1
end
for k in gen, nil, 0 do
    if k == 2 then continue end
    if k == 4 then break end
    hits = hits + 1
    sum = sum + k
end
return sum * 10 + hits
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn generic_for_empty_iteration() {
    let r = run_last(
        r#"
local function iter(s, k)
    return nil
end
local count = 0
for k in iter, nil, nil do
    count = count + 1
end
return count
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}
