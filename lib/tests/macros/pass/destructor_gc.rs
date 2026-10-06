use duka_lib::errors::DukaRuntimeError;
use duka_macros::duka_user_data;

duka_user_data! {
    struct Res;
    constructor fn new() -> Self {
        Res
    }
    destructor fn drop(&mut self) -> Result<(), DukaRuntimeError> {
        Ok(())
    }
}

fn main() {}
