//! Reading a pattern against a value.
//!
//! A pattern appears in three places -- `local { a } = e`, a `match` arm, and
//! `where type [C, _] = T` -- and each of them used to read it apart on its own.
//! They disagreed: a name took the whole value in one of them and its own place
//! in another, so `type [C, _] = [V, T]` gave `C` the type `[V, T]`.
//!
//! So this is the one walk, and every place that has a pattern to read goes
//! through it. What a pattern binds is decided here and nowhere else.

use duka_shared::{constants::cpar, dtype::Type};

use crate::parser::ast::{DestructingTableTerm, DestructingTerm, Destructuring, Name};

/// One place in a pattern.
pub enum Bind {
    /// A name, and what the value holds at that place -- `None` when the value
    /// says nothing about it, which is not the same as it being `any`.
    Named(Name, Option<Type>),
    /// `_`. Read, and thrown away: it takes no name and gets no type.
    Ignored,
    /// `...`. Everything the pattern did not reach.
    Rest,
}

#[inline]
/// The names `_` and `...` are written with. Neither binds anything, and a
/// pattern that reads them as names would put a `_` in scope.
pub fn is_placeholder(name: &str) -> bool {
    matches!(name, cpar::DISCARD | "...")
}

/// What `pattern` binds, read against `value`.
///
/// A record pattern takes each field by name and an array pattern takes each
/// position, so a name gets what sits where it was written rather than the whole
/// value. A nested pattern reads further into what its own place already is, so
/// `{{ a }}` asks the inner value for `a`.
pub fn bind(pattern: &Destructuring, value: &Type) -> Vec<Bind> {
    let mut out = Vec::new();
    bind_into(pattern, value, &mut out);
    out
}

fn bind_into(pattern: &Destructuring, value: &Type, out: &mut Vec<Bind>) {
    match pattern {
        // a bare name takes the whole value, which is what makes it different
        // from a pattern of one element
        Destructuring::Name(name) => out.push(Bind::Named(name.clone(), Some(value.clone()))),
        Destructuring::Table(terms) => {
            for DestructingTableTerm(field, term) in terms {
                bind_term(term, value.field(field.0.as_str()).as_ref(), out);
            }
        }
        Destructuring::Array(terms) => {
            let mut idx = 0;
            for term in terms.iter() {
                let found = match term {
                    // a rest takes what every place after it left, so it needs
                    // to know how far the pattern has already read
                    DestructingTerm::Rest(..) => at_rest(value, idx),
                    _ => at_index(value, idx),
                };
                bind_term(term, found.as_ref(), out);
                if !matches!(term, DestructingTerm::Rest(..)) {
                    idx += 1;
                }
            }
        }
    }
}

/// What the value holds at one position of an array pattern.
///
/// A tuple says what sits at each position, so that is what is read. A declared
/// `array` has one element type rather than a shape, and every name in an array
/// pattern is told it, which is the only thing an array of them can say.
fn at_index(value: &Type, idx: usize) -> Option<Type> {
    match value {
        Type::TypeTuple(items) => items.get(idx).cloned(),
        Type::Array(Some(inner)) => Some((**inner).clone()),
        _ => None,
    }
}

/// What is left of the value once `consumed` places have been read.
fn at_rest(value: &Type, consumed: usize) -> Option<Type> {
    match value {
        Type::TypeTuple(items) => {
            let rest: Vec<Type> = items.iter().skip(consumed).cloned().collect();
            (!rest.is_empty()).then_some(Type::TypeTuple(rest))
        }
        // one element type is the same however much of it is taken
        Type::Array(Some(inner)) => Some(Type::Array(Some(inner.clone()))),
        _ => None,
    }
}

fn bind_term(term: &DestructingTerm, found: Option<&Type>, out: &mut Vec<Bind>) {
    match term {
        DestructingTerm::Bind(name) => {
            out.push(Bind::Named(name.clone(), found.cloned()));
        }
        DestructingTerm::Ignored(_) => out.push(Bind::Ignored),
        DestructingTerm::Rest(name, _) => match name {
            // `...rest` takes what is left under a name; a bare `...` binds nothing
            Some(name) => out.push(Bind::Named(name.clone(), found.cloned())),
            None => out.push(Bind::Rest),
        },
        DestructingTerm::Term(nested) => {
            // a nested pattern reads further into what this place already is
            let found = found.cloned().unwrap_or(Type::Any);
            bind_into(nested, &found, out);
        }
    }
}

/// Every name `pattern` introduces, in the order they are written.
pub fn names(pattern: &Destructuring) -> Vec<Name> {
    let mut out = Vec::new();
    collect_names(pattern, &mut out);
    out
}

/// Whether a written name introduces a binding at all.
///
/// A pattern that reached `_` before this existed read it as a name, so `local
/// { a, _ }` and a `match` arm alike put a `_` in scope. It is the one question
/// every pattern form has to answer the same way.
pub fn binds(name: &str) -> bool {
    !matches!(name, "_" | "...")
}

fn collect_names(pattern: &Destructuring, out: &mut Vec<Name>) {
    match pattern {
        Destructuring::Name(name) => out.push(name.clone()),
        Destructuring::Table(terms) => terms.iter().for_each(|t| collect_term(&t.1, out)),
        Destructuring::Array(terms) => terms.iter().for_each(|t| collect_term(t, out)),
    }
}

fn collect_term(term: &DestructingTerm, out: &mut Vec<Name>) {
    match term {
        DestructingTerm::Bind(name) => out.push(name.clone()),
        // neither `_` nor `...` introduces a name
        DestructingTerm::Ignored(_) => {}
        DestructingTerm::Rest(None, _) => {}
        DestructingTerm::Rest(Some(name), _) => out.push(name.clone()),
        DestructingTerm::Term(nested) => collect_names(nested, out),
    }
}
