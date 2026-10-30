use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    os::raw::{c_long, c_ulong},
};

use libffi::middle::{Arg, Cif, Type};
use libloading::Library;

use crate::cdef::{CBaseType, CDecls, CEnum, CFnSig, CStruct, CTag, CType, FFIError, Sign};

/// Which C integer an enum is laid out as.
///
/// The C standard leaves the choice to the implementation; this follows the
/// smallest of `int`, `unsigned int` and `long` that holds every value, which
/// is what the CIF and the return value have to agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumKind {
    I32,
    U32,
    Long,
}

pub(crate) fn enum_kind(e: &CEnum) -> Result<EnumKind, FFIError> {
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
        EnumKind::I32
    } else if min >= 0 && max <= u32::MAX as isize {
        EnumKind::U32
    } else if min >= i64::MIN as isize && max <= i64::MAX as isize {
        EnumKind::Long
    } else {
        return Err(FFIError::InvalidType("enum values out of range".into()));
    })
}

#[derive(Debug, Clone)]
/// Get FFI type from CDecls
pub(crate) struct Layouts {
    visiting: HashSet<usize>,

    tags_cache: HashMap<usize, Type>,
    cif_cache: HashMap<String, (Cif, CFnSig)>,
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

    pub fn fn_cif(
        &mut self,
        name: &str,
        decls: &CDecls,
    ) -> Result<Option<(Cif, CFnSig)>, FFIError> {
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

        self.cif_cache
            .insert(name.to_string(), (cif.clone(), sig.clone()));
        Ok(Some((cif, sig.clone())))
    }

    pub fn var_type(&mut self, name: &str, decls: &CDecls) -> Result<Option<Type>, FFIError> {
        let Some(ty) = decls.variables.get(name) else {
            return Ok(None);
        };
        self.ffi_type(ty, decls).map(Some)
    }

    /// The CIF of one variadic call.
    ///
    /// A variadic CIF has to know the types of the arguments written after the
    /// fixed list, and those are only known per call -- so this one is not
    /// cached, unlike [`Self::fn_cif`].
    pub fn variadic_cif(
        &mut self,
        sig: &CFnSig,
        args: &[Value],
        decls: &CDecls,
    ) -> Result<Cif, FFIError> {
        let mut types: Vec<Type> = sig
            .params
            .iter()
            .map(|(_, ty)| self.ffi_type(ty, decls))
            .collect::<Result<_, _>>()?;
        for arg in args.iter().skip(sig.params.len()) {
            types.push(arg.ffi_type()?);
        }
        Ok(Cif::new_variadic(
            types,
            sig.params.len(),
            self.ffi_type(&sig.ret, decls)?,
        ))
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
        Ok(match enum_kind(e)? {
            EnumKind::I32 => Type::i32(),
            EnumKind::U32 => Type::u32(),
            EnumKind::Long => Type::c_long(),
        })
    }
}

#[derive(Debug)]
/// Bind CDecls with given Library
pub struct Symbols {
    pub funcs: HashMap<String, *const c_void>,
    pub vars: HashMap<String, *mut c_void>,
    lib: Library,
}

impl Symbols {
    pub fn rebind(&mut self, decls: &CDecls) {
        for name in decls.functions.keys() {
            if let Ok(sym) = unsafe { self.lib.get::<*const c_void>(name) } {
                self.funcs.insert(name.clone(), *sym);
            }
        }
        for name in decls.variables.keys() {
            if let Ok(sym) = unsafe { self.lib.get::<*mut c_void>(name) } {
                self.vars.insert(name.clone(), *sym);
            }
        }
    }

