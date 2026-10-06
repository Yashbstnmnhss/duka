#[test]
fn macro_test() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/macros/fail/*.rs");
    t.pass("tests/macros/pass/*.rs");
}
