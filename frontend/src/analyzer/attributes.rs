use crate::parser::ast::Attrs;
use duka_shared::{
    attributes::{AttrError, AttrReturns, AttrUsage, Attribute, Attributes},
    constants::catt,
    effects::{EFFECT_ATOMS, Effect, EffectSet},
    errors::Span,
    utils::{OrError, TryDo},
    value::{AttrValue, ConstValue},
};

#[inline]
fn effect_atom(value: &ConstValue) -> Option<Effect> {
    match value {
        ConstValue::String(s) => std::str::from_utf8(s).ok().and_then(Effect::parse),
        _ => None,
    }
}

pub fn get_effects(attrs: &Attrs) -> TryDo<EffectSet, (String, String, Span)> {
    let Some(entry) = attrs.iter().find(|a| a.0.0 == catt::EFFECTS) else {
        return Ok(None);
    };
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
                            return Err((
                                effect.name().into(),
                                format!("{} is marked and denied at once", effect.name()),
                                *key_span,
                            ));
                        }
                    }
                    None => continue,
                }
            }
        } else {
            match Effect::parse(key) {
                Some(effect) => {
                    let effect_set = set.get_or_insert_with(EffectSet::default);
                    if effect_set.with(effect).is_err() {
                        return Err((
                            effect.name().into(),
                            format!("{} is marked and denied at once", effect.name()),
                            *key_span,
                        ));
                    }
                }
                None => {
                    return Err((
                        key.to_string(),
                        format!(
                            "unknown effect \"{key}\", atoms are {}, or deny(...) to exclude them explicitly",
                            EFFECT_ATOMS
                                .iter()
                                .map(|e| e.name())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        *key_span,
                    ));
                }
            }
        }
    }
    Ok(set)
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
