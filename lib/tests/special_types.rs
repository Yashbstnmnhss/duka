//! Coroutines and userdata: the types that are values of their own

use duka_lib::harness::{run, run_results};

#[test]
fn a_spawned_coroutine_is_a_coroutine() {
    let res = run_results(
        r#"
local function worker()
    return 1
end
local co = spawn(worker)
return costatus(co)
"#,
    )
    .unwrap();
    assert_eq!(res.len(), 1);
}

#[test]
fn a_coroutine_is_not_a_number() {
    let err = run(r#"
local function worker()
    return 1
end
local co = spawn(worker)
local n: int = co
return n
"#)
    .unwrap_err();
    assert!(err.to_string().contains("coroutine"), "{err}");
}

#[test]
fn a_userdata_is_the_userdata_it_is() {
    let res = run_results(
        r#"
local re = regex.compile("a+")
return re:is_match("aa")
"#,
    )
    .unwrap();
    assert_eq!(res.len(), 1);
}

#[test]
fn a_userdata_is_not_a_number() {
    let err = run(r#"
local re = regex.compile("a+")
local n: int = re
return n
"#)
    .unwrap_err();
    assert!(err.to_string().contains("CompiledRegex"), "{err}");
}

/// A module holds types as well as values, and a type position reads a module
/// as a table of them: `io.File` is the file type.
#[test]
fn a_module_type_is_named_in_a_type_position() {
    let err = run(r#"
local f: io.File = nil
return f
"#)
    .unwrap_err();
    let err = err.to_string();
    assert!(err.contains("File"), "{err}");
    assert!(!err.contains("unsupported access"), "{err}");
}
