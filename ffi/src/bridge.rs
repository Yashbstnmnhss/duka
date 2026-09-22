use std::collections::HashSet;

use libffi::middle::{Arg, Type};
use libloading::Library;

use crate::{
    cdef::{CBaseType, CDecls, CDefError, CStruct, CTag, CType, Sign},
    layout::{Target, enum_info},
};

pub struct Bridge<'a> {
    decls: &'a CDecls,
    visiting: HashSet<usize>,
}

impl<'a> Bridge<'a> {
    pub fn new(decls: &'a CDecls) -> Bridge<'a> {
        Bridge {
            decls,
            visiting: HashSet::new(),
        }
    }

    fn ffi_type(&mut self, ty: &CType) -> Result<Type, CDefError> {
        Ok(match ty {
            CType::Base(base) => match base {
                CBaseType::Void => Type::void(),
                CBaseType::Bool => Type::u8(),
                CBaseType::Char(sign) => match sign {
                    Sign::Default => Type::u8(),
                    Sign::Signed => Type::c_schar(),
                    Sign::Unsigned => Type::c_uchar(),
                },
                CBaseType::Short(sign) => {
                    if matches!(sign, Sign::Unsigned) {
                        Type::u16()
                    } else {
                        Type::i16()
                    }
                }
                CBaseType::Int(sign) => {
                    if matches!(sign, Sign::Unsigned) {
                        Type::c_uint()
                    } else {
                        Type::c_int()
                    }
                }
                CBaseType::Long(sign) => {
                    if matches!(sign, Sign::Unsigned) {
                        Type::c_ulong()
                    } else {
                        Type::c_long()
                    }
                }
                CBaseType::LongLong(sign) => {
                    if matches!(sign, Sign::Unsigned) {
                        Type::c_ulonglong()
                    } else {
                        Type::c_longlong()
                    }
                }
                CBaseType::Float => Type::f32(),
                CBaseType::Double => Type::f64(),
                CBaseType::LongDouble => unimplemented!(),
            },

            CType::Ptr(_) => Type::pointer(),
            CType::Array(_, _) => Type::pointer(),
            CType::Function(_) => Type::pointer(),

            CType::Struct(CStruct(_, fields)) => Type::structure(
                fields
                    .iter()
                    .map(|(_, ty)| self.ffi_type(ty))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            CType::Enum(cenum) => {
                if let Some(((size, _), signed)) = enum_info(cenum, &Target::HOST) {
                    match size {
                        4 if signed => Type::i32(),
                        4 => Type::u32(),
                        8 if signed => Type::c_long(),
                        8 => Type::c_ulong(),
                        _ => panic!("Invalid enum size"),
                    }
                } else {
                    panic!("Invalid enum")
                }
            }
            CType::Union(_) => panic!("Union isn't supported"),

            CType::TagRef(r) => {
                if !self.visiting.insert(*r) {
                    return Err(CDefError::IncompleteType("circular reference".to_owned()));
                }

                let c = self
                    .decls
                    .get_tag_by_id(*r)
                    .ok_or(CDefError::UnknownType("unknown tag".to_owned()))?;
                let cty = match c {
                    CTag::Enum(e) => CType::Enum(e.clone()),
                    CTag::Struct(s) => CType::Struct(s.clone()),
                    CTag::Union(u) => CType::Union(u.clone()),
                };
                let ty = self.ffi_type(&cty)?;
                self.visiting.remove(r);
                ty
            }
            CType::TypeRef(r) => {
                if let Some(ty) = self.decls.get_type(*r) {
                    self.ffi_type(ty)?
                } else {
                    return Err(CDefError::UnknownType("unknown typedef".to_owned()));
                }
            }
        })
    }
}
