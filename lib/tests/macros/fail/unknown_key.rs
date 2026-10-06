use duka_macros::duka_builtin;

#[duka_builtin(name = "f", bogus = "x")]
fn f() -> Result<(), ()> {
    Ok(())
}

fn main() {}
