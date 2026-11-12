use duka_lib::errors::DukaRuntimeError;
use duka_lib::value::RuntimeValue;
use duka_macros::duka_user_data;

duka_user_data! {
    struct Counter {
        value: i64
    }
    constructor fn new(value: i64) -> Self {
        Counter { value }
    }
    // `params(self)` is the receiver without spelling out `userdata`
    #[duka_builtin(doc = "current value", params(self), returns(int))]
    fn get(&self) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Int(self.value))
    },
    // `Self` in a written type is this userdata's type
    #[duka_builtin(doc = "same value", params(self, other: Self), returns(bool))]
    fn same(&self, other: RuntimeValue) -> Result<RuntimeValue, DukaRuntimeError> {
        let _ = other;
        Ok(RuntimeValue::Bool(false))
    },
    // no receiver: a static method, called with `.`
    #[duka_builtin(doc = "add two numbers", params(a: int, b: int), returns(int))]
    fn add(a: i64, b: i64) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Int(a + b))
    },
}

fn main() {}
