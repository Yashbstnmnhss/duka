use duka_macros::duka_builtin;

#[duka_builtin(name = "X", type = "fn")]
const X: i32 = 1;

fn main() {
    let _ = X;
}