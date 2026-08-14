//! UserData

use duka_backend::{DukaVM, codegen::DefaultGenerator, value::RuntimeValue, vm::VM};
use duka_gc::Heap;
use duka_lib::{duka_shared::types::DukaGenerator, errors::DukaRuntimeError, harness::to_ir};
use duka_macros::duka_user_data;

duka_user_data! {
    struct Counter {
        value: i64
    }
    constructor fn new(value: i64) -> Self {
        Self { value }
    }
    #[duka_builtin(doc = "get current value", params(self: userdata), returns(int))]
    fn get(&self) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Int(self.value))
    },
    #[duka_builtin(doc = "add n", params(self: userdata, n: int))]
    fn add(&mut self, n: i64) -> Result<(), DukaRuntimeError> {
        self.value += n;
        Ok(())
    },
    #[duka_builtin(doc = "set value", params(self: userdata, v: int))]
    fn set(&mut self, v: i64) -> Result<(), DukaRuntimeError> {
        self.value = v;
        Ok(())
    },
}

fn run_with_obj(src: &str) -> Result<RuntimeValue, String> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    let mut vm = VM::new(Heap::new());
    let obj = Counter::new(0).into_value(&mut vm.heap);
    vm.set_global("obj", obj);
    let count = vm.execute(&proto).map_err(|e| format!("{e}"))?;
    let mut main = vm.main_coroutine_mut();
    let mut state = std::mem::take(&mut main.inner);
    let mut vals: Vec<RuntimeValue> = state
        .take_stack_many(0, count)
        .map_err(|e| format!("{e}"))?
        .into();
    Ok(vals.pop().unwrap_or(RuntimeValue::Nil))
}

#[test]
fn method_get() {
    let r = run_with_obj("return obj:get()").unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn method_mutation_persists() {
    let r = run_with_obj(
        r#"
obj:add(5)
obj:add(3)
return obj:get()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(8));
}

#[test]
fn method_set_then_get() {
    let r = run_with_obj(
        r#"
obj:set(10)
return obj:get()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(10));
}

#[test]
fn method_as_function_value() {
    let r = run_with_obj("return obj.get(obj)").unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn set_unknown_field_is_noop() {
    let r = run_with_obj(
        r#"
obj.bad = 1
return obj:get()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(0));
}

#[test]
fn metatable_is_not_exposed() {
    let r = run_with_obj("return get_metatable(obj)");
    assert!(r.is_err());
}

#[test]
fn missing_field_is_nil() {
    let r = run_with_obj("return obj.nope").unwrap();
    assert!(r.is_nil());
}
