//! UserData

use duka_backend::{DukaVM, codegen::DefaultGenerator, value::RuntimeValue, vm::VM};
use duka_gc::Heap;
use duka_lib::{
    duka_shared::types::DukaGenerator,
    errors::DukaRuntimeError,
    harness::{DukaError, to_ir},
};
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
    #[duka_builtin(doc = "set value", params(self, v: int))]
    fn set(&mut self, v: i64) -> Result<(), DukaRuntimeError> {
        self.value = v;
        Ok(())
    },
    // no receiver: a static method, called with `.`
    #[duka_builtin(doc = "add two numbers", params(a: int, b: int), returns(int))]
    fn impl_add_ints(a: i64, b: i64) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Int(a + b))
    },
    // `Self` in a written type is this userdata's type
    #[duka_builtin(doc = "whether both counters hold the same value", params(self, other: Self), returns(bool))]
    fn impl_same(&self, other: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
        let same = match other {
            RuntimeValue::UserData(cell) => {
                let other = cell.borrow();
                (other.payload.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<Counter>()
                    .is_some_and(|other| other.value == self.value)
            }
            _ => false,
        };
        Ok(RuntimeValue::Bool(same))
    },
}

fn run_with_obj(src: &str) -> Result<RuntimeValue, DukaError> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(DukaError::Codegen)?;
    let mut vm = VM::new(Heap::new());
    let obj = Counter::new(0).into_value(&mut vm.heap);
    vm.set_global("obj", obj);
    let count = vm.execute(&proto).map_err(DukaError::RuntimeTrace)?;
    let mut main = vm.main_coroutine_mut();
    let mut state = std::mem::take(&mut main.inner);
    let mut vals: Vec<RuntimeValue> = state
        .take_stack_many(0, count)
        .map_err(DukaError::Runtime)?
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

#[test]
fn static_method_is_called_with_dot() {
    let r = run_with_obj("return obj.add_ints(2, 3)").unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn self_in_a_written_type_is_the_own_userdata() {
    let r = run_with_obj("return obj:same(obj)").unwrap();
    assert_eq!(r, RuntimeValue::Bool(true));
}

#[test]
fn generated_meta_names_the_self_type_and_marks_static_methods() {
    use duka_lib::duka_shared::docs::{DocType, MetaItemInfo};
    let MetaItemInfo::UserData { ty_name, methods } = &__DUKA_COUNTER_META.info else {
        panic!("a userdata meta holds its methods");
    };
    assert_eq!(*ty_name, "Counter");

    let same = methods.iter().find(|m| m.name == "same").unwrap();
    let MetaItemInfo::Function { params, .. } = &same.info else {
        panic!("a method is a function");
    };
    assert_eq!(params[0].name, "self");
    assert_eq!(params[0].ty, DocType::UserData("Counter"));
    assert_eq!(params[1].name, "other");
    assert_eq!(params[1].ty, DocType::UserData("Counter"));

    let add = methods.iter().find(|m| m.name == "add_ints").unwrap();
    assert!(add.get_flag("static_method").is_some());
    let MetaItemInfo::Function { params, .. } = &add.info else {
        panic!("a method is a function");
    };
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "a");
}
