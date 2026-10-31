//! Regex builtins: pattern first, and matching scans forward from `from`.

use duka_lib::harness::run;

#[test]
fn is_match_scans_the_whole_text() {
    run(r#"
assert(regex.is_match("\\d+", "abc123"))
assert(not regex.is_match("\\d+", "abcdef"))
local re = regex.compile("a+")
assert(re:is_match("baa"))
assert(not re:is_match("bbb"))
"#)
    .unwrap();
}

#[test]
fn search_finds_a_match_after_the_start() {
    run(r#"
local ok, caps = regex.search("(\\d+)", "ab 42")
assert(ok)
assert(caps[0] == "42")
"#)
    .unwrap();
}

#[test]
fn replace_takes_the_pattern_first() {
    run(r#"
assert(regex.replace_all("\\s+", "a  b", "-") == "a-b")
assert(regex.replace("\\s+", "a  b  c", "-") == "a-b  c")
"#)
    .unwrap();
}