    pub fn bind(decls: &CDecls, lib: Library) -> Self {
        let mut syms = Self {
            funcs: HashMap::with_capacity(decls.functions.len()),
            vars: HashMap::with_capacity(decls.variables.len()),
            lib,
        };
        syms.rebind(decls);
        syms
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

#[derive(Debug, Clone)]
pub enum Value {
    Void,
    Long(i64),
    ULong(u64),
    LongLong(i64),
    ULongLong(u64),
    Int(i32),
    UInt(u32),
    Short(i16),
    UShort(u16),
    Char(i8),
    UChar(u8),
    Float(f32),
    Double(f64),
    Ptr(*mut c_void),
}

impl Value {
    pub fn check_type(&self, ty: &CType) -> bool {
        match (self, ty) {
            (Self::Void, CType::Base(CBaseType::Void))
            | (Self::Int(_), CType::Base(CBaseType::Int(Sign::Default | Sign::Signed)))
            | (Self::UInt(_), CType::Base(CBaseType::Int(Sign::Unsigned)))
            | (Self::Short(_), CType::Base(CBaseType::Short(Sign::Default | Sign::Signed)))
            | (Self::UShort(_), CType::Base(CBaseType::Short(Sign::Unsigned)))
            | (Self::Long(_), CType::Base(CBaseType::Long(Sign::Default | Sign::Signed)))
            | (Self::ULong(_), CType::Base(CBaseType::Long(Sign::Unsigned)))
            | (Self::LongLong(_), CType::Base(CBaseType::LongLong(Sign::Default | Sign::Signed)))
            | (Self::ULongLong(_), CType::Base(CBaseType::LongLong(Sign::Unsigned)))
            | (Self::Char(_), CType::Base(CBaseType::Char(Sign::Default | Sign::Signed)))
            | (Self::UChar(_), CType::Base(CBaseType::Char(Sign::Unsigned)))
            | (Self::UChar(_), CType::Base(CBaseType::Bool))
            | (Self::Int(_), CType::Base(CBaseType::Bool))
            | (Self::Float(_), CType::Base(CBaseType::Float))
            | (Self::Double(_), CType::Base(CBaseType::Double))
            | (Self::Ptr(_), CType::Ptr(_))
            | (Self::Ptr(_), CType::Array(..))
            | (Self::Ptr(_), CType::Function(_)) => true,
            _ => false,
        }
    }

    /// The integer this value is, if it is one.
    pub fn as_integer(&self) -> Option<i128> {
        Some(match self {
            Value::Char(v) => *v as i128,
            Value::UChar(v) => *v as i128,
            Value::Short(v) => *v as i128,
            Value::UShort(v) => *v as i128,
            Value::Int(v) => *v as i128,
            Value::UInt(v) => *v as i128,
            Value::Long(v) | Value::LongLong(v) => *v as i128,
            Value::ULong(v) | Value::ULongLong(v) => *v as i128,
            _ => return None,
        })
    }

    /// This value written as the C type says, which is what a call needs.
    ///
    /// A Duka number is one number while C has a width per parameter, so the
    /// caller hands over what it has and the argument is narrowed or widened
    /// here rather than at every call site.
    pub fn coerce(self, ty: &CType) -> Result<Value, FFIError> {
        let int = |v: &Value| {
            v.as_integer()
                .ok_or_else(|| FFIError::Mismatch(format!("{v:?}"), format!("{ty:?}")))
        };
        Ok(match ty {
            CType::Base(base) => match base {
                CBaseType::Void => Value::Void,
                CBaseType::Bool => Value::UChar((int(&self)? != 0) as u8),
                CBaseType::Char(sign) => match sign {
                    Sign::Unsigned => Value::UChar(int(&self)? as u8),
                    _ => Value::Char(int(&self)? as i8),
                },
                CBaseType::Short(sign) => match sign {
                    Sign::Unsigned => Value::UShort(int(&self)? as u16),
                    _ => Value::Short(int(&self)? as i16),
                },
                CBaseType::Int(sign) => match sign {
                    Sign::Unsigned => Value::UInt(int(&self)? as u32),
                    _ => Value::Int(int(&self)? as i32),
                },
                CBaseType::Long(sign) => match sign {
                    Sign::Unsigned => Value::ULong(int(&self)? as u64),
                    _ => Value::Long(int(&self)? as i64),
                },
                CBaseType::LongLong(sign) => match sign {
                    Sign::Unsigned => Value::ULongLong(int(&self)? as u64),
                    _ => Value::LongLong(int(&self)? as i64),
                },
                CBaseType::Float => Value::Float(match self {
                    Value::Float(f) => f,
                    Value::Double(d) => d as f32,
                    other => int(&other)? as f32,
                }),
                CBaseType::Double => Value::Double(match self {
                    Value::Double(d) => d,
                    Value::Float(f) => f as f64,
                    other => int(&other)? as f64,
                }),
                CBaseType::LongDouble => {
                    return Err(FFIError::Unsupported(
                        "\"long double\" isn't supported yet".to_owned(),
                    ));
                }
            },
            CType::Ptr(_) | CType::Array(..) | CType::Function(_) => match self {
                Value::Ptr(p) => Value::Ptr(p),
                Value::Void => Value::Ptr(std::ptr::null_mut()),
                other => Value::Ptr(int(&other)? as usize as *mut c_void),
            },
            CType::Enum(e) => match enum_kind(e)? {
                EnumKind::I32 => Value::Int(int(&self)? as i32),
                EnumKind::U32 => Value::UInt(int(&self)? as u32),
                EnumKind::Long => Value::Long(int(&self)? as i64),
            },
            CType::Struct(_) | CType::Union(_) => {
                return Err(FFIError::Unsupported(format!(
                    "{ty:?} passed by value isn't supported"
                )));
            }
            CType::TagRef(_) | CType::TypeRef(_) => {
                return Err(FFIError::UnknownType(format!("{ty:?}")));
            }
        })
    }

    /// What C's default argument promotions make of this value, which is what a
    /// variadic call passes.
    pub fn promote(self) -> Value {
        match self {
            Value::Char(v) => Value::Int(v as i32),
            Value::UChar(v) => Value::Int(v as i32),
            Value::Short(v) => Value::Int(v as i32),
            Value::UShort(v) => Value::Int(v as i32),
            Value::Float(f) => Value::Double(f as f64),
            other => other,
        }
    }

    /// The C type a variadic argument is passed as.
    ///
    /// The value already carries its width, and C's default argument promotions
    /// are the caller's business: an `int` is passed as one and a `float` as a
    /// `double`.
    pub fn ffi_type(&self) -> Result<Type, FFIError> {
        Ok(match self {
            Value::Void => return Err(FFIError::Unsupported("Void in argument".to_owned())),
            Value::Char(_) | Value::UChar(_) | Value::Short(_) | Value::UShort(_) => Type::i32(),
            Value::Int(_) | Value::UInt(_) => Type::i32(),
            Value::Long(_) => Type::c_long(),
            Value::ULong(_) => Type::c_ulong(),
            Value::LongLong(_) => Type::i64(),
            Value::ULongLong(_) => Type::u64(),
            Value::Float(_) => Type::f64(),
            Value::Double(_) => Type::f64(),
            Value::Ptr(_) => Type::pointer(),
        })
    }

    pub fn to_arg<'a>(&'a self) -> Result<Arg<'a>, FFIError> {
        Ok(match self {
            Value::Void => return Err(FFIError::Unsupported("Void in argument".to_owned())),
            Value::Int(i) => Arg::new(i),
            Value::UInt(u) => Arg::new(u),
            Value::Float(f) => Arg::new(f),
            Value::Double(d) => Arg::new(d),
            Value::Ptr(p) => Arg::new(p),
            Value::Long(l) => Arg::new(l),
            Value::ULong(u) => Arg::new(u),
            Value::LongLong(l) => Arg::new(l),
            Value::ULongLong(u) => Arg::new(u),
            Value::Short(s) => Arg::new(s),
            Value::UShort(u) => Arg::new(u),
            Value::Char(c) => Arg::new(c),
            Value::UChar(u) => Arg::new(u),
        })
    }
}

