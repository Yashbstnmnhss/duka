//! What a reader gets when they ask the editor about a name.
//!
//! The language server resolves `hover` and `goto` against the symbol table that
//! scope analysis fills, and it does not consult the type checker. So a name the
//! type checker knows and this table does not is a name the editor cannot answer
//! for, and the answer it gives instead is `any` -- a name that looks fine and is
//! wrong in exactly the way that costs time.
//!
//! `where` is where this showed up. `type C = T` was declared inside the type
//! checker, so `(int, C, ...)` in the signature above it hovered as `any`.

use duka_lib::harness::{run, scope_symbols};
use duka_shared::utils::SymbolType;

/// Every name the table knows, with the kind it recorded.
fn names(src: &str) -> Vec<(String, String)> {
    let table = scope_symbols(src).expect("analyzes");
    table
        .scopes
        .iter()
        .flat_map(|scope| scope.symbols.iter())
        .map(|(name, symbols)| {
            let kind = symbols
                .first()
                .map(|symbol| format!("{:?}", symbol.symbol_type))
                .unwrap_or_default();
            (name.to_string(), kind)
        })
        .collect()
}

fn kind_of(src: &str, name: &str) -> Option<String> {
    names(src)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, kind)| kind)
}

#[test]
fn a_where_binding_is_a_name_the_editor_knows() {
    let src = r#"
function B<T: [V], V>(a: T, b: array<T!>!, ...): (int, C, ...)
where type C = T
    return 1, 2
end
return nil
"#;
    let table = names(src);
    assert_eq!(
        kind_of(src, "C").as_deref(),
        Some(r#"TypeBinding { source: "T" }"#),
        "a `where type C = T` should be reachable as a type name: {table:?}"
    );
}

/// The type parameter itself is still a type parameter. The binding above is an
/// addition, not a replacement, and a reader hovering `T` should not be told it
/// is a binding.
#[test]
fn a_type_parameter_is_still_a_type_parameter() {
    let src = r#"
function B<T: [V], V>(a: T, ...): int
where type C = T
    return 0
end
return nil
"#;
    let kind = kind_of(src, "T").expect("T is declared");
    assert!(kind.starts_with("TypeParam"), "{kind}");
}

/// Without the `where`, `C` is not a name. The previous test is only worth
/// something if this one still holds: it is what makes the absence of an entry a
/// decision rather than an accident of how the table is built.
#[test]
fn a_name_no_where_introduces_is_not_in_the_table() {
    let src = r#"
function B<T>(a: T, ...): int
    return 0
end
return nil
"#;
    assert_eq!(kind_of(src, "C"), None);
}

/// A `where` binding is written *below* the signature it serves, and the name
/// has to resolve there.
///
/// This is the other half, and it is a separate defect from the `any` above: that
/// one was the type checker reading the signature before the clause ran, this one
/// is the editor not finding the name at all. A reader hovers `C` in
/// `(int, C, ...)` -- text that sits above the `where` that gives `C` its
/// meaning -- and the lookup takes the declaration nearest *above* the cursor,
/// so a declaration one line lower was not a candidate.
///
/// `resolve_at` is what the hover falls back on for a name in a type position,
/// so this asks it exactly the question the editor asks.
#[test]
fn a_binding_resolves_where_it_is_written_and_above() {
    let src = r#"
function B<T: [V], V>(a: T, b: array<T!>!, ...): (int, C, ...)
where type C = T
    return 1, 2
end
return nil
"#;
    let table = scope_symbols(src).expect("analyzes");

    // On the clause itself, and on the signature above it: the same name, two
    // positions, and the one above is the one a reader is actually asking about.
    let on_clause = table
        .resolve_at((2, 6), "C")
        .expect("written on the where line");
    let in_signature = table
        .resolve_at((1, 55), "C")
        .expect("read in the signature above the where");

    for symbol in [on_clause, in_signature] {
        assert!(
            matches!(symbol.symbol_type, SymbolType::TypeBinding { .. }),
            "{:?}",
            symbol.symbol_type
        );
    }
}

/// The signature has to *use* the binding, not merely carry the name.
/// `(int, C, ...)` returns `2`, so if `C` resolved to `string` -- which is what
/// the clause says -- the annotation and the returned value disagree and the
/// program is rejected. It was accepted, because an unresolved name in an
/// annotation reads as `any` rather than as an error, so `any` accepted the `2`
/// and the signature quietly held `any` where `C` was written. `(int, C, ...)`
/// returns `2`, so if `C` resolved to `string` -- which is what the clause says
/// -- the annotation and the returned value disagree and the program is rejected.
/// It was accepted, because an unresolved name in an annotation reads as `any`
/// rather than as an error, so `any` accepted the `2` and the signature quietly
/// held `any` where `C` was written.
///
/// The `where` has to be present for this to mean anything: the same signature
/// without it is an unresolved name, which is an error, and that is the
/// difference the next test pins down.
#[test]
fn a_where_binding_is_the_type_it_binds_in_the_signature() {
    let err = run(r#"
function B<T>(a: T, ...): (int, C, ...)
where type C = string
    return 1, 2
end
return nil
"#)
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("string"), "{message}");
}

/// The same signature with the `where` gone. `C` is then a name nothing
/// introduces, and it is reported rather than absorbed -- the difference between
/// an error and `any` that the previous test depends on.
#[test]
fn the_same_signature_without_the_where_is_an_unresolved_name() {
    let err = run(r#"
function B<T>(a: T, ...): (int, C, ...)
    return 1, 2
end
return nil
"#)
    .unwrap_err();
    assert!(err.to_string().contains("Unknown type"), "{err}");
}
