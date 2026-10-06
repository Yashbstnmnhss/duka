use duka_lib::errors::DukaRuntimeError;
use duka_macros::duka_user_data;

duka_user_data! {
    struct Vec2 {
        x: f64
    }
    constructor fn new(x: f64) -> Self {
        Vec2 { x }
    }
    metamethod {
        #[duka_builtin(name = "len", params(self: userdata))]
        fn impl_len(&self) -> Result<(), DukaRuntimeError> {
            Ok(())
        },
        #[duka_builtin(name = "index", params(self: userdata, key: int))]
        fn impl_index(&self, key: i64) -> Result<(), DukaRuntimeError> {
            let _ = key;
            Ok(())
        },
    },
    #[duka_builtin(name = "get", params(self: userdata))]
    fn get(&self) -> Result<(), DukaRuntimeError> {
        Ok(())
    },
}

fn main() {}
