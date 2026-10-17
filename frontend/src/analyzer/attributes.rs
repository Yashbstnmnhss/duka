use crate::parser::ast::Attrs;
use duka_shared::{
    attributes::{AttrError, AttrReturns, AttrUsage, Attribute, Attributes},
    constants::catt,
    errors::Span,
    utils::OrError,
};

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
        if let Some(mut attr) = Attribute::new(&name) {
            attr.get_def()
                .check(at)
                .or_else_error(|| (AttrError::BadUsage, *span, name.to_owned()))?;
            for ((k, s), v) in properties {
                attr.set(&k, v.clone())
                    .map_err(|e| (e, *s, name.to_owned()))?;
            }
            res.append(attr).map_err(|e| (e, *span, name.to_owned()))?;
        }
    }
    Ok(res)
}
