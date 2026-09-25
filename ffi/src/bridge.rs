use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
};

use libffi::middle::{Cif, Type};
use libloading::Library;

use crate::cdef::{CBaseType, CDecls, CEnum, CStruct, CTag, CType, FFIError, Sign};

#[derive(Debug, Clone)]
/// Get FFI type from CDecls
pub(crate) struct Layouts {
    visiting: HashSet<usize>,

    tags_cache: HashMap<usize, Type>,
    cif_cache: HashMap<String, Cif>,
}

impl Layouts {
    pub fn new() -> Layouts {
        Layouts {
            visiting: HashSet::new(),
            tags_cache: HashMap::new(),
            cif_cache: HashMap::new(),
        }
    }

    #[inline]
    pub fn clear_cache(&mut self) {
        self.tags_cache.clear();
        self.cif_cache.clear();
    }

    pub fn fn_cif(&mut self, name: &str, decls: &CDecls) -> Result<Option<Cif>, FFIError> {
        if let Some(cif) = self.cif_cache.get(name) {
            return Ok(Some(cif.clone()));
        }

        let Some(sig) = decls.functions.get(name) else {
            return Ok(None);
        };

        let cif = if sig.var_arg {
            Cif::new_variadic(
                sig.params
                    .iter()
                    .map(|(_, ty)| self.ffi_type(ty, decls))
                    .collect::<Result<Vec<_>, _>>()?,
                sig.params.len(),
                self.ffi_type(&sig.ret, decls)?,
            )
        } else {
            Cif::new(
                sig.params
                    .iter()
                    .map(|(_, ty)| self.ffi_type(ty, decls))
                    .collect::<Result<Vec<_>, _>>()?,
                self.ffi_type(&sig.ret, decls)?,
            )
        };

        self.cif_cache.insert(name.to_string(), cif.clone());
        Ok(Some(cif))
    }

    pub fn var_type(&mut self, name: &str, decls: &CDecls) -> Result<Option<Type>, FFIError> {
        let Some(ty) = decls.variables.get(name) else {
            return Ok(None);
        };
        self.ffi_type(ty, decls).map(Some)
    }

    pub fn ffi_type(&mut self, ty: &CType, decls: &CDecls) -> Result<Type, FFIError> {
        Ok(match ty {
            CType::Base(base) => match base {
                CBaseType::Void => Type::void(),
                CBaseType::Bool => Type::u8(),
                CBaseType::Char(sign) => match sign {
                    Sign::Default | Sign::Signed => Type::c_schar(),
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
                CBaseType::LongDouble => {
                    return Err(FFIError::Unsupported(
                        "\"long double\" isn't supported yet".to_owned(),
                    ));
                }
            },

            CType::Ptr(_) => Type::pointer(),
            CType::Array(_, _) => Type::pointer(),
            CType::Function(_) => Type::pointer(),

            CType::Struct(CStruct(_, fields)) => Type::structure(
                fields
                    .iter()
                    .map(|(_, ty)| self.ffi_type(ty, decls))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            CType::Enum(e) => Self::enum_ffi_type(e)?,
            CType::Union(_) => {
                return Err(FFIError::Unsupported(
                    "Union by-value isn't supported".to_owned(),
                ));
            }

            CType::TagRef(r) => {
                if let Some(ty) = self.tags_cache.get(r) {
                    return Ok(ty.clone());
                }

                if !self.visiting.insert(*r) {
                    return Err(FFIError::IncompleteType(
                        "circular reference detected".to_owned(),
                    ));
                }

                let c = decls
                    .get_tag_by_id(*r)
                    .ok_or(FFIError::UnknownType("unknown tag got".to_owned()))?;
                let cty = match c {
                    CTag::Enum(e) => CType::Enum(e.clone()),
                    CTag::Struct(s) => CType::Struct(s.clone()),
                    CTag::Union(u) => CType::Union(u.clone()),
                };
                let ty = self.ffi_type(&cty, decls)?;
                self.visiting.remove(r);
                ty
            }
            CType::TypeRef(r) => {
                if let Some(ty) = decls.get_type(*r) {
                    self.ffi_type(ty, decls)?
                } else {
                    return Err(FFIError::UnknownType("unknown typedef".to_owned()));
                }
            }
        })
    }

    fn enum_ffi_type(e: &CEnum) -> Result<Type, FFIError> {
        if e.1.is_empty() {
            return Err(FFIError::InvalidType("empty enum".into()));
        }

        let mut next = 0isize;
        let mut min = 0isize;
        let mut max = 0isize;
        for (_, v) in &e.1 {
            let val = v.unwrap_or(next);
            min = min.min(val);
            max = max.max(val);
            next = val + 1;
        }

        Ok(if min >= i32::MIN as isize && max <= i32::MAX as isize {
            Type::i32()
        } else if min >= 0 && max <= u32::MAX as isize {
            Type::u32()
        } else if min >= i64::MIN as isize && max <= i64::MAX as isize {
            Type::c_long()
        } else {
            return Err(FFIError::InvalidType("enum values out of range".into()));
        })
    }
}

#[derive(Debug)]
/// Bind CDecls with given Library
pub struct Symbols {
    pub funcs: HashMap<String, *const c_void>,
    pub vars: HashMap<String, *mut c_void>,
    _lib: Library,
}

impl Symbols {
    pub fn bind(decls: &CDecls, lib: Library) -> Self {
        let mut funcs = HashMap::with_capacity(decls.functions.len());
        let mut vars = HashMap::with_capacity(decls.variables.len());
        for (name, _) in &decls.functions {
            if let Ok(sym) = unsafe { lib.get::<*const c_void>(name) } {
                funcs.insert(name.clone(), *sym);
            }
        }
        for (name, _) in &decls.variables {
            if let Ok(sym) = unsafe { lib.get::<*mut c_void>(name) } {
                vars.insert(name.clone(), *sym);
            }
        }
        Self {
            funcs,
            vars,
            _lib: lib,
        }
    }

    #[inline]
    pub fn func(&self, name: &str) -> Option<*const c_void> {
        self.funcs.get(name).copied()
    }
    #[inline]
    pub fn var(&self, name: &str) -> Option<*mut c_void> {
        self.vars.get(name).copied()
    }
}
