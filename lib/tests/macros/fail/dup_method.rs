use duka_macros::duka_user_data;

duka_user_data! {
    struct S;
    #[duka_builtin(name = "m", params(self: userdata))]
    fn a(&self) -> Result<(), ()> {
        Ok(())
    },
    #[duka_builtin(name = "m", params(self: userdata))]
    fn b(&self) -> Result<(), ()> {
        Ok(())
    },
}

fn main() {}
