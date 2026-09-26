use duka_macros::duka_user_data;

duka_user_data! {
    struct S;
    #[duka_builtin(params(self: any))]
    fn m(&self) -> Result<(), ()> {
        Ok(())
    }
}

fn main() {}