/// The value stored where `ptr` points, read as `ty`.
///
/// The type is the one a declaration resolved to, so it is a base type, a
/// pointer, an array or an enum; an aggregate is read through its address
/// rather than copied, which is what `read_ptr` is for.
pub fn read_c_value(ptr: *const c_void, ty: &CType) -> Result<Value, FFIError> {
    unsafe {
        Ok(match ty {
            CType::Base(base) => match base {
                CBaseType::Void => Value::Void,
                CBaseType::Bool => Value::UChar(ptr.cast::<u8>().read_unaligned()),
                CBaseType::Char(sign) => match sign {
                    Sign::Unsigned => Value::UChar(ptr.cast::<u8>().read_unaligned()),
                    _ => Value::Char(ptr.cast::<i8>().read_unaligned()),
                },
                CBaseType::Short(sign) => match sign {
                    Sign::Unsigned => Value::UShort(ptr.cast::<u16>().read_unaligned()),
                    _ => Value::Short(ptr.cast::<i16>().read_unaligned()),
                },
                CBaseType::Int(sign) => match sign {
                    Sign::Unsigned => Value::UInt(ptr.cast::<u32>().read_unaligned()),
                    _ => Value::Int(ptr.cast::<i32>().read_unaligned()),
                },
                CBaseType::Long(sign) => match sign {
                    Sign::Unsigned => Value::ULong(ptr.cast::<c_ulong>().read_unaligned() as u64),
                    _ => Value::Long(ptr.cast::<c_long>().read_unaligned() as i64),
                },
                CBaseType::LongLong(sign) => match sign {
                    Sign::Unsigned => Value::ULongLong(ptr.cast::<u64>().read_unaligned()),
                    _ => Value::LongLong(ptr.cast::<i64>().read_unaligned()),
                },
                CBaseType::Float => Value::Float(ptr.cast::<f32>().read_unaligned()),
                CBaseType::Double => Value::Double(ptr.cast::<f64>().read_unaligned()),
                CBaseType::LongDouble => {
                    return Err(FFIError::Unsupported(
                        "\"long double\" isn't supported yet".to_owned(),
                    ));
                }
            },
            CType::Ptr(_) | CType::Array(..) | CType::Function(_) => {
                Value::Ptr(ptr.cast::<*mut c_void>().read_unaligned())
            }
            CType::Enum(e) => match enum_kind(e)? {
                EnumKind::I32 => Value::Int(ptr.cast::<i32>().read_unaligned()),
                EnumKind::U32 => Value::UInt(ptr.cast::<u32>().read_unaligned()),
                EnumKind::Long => Value::Long(ptr.cast::<c_long>().read_unaligned() as i64),
            },
            CType::Struct(_) | CType::Union(_) => {
                return Err(FFIError::Unsupported(format!(
                    "{ty:?} read by value isn't supported"
                )));
            }
            CType::TagRef(_) | CType::TypeRef(_) => {
                return Err(FFIError::UnknownType(format!("{ty:?}")));
            }
        })
    }
}

