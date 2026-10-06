use duka_macros::duka_builtin;

#[duka_builtin(name = "f", params(x: int, x: int))]
fn f(x: i64, y: i64) -> Result<(), ()> {
    Ok(())
}

fn main() {}
