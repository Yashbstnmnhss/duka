//! The parser recurses with the source, so nesting is bounded on purpose: a
//! pathological file must produce a diagnostic, not take the process down.

use duka_lib::harness::run;

#[test]
fn deep_expression_nesting_is_a_diagnostic_not_a_crash() {
    let n = 2000;
    let src = format!("local x = {}1{}\nreturn x\n", "(".repeat(n), ")".repeat(n));
    let err = run(&src).expect_err("nesting past the limit is refused");
    assert!(err.to_string().contains("Nesting is too deep"), "{err}");
}

#[test]
fn deep_block_nesting_is_a_diagnostic_not_a_crash() {
    let n = 2000;
    let src = format!(
        "{}return 1{}\n",
        "if true then ".repeat(n),
        " end".repeat(n)
    );
    let err = run(&src).expect_err("nesting past the limit is refused");
    assert!(err.to_string().contains("Nesting is too deep"), "{err}");
}

#[test]
fn deep_type_nesting_is_a_diagnostic_not_a_crash() {
    let n = 2000;
    let src = format!(
        "local x: {}int{}\nreturn 0\n",
        "fn(".repeat(n),
        ")".repeat(n)
    );
    let err = run(&src).expect_err("nesting past the limit is refused");
    assert!(err.to_string().contains("Nesting is too deep"), "{err}");
}

#[test]
fn ordinary_nesting_still_parses() {
    let src = format!(
        "local x = {}1{}\nreturn x\n",
        "(".repeat(20),
        ")".repeat(20)
    );
    run(&src).expect("twenty levels is not deep");
}
