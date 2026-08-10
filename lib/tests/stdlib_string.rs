use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
    let lexer = Lexer::new(Cursor::new(src), None, Default::default());
    let stream = lexer.tokenize().map_err(|e| format!("{e}"))?;
    let chunk = Parser::parse(stream, Default::default()).map_err(|e| format!("{e}"))?;
    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(format!("{err}"));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);
    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

fn s(src: &str) -> Result<String, String> {
    Ok(dbg!(
        run(src)?
            .last()
            .cloned()
            .unwrap_or(RuntimeValue::Nil)
            .eval_to_string()
            .into_owned()
    ))
}

fn args(src: &str) -> Result<Vec<String>, String> {
    Ok(run(src)?
        .iter()
        .map(|v| v.eval_to_string().into_owned())
        .collect())
}

#[test]
fn substr_basic() {
    assert_eq!(s(r#"return string.substr("hello", 2, 3)"#).unwrap(), "llo");
}

#[test]
fn substr_to_end_when_no_count() {
    assert_eq!(s(r#"return string.substr("hello", 3)"#).unwrap(), "lo");
}

#[test]
fn substr_negative_start() {
    // -2 => 倒数第二个字节起
    assert_eq!(s(r#"return string.substr("hello", -2)"#).unwrap(), "lo");
}

#[test]
fn substr_start_beyond_len() {
    assert_eq!(s(r#"return string.substr("hello", 100)"#).unwrap(), "");
}

#[test]
fn substr_count_clamps() {
    assert_eq!(
        s(r#"return string.substr("hello", 1, 999)"#).unwrap(),
        "ello"
    );
}

#[test]
fn slice_half_open() {
    assert_eq!(s(r#"return string.slice("hello", 1, 3)"#).unwrap(), "el");
}

#[test]
fn slice_open_end_when_no_end() {
    assert_eq!(s(r#"return string.slice("hello", 2)"#).unwrap(), "llo");
}

#[test]
fn slice_negative() {
    assert_eq!(s(r#"return string.slice("hello", -3, -1)"#).unwrap(), "ll");
}

#[test]
fn slice_start_ge_end_empty() {
    assert_eq!(s(r#"return string.slice("hello", 3, 1)"#).unwrap(), "");
}

#[test]
fn split_multi() {
    let v = args(r#"local d = string.split("a,b,c", ","); return d[0], d[1], d[2]"#).unwrap();
    assert_eq!(v, vec!["a", "b", "c"]);
}

#[test]
fn split_no_sep() {
    let v = args(r#"local d = string.split("abc", ","); return d[0]"#).unwrap();
    assert_eq!(v, vec!["abc"]);
}

#[test]
fn split_adjacent_sep() {
    let v = args(r#"local d = string.split("a,,b", ","); return d[0], d[1], d[2]"#).unwrap();
    assert_eq!(v, vec!["a", "", "b"]);
}

#[test]
fn split_multi_byte_sep() {
    let v = args(r#"local d = string.split("a::bc:::d", "::"); return d[0], d[1], d[2]"#).unwrap();
    assert_eq!(v, vec!["a", "bc", ":d"]);
}

#[test]
fn split_empty_sep_errors() {
    assert!(run(r#"string.split("abc", "")"#).is_err());
}

#[test]
fn len_ascii() {
    assert_eq!(s(r#"return string.len("hello")"#).unwrap(), "5");
}

#[test]
fn len_multibyte_bytes() {
    assert_eq!(s(r#"return string.len("你好")"#).unwrap(), "6");
}

#[test]
fn upper_ascii() {
    assert_eq!(s(r#"return string.upper("aBc")"#).unwrap(), "ABC");
}

#[test]
fn lower_ascii() {
    assert_eq!(s(r#"return string.lower("AbC")"#).unwrap(), "abc");
}

#[test]
fn trim_whitespace() {
    assert_eq!(s(r#"return string.trim("  hi\t\n")"#).unwrap(), "hi");
}

#[test]
fn trim_all_whitespace() {
    assert_eq!(s(r#"return string.trim("   ")"#).unwrap(), "");
}

#[test]
fn trim_none() {
    assert_eq!(s(r#"return string.trim("hi")"#).unwrap(), "hi");
}

#[test]
fn repeat_three() {
    assert_eq!(s(r#"return string.repeat("ab", 3)"#).unwrap(), "ababab");
}

#[test]
fn repeat_zero() {
    assert_eq!(s(r#"return string.repeat("ab", 0)"#).unwrap(), "");
}

#[test]
fn repeat_negative() {
    assert_eq!(s(r#"return string.repeat("ab", -1)"#).unwrap(), "");
}

#[test]
fn find_present() {
    assert_eq!(
        s(r#"return string.find("hello world", "world")"#).unwrap(),
        "6"
    );
}

#[test]
fn find_absent() {
    assert_eq!(s(r#"return string.find("hello", "xyz")"#).unwrap(), "nil");
}

#[test]
fn find_from() {
    assert_eq!(s(r#"return string.find("aaaa", "aa", 2)"#).unwrap(), "2");
}

#[test]
fn find_empty_sub() {
    assert_eq!(s(r#"return string.find("abcdef", "", 3)"#).unwrap(), "3");
}

#[test]
fn reverse_simple() {
    assert_eq!(s(r#"return string.reverse("abc")"#).unwrap(), "cba");
}

#[test]
fn reverse_non_ascii_no_crash() {
    // 字节反转会切坏 UTF-8,但不允许 panic
    let _ = s(r#"local r = string.reverse("你好"); return "" "#).unwrap();
}

#[test]
fn repeatn_basic() {
    assert_eq!(
        s(r#"return string.repeatn("ab", 3, "-")"#).unwrap(),
        "ab-ab-ab"
    );
}

#[test]
fn repeatn_default_sep() {
    assert_eq!(s(r#"return string.repeatn("x", 3)"#).unwrap(), "xxx");
}

#[test]
fn repeatn_zero_or_negative() {
    assert_eq!(s(r#"return string.repeatn("x", 0)"#).unwrap(), "");
    assert_eq!(s(r#"return string.repeatn("x", -5)"#).unwrap(), "");
}

#[test]
fn repeatn_invalid_type() {
    assert!(run(r#"return string.repeatn("x", "y")"#).is_err());
}
