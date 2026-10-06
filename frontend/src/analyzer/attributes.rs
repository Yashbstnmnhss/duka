//! What the attributes written on a declaration mean.
//!
//! An attribute is one syntactic form with many meanings, and the language only
//! gives a meaning to the ones it knows. An attribute it does not know is
//! documentation, so it is left alone rather than guessed at, and one whose
//! argument it does not understand means nothing either.
//!
//! `@name(value)` reads `value` off the argument's name, which is what makes
//! the same form usable here and in the metadata a runtime declares, where
//! `@returns(result)` is a flag with one value.
//!
//! A declaration written by hand and one the runtime declares are read the same
//! way, because a runtime writes them the same way it writes its documentation.

use duka_shared::docs::{Attribute, Returns};

use crate::parser::ast::{Attrs, Name};
use duka_shared::value::ConstValue;

#[inline]
/// The word an attribute is given, for instance `result` in `@returns(result)`.
fn value_of(properties: &[(Name, ConstValue)]) -> Option<&str> {
    properties.first().map(|((name, _), _)| name.as_str())
}

fn of(attrs: &Attrs) -> Vec<Attribute> {
    attrs
        .iter()
        .filter_map(|((name, _), properties)| Attribute::of(name, value_of(properties)))
        .collect()
}

#[inline]
/// What the return slots of the declaration stand for, if it said.
pub fn returns(attrs: &Attrs) -> Option<Returns> {
    of(attrs).into_iter().find_map(|a| a.returns())
}

#[inline]
/// Whether the declaration reads as a keyword rather than as a name.
pub fn is_keywordish(attrs: &Attrs) -> bool {
    of(attrs).into_iter().any(Attribute::is_keywordish)
}

#[inline]
/// The attribute worth showing on the declaration, which is the first one the
/// language gives a meaning to.
pub fn first(attrs: &Attrs) -> Option<Attribute> {
    of(attrs).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_shared::errors::Span;

    fn name(text: &str) -> Name {
        (String::from(text), Span::EMPTY)
    }

    fn attrs(pairs: &[(&str, &[&str])]) -> Attrs {
        pairs
            .iter()
            .map(|(attr, values)| {
                let properties = values
                    .iter()
                    .map(|v| (name(v), ConstValue::Bool(true)))
                    .collect::<Vec<_>>()
                    .into();
                (name(attr), properties)
            })
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn an_attribute_the_language_does_not_know_means_nothing() {
        assert!(returns(&attrs(&[("whatever", &["result"])])).is_none());
        assert!(!is_keywordish(&attrs(&[("whatever", &[])])));
        assert_eq!(first(&attrs(&[("whatever", &[])])), None);
    }

    #[test]
    fn a_return_protocol_is_read_off_the_argument() {
        assert_eq!(
            returns(&attrs(&[("returns", &["result"])])),
            Some(Returns::Result)
        );
        assert_eq!(
            returns(&attrs(&[("returns", &["exit"])])),
            Some(Returns::Exit)
        );
        assert_eq!(returns(&attrs(&[("returns", &[])])), None);
        assert_eq!(returns(&attrs(&[("returns", &["nonsense"])])), None);
    }

    #[test]
    fn keywordish_needs_no_argument() {
        assert!(is_keywordish(&attrs(&[("keywordish", &[])])));
        assert!(is_keywordish(&attrs(&[("keywordish", &["yes"])])));
        assert!(!is_keywordish(&attrs(&[("returns", &["result"])])));
    }

    #[test]
    fn a_declaration_can_carry_several() {
        let both = attrs(&[("returns", &["result"]), ("keywordish", &[])]);
        assert_eq!(returns(&both), Some(Returns::Result));
        assert!(is_keywordish(&both));
        assert_eq!(first(&both), Some(Attribute::Returns(Returns::Result)));
    }
}
