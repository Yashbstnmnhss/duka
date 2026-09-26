use duka_macros::duka_builtin;

#[duka_builtin(name = "f", params())]
fn f(x: i64) -> Result<(), ()> {
    Ok(())
}

fn main() {}
