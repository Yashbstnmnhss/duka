use crate::parser::ast::Attrs;
use duka_shared::{
    attributes::{AttrError, AttrReturns, AttrUsage, Attribute, Attributes},
    constants::catt,
    effects::{Effect, EffectSet},
    errors::Span,
    utils::OrError,
    value::{AttrValue, ConstValue},
};

#[inline]
fn effect_atom(value: &ConstValue) -> Option<Effect> {
    match value {
        ConstValue::String(s) => std::str::from_utf8(s).ok().and_then(Effect::parse),
        _ => None,
    }
}

pub fn get_effects(attrs: &Attrs, report: &mut dyn FnMut(Box<str>, Span, Box<str>)) -> Option<EffectSet> {
    let entry = attrs.iter().find(|a| a.0.0 == catt::EFFECTS)?;
    let mut set: Option<EffectSet> = None;
    for ((key, key_span), value) in entry.1.iter() {
        let key = key.as_str();
        if key == "deny" {
            let values: Vec<&ConstValue> = match value {
                AttrValue::Single(v) => vec![v],
                AttrValue::Many(vs) => vs.iter().collect(),
            };
            for value in values {
                match effect_atom(value) {
                    Some(effect) => {
                        let effect_set = set.get_or_insert_with(EffectSet::default);
                        if effect_set.deny(effect).is_err() {
                            report(
                                effect.name().into(),
                                *key_span,
                                "is marked and denied at once".into(),
                            );
                        }
                    }
                    None => report(
                        key.into(),
                        *key_span,
                        "expected effect atoms inside deny(...), written as \
                         deny(mutable, external, throw, diverge, nondet, control, ...)"
                            .into(),
                    ),
                }
            }
        } else {
            match Effect::parse(key) {
                Some(effect) => {
                    let effect_set = set.get_or_insert_with(EffectSet::default);
                    if effect_set.with(effect).is_err() {
                        report(
                            effect.name().into(),
                            *key_span,
                            "is marked and denied at once".into(),
                        );
                    }
                }
                None => report(
                    key.into(),
                    *key_span,
                    "unknown effect, atoms are mutable, external, throw, diverge, nondet, \
                     control, or deny(...) to exclude them"
                        .into(),
                ),
            }
        }
    }
    set
}

#[inline]
pub fn get_returns(attrs: &Attrs) -> Option<AttrReturns> {
    attrs.iter().find_map(|a| {
        (a.0.0 == catt::RETURNS)
            .then(|| match a.1.first()?.0.0.as_str() {
                catt::RETURNS_EXIT => Some(AttrReturns::Exit),
                catt::RETURNS_RESULT => Some(AttrReturns::Result),
                _ => None,
            })
            .flatten()
    })
}

#[inline]
pub fn has_declare(attrs: &Attrs) -> bool {
    attrs.iter().any(|a| a.0.0 == catt::DECLARE)
}

pub fn from_attrs(attrs: &Attrs, at: AttrUsage) -> Result<Attributes, (AttrError, Span, String)> {
    let mut res = Attributes::new();
    for ((name, span), properties) in attrs {
        if let Some(mut attr) = Attribute::new(name) {
            attr.get_def()
                .check(at)
                .or_else_error(|| (AttrError::BadUsage, *span, name.to_owned()))?;
            for ((k, s), v) in properties {
                attr.set(k, v.clone())
                    .map_err(|e| (e, *s, name.to_owned()))?;
            }
            res.append(attr).map_err(|e| (e, *span, name.to_owned()))?;
        }
    }
    Ok(res)
}