/// `value` written where `ptr` points, as `ty`.
pub fn write_c_value(ptr: *mut c_void, ty: &CType, value: Value) -> Result<(), FFIError> {
    let value = value.coerce(ty)?;
    unsafe {
        match (ty, value) {
            (CType::Base(CBaseType::Void), _) => {}
            (CType::Base(CBaseType::Bool), Value::UChar(v)) => ptr.cast::<u8>().write_unaligned(v),
            (CType::Base(CBaseType::Char(_)), Value::Char(v)) => {
                ptr.cast::<i8>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Char(_)), Value::UChar(v)) => {
                ptr.cast::<u8>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Short(_)), Value::Short(v)) => {
                ptr.cast::<i16>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Short(_)), Value::UShort(v)) => {
                ptr.cast::<u16>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Int(_)), Value::Int(v)) => ptr.cast::<i32>().write_unaligned(v),
            (CType::Base(CBaseType::Int(_)), Value::UInt(v)) => {
                ptr.cast::<u32>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Long(_)), Value::Long(v)) => {
                ptr.cast::<c_long>().write_unaligned(v as c_long)
            }
            (CType::Base(CBaseType::Long(_)), Value::ULong(v)) => {
                ptr.cast::<c_ulong>().write_unaligned(v as c_ulong)
            }
            (CType::Base(CBaseType::LongLong(_)), Value::LongLong(v)) => {
                ptr.cast::<i64>().write_unaligned(v)
            }
            (CType::Base(CBaseType::LongLong(_)), Value::ULongLong(v)) => {
                ptr.cast::<u64>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Float), Value::Float(v)) => {
                ptr.cast::<f32>().write_unaligned(v)
            }
            (CType::Base(CBaseType::Double), Value::Double(v)) => {
                ptr.cast::<f64>().write_unaligned(v)
            }
            (CType::Ptr(_) | CType::Array(..) | CType::Function(_), Value::Ptr(v)) => {
                ptr.cast::<*mut c_void>().write_unaligned(v)
            }
            (CType::Enum(e), Value::Int(v)) => {
                debug_assert!(enum_kind(e)? == EnumKind::I32);
                ptr.cast::<i32>().write_unaligned(v)
            }
            (CType::Enum(e), Value::UInt(v)) => {
                debug_assert!(enum_kind(e)? == EnumKind::U32);
                ptr.cast::<u32>().write_unaligned(v)
            }
            (CType::Enum(e), Value::Long(v)) => {
                debug_assert!(enum_kind(e)? == EnumKind::Long);
                ptr.cast::<c_long>().write_unaligned(v as c_long)
            }
            (ty, value) => {
                return Err(FFIError::Mismatch(format!("{value:?}"), format!("{ty:?}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_ffis;

    #[test]
    fn an_enum_is_laid_out_at_the_smallest_int_that_holds_it() {
        let decls = parse_ffis("enum Small { A, B }; enum Negative { X = -1 };").unwrap();
        for name in ["Small", "Negative"] {
            let Some(CTag::Enum(e)) = decls.get_tag(name) else {
                panic!("{name} is an enum");
            };
            assert_eq!(enum_kind(e).unwrap(), EnumKind::I32, "{name}");
        }
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_wide_enum_is_laid_out_wider_than_an_int() {
        let decls = parse_ffis("enum Wide { HUGE = 5000000000 };").unwrap();
        let Some(CTag::Enum(e)) = decls.get_tag("Wide") else {
            panic!("Wide is an enum");
        };
        assert_eq!(enum_kind(e).unwrap(), EnumKind::Long);
    }

    #[test]
    fn a_struct_gets_a_layout_and_its_pointer_is_a_pointer() {
        let decls = parse_ffis("struct Pair { int a; double b; };").unwrap();
        let Some(CTag::Struct(s)) = decls.get_tag("Pair") else {
            panic!("Pair is a struct");
        };
        let mut layouts = Layouts::new();
        assert!(layouts.ffi_type(&CType::Struct(s.clone()), &decls).is_ok());
        let ptr = CType::Ptr(Box::new(CType::Struct(s.clone())));
        assert!(layouts.ffi_type(&ptr, &decls).is_ok());
    }

    #[test]
    fn a_struct_that_contains_itself_is_reported_rather_than_recursed_into() {
        let decls = parse_ffis("struct Loop { struct Loop inner; };").unwrap();
        let Some(CTag::Struct(s)) = decls.get_tag("Loop") else {
            panic!("Loop is a struct");
        };
        let mut layouts = Layouts::new();
        assert!(matches!(
            layouts.ffi_type(&CType::Struct(s.clone()), &decls),
            Err(FFIError::IncompleteType(_))
        ));
    }

    #[test]
    fn an_integer_takes_the_width_the_parameter_asks_for() {
        let longlong = CType::Base(CBaseType::LongLong(Sign::Unsigned));
        assert!(matches!(
            Value::Int(7).coerce(&longlong).unwrap(),
            Value::ULongLong(7)
        ));
        let boolean = CType::Base(CBaseType::Bool);
        assert!(matches!(
            Value::Int(2).coerce(&boolean).unwrap(),
            Value::UChar(1)
        ));
        assert!(matches!(
            Value::Int(0).coerce(&boolean).unwrap(),
            Value::UChar(0)
        ));
        let ptr = CType::Ptr(Box::new(CType::Base(CBaseType::Void)));
        assert!(matches!(
            Value::Int(0).coerce(&ptr).unwrap(),
            Value::Ptr(p) if p.is_null()
        ));
        assert!(matches!(
            Value::Float(1.5).coerce(&CType::Base(CBaseType::Double)).unwrap(),
            Value::Double(f) if f == 1.5
        ));
    }

    #[test]
    fn a_value_written_and_read_back_is_the_same_one() {
        let mut buffer = [0u8; 16];
        let at = buffer.as_mut_ptr().cast::<c_void>();
        for (ty, value) in [
            (CType::Base(CBaseType::Int(Sign::Signed)), Value::Int(-7)),
            (
                CType::Base(CBaseType::Short(Sign::Unsigned)),
                Value::UShort(9),
            ),
            (CType::Base(CBaseType::Double), Value::Double(1.25)),
            (
                CType::Enum(CEnum(None, vec![("A".into(), Some(1))])),
                Value::Int(1),
            ),
        ] {
            write_c_value(at, &ty, value.clone()).unwrap();
            let round = read_c_value(at, &ty).unwrap();
            assert_eq!(format!("{round:?}"), format!("{value:?}"), "{ty:?}");
        }
    }

    #[test]
    fn a_pointer_value_is_written_and_read_back() {
        let mut buffer = [0u8; 8];
        let address = buffer.as_mut_ptr().cast::<c_void>();
        let ty = CType::Ptr(Box::new(CType::Base(CBaseType::Void)));
        write_c_value(buffer.as_mut_ptr().cast(), &ty, Value::Ptr(address)).unwrap();
        let round = read_c_value(buffer.as_ptr().cast(), &ty).unwrap();
        assert!(matches!(round, Value::Ptr(p) if p == address));
    }

    #[test]
    fn a_value_that_is_not_the_parameter_is_a_mismatch() {
        let ty = CType::Base(CBaseType::Int(Sign::Signed));
        assert!(matches!(
            Value::Ptr(std::ptr::null_mut()).coerce(&ty),
            Err(FFIError::Mismatch(..))
        ));
        assert!(matches!(
            Value::Int(1).coerce(&CType::Struct(CStruct(None, vec![]))),
            Err(FFIError::Unsupported(_))
        ));
    }
}
