//! Comments kept in the token stream, which is how the editor reads a file
//!
//! The editor lexes with `keep_comment` on so it can colour the comments, and
//! the parser then has to read the language as if they were not there. These
//! are the shapes where that used to go wrong.

use std::io::Cursor;

use duka_lib::duka_frontend::lexer::LexerWithMacro;
use duka_lib::duka_frontend::parser::Parser;
use duka_lib::duka_shared::config::DukaLexerConfig;
use duka_lib::duka_shared::types::{DukaLexer, SourceName};

/// Parse the way the editor does, comments included, and report what it said.
fn parse(text: &str) -> Vec<String> {
    let lexer = LexerWithMacro::new(
        Cursor::new(text),
        SourceName::Virtual("test".into()),
        DukaLexerConfig { keep_comment: true },
    );
    let tokens = lexer
        .tokenize()
        .unwrap_or_else(|e| panic!("lex failed: {e}"));
    let (_chunk, errors) = Parser::parse_lenient(tokens, Default::default());
    errors.iter().map(|e| e.to_string()).collect()
}

fn assert_parses(name: &str, text: &str) {
    let errors = parse(text);
    assert!(errors.is_empty(), "{name}: {errors:?}");
}

#[test]
fn a_comment_after_a_type_function_header_is_a_comment() {
    // The header is reported at `a` when this goes wrong, and that is the tell:
    // the mistake is in the header, the failure is in the window past it.
    assert_parses(
        "between signature and body",
        "type function G(a: table)\n    -- FIXME\n    return\nend\n",
    );
    assert_parses(
        "between the signature and `end`",
        "type function G(a: table)\n    -- FIXME\nend\n",
    );
    assert_parses(
        "inside the parentheses",
        "type function G(a: table -- FIXME\n)\n    return\nend\n",
    );
    assert_parses(
        "after the opening parenthesis",
        "type function G(\n    -- FIXME\n    a: table)\n    return\nend\n",
    );
}

#[test]
fn a_comment_after_a_type_function_without_parameters_is_a_comment() {
    assert_parses(
        "no parameters",
        "type function G()\n    -- FIXME\n    return\nend\n",
    );
}

#[test]
fn a_comment_after_an_ordinary_header_is_a_comment() {
    // The ordinary `function` already tolerated these; they are here so the two
    // stay in step, since they share one header reader.
    assert_parses(
        "plain function",
        "function G(a: table)\n    -- FIXME\n    return 1\nend\n",
    );
    assert_parses(
        "generic function with a where",
        concat!(
            "function B<T, ...V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
            "    where T: [V], type C = [V, T]\n",
            "    return 1, false\n",
            "end\n",
        ),
    );
}

#[test]
fn several_comments_in_a_row_are_all_comments() {
    assert_parses(
        "two comments",
        "type function G(a: table)\n    -- one\n    -- two\n    return\nend\n",
    );
}

#[test]
fn a_comment_does_not_shift_what_follows_it() {
    // The window after a comment is what the header reader looks at, so a
    // comment in front of a statement must not change how that statement reads.
    for text in [
        "local a = 1\nlocal b = 2\nreturn a + b\n",
        "local a = 1\n-- between\nlocal b = 2\n-- between\nreturn a + b\n",
        "local a = 1 -- trailing\nlocal b = 2\nreturn a + b\n",
    ] {
        assert_parses(text, text);
    }
}
