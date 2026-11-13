//! `Self`: the type an object or table encloses itself with.
//!
//! These tests stop at IR generation. The VM has an unrelated pre-existing
//! failure on object and multi-return scripts, so running the chunk would say
//! nothing about `Self` here.

use duka_lib::harness::simple_to_ir;

fn ok(src: &str) {
    if let Err(e) = simple_to_ir(src) {
        panic!("expected the source to analyze:\n{e}\n---\n{src}");
    }
}

fn message(src: &str) -> String {
    match simple_to_ir(src) {
        Ok(_) => panic!("expected an error:\n{src}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_table_can_name_itself_in_a_function() {
    ok(r#"
type Handler = { run: function(Self) -> int }
function call(h: Handler): int
    return h:run()
end
return 0
"#);
}

#[test]
fn a_table_can_name_itself_in_a_field() {
    ok(r#"
type Node = { value: int, next: Self? }
local n: Node = { value = 1, next = nil }
n.next = { value = 2, next = nil }
local v: int = n.next.value
return v
"#);
}

#[test]
fn a_nested_table_self_is_the_inner_table() {
    ok(r#"
type Outer = { inner: { run: function(Self) -> int } }
function call(o: Outer): int
    return o.inner:run()
end
return 0
"#);
}

#[test]
fn an_inline_table_can_name_itself() {
    ok(r#"
function call(x: { run: function(Self) -> int }): int
    return x.run(x)
end
return 0
"#);
}

#[test]
fn a_type_function_can_build_a_table_with_self() {
    ok(r#"
type function Node(T) = { value: T, next: Self? }
function use(n: Node(int)): int
    return n.value
end
return 0
"#);
}

#[test]
fn self_in_a_standalone_function_type_is_refused() {
    let text = message("type F = function(Self) -> int\nlocal f: F = nil\nreturn f\n");
    assert!(
        text.contains("only available inside an object or table"),
        "{text}"
    );
}

#[test]
fn an_object_can_name_itself() {
    ok(r#"
object Counter
    v: int = 0
    next: Self? = nil
    function :same(other: Self): bool
        return other.v == self.v
    end
    function create(): Self
        local made: Self = Counter.new()
        return made
    end
end
function run(): bool
    local a = Counter.create()
    local b = Counter.create()
    a.next = b
    return a:same(b)
end
return 0
"#);
}

#[test]
fn self_outside_an_object_or_table_is_reported() {
    let text = message("local x: Self = nil\nreturn x\n");
    assert!(
        text.contains("only available inside an object or table"),
        "{text}"
    );
}

#[test]
fn a_direct_self_reference_still_needs_an_explicit_rec() {
    let text = message("type A = { x: A }\nlocal a: A = nil\nreturn a\n");
    assert!(text.contains("refers to itself"), "{text}");
}

#[test]
fn self_is_reserved_as_a_type_name() {
    let text = message("type Self = int\nlocal x: Self = 1\nreturn x\n");
    assert!(text.contains("Self"), "{text}");
}
