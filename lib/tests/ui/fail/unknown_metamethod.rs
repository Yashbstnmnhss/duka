use duka_macros::duka_user_data;

duka_user_data! {
    struct S;
    #[duka_builtin(name = "__toString", params(self: userdata))]
    fn fmt(&self) -> Result<(), ()> {
        Ok(())
    },
}

fn main() {}
