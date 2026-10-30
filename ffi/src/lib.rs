use libffi::low::CodePtr;
use libloading::Library;

use crate::{
    bridge::{Layouts, Symbols, Value, read_c_value, write_c_value},
    cdef::{CBaseType, CDecls, CType, FFIError, Sign},
    parser::parse_ffis,
};

pub mod bridge;
pub mod cdef;
pub mod parser;

pub type LibID = usize;

#[derive(Debug)]
pub struct FFI {
    decls: CDecls,
    layouts: Layouts,
    libs: Vec<Option<Symbols>>,
}

impl Default for FFI {
    fn default() -> Self {
        Self::new()
    }
}

impl FFI {
    #[inline]
    fn get_fn(&self, name: &str, from_lib: Option<LibID>) -> Option<*const std::ffi::c_void> {
        if let Some(id) = from_lib {
            if let Some(Some(lib)) = self.libs.get(id) {
                lib.func(name)
            } else {
                None
            }
        } else {
            self.libs
                .iter()
                .rev()
                .find_map(|f| f.as_ref().and_then(|i| i.func(name)))
        }
    }

    #[inline]
    fn get_var_ptr(&self, name: &str, from_lib: Option<LibID>) -> Option<*mut std::ffi::c_void> {
        if let Some(id) = from_lib {
            if let Some(Some(lib)) = self.libs.get(id) {
                lib.var(name)
            } else {
                None
            }
        } else {
            self.libs
                .iter()
                .rev()
                .find_map(|f| f.as_ref().and_then(|i| i.var(name)))
        }
    }

    /// The signature a declaration wrote under `name`.
    pub fn signature(&self, name: &str) -> Option<&crate::cdef::CFnSig> {
        self.decls.function(name)
    }

    pub fn call(
        &mut self,
        lib: Option<LibID>,
        name: &str,
        args: &[Value],
    ) -> Result<Value, FFIError> {
        let sig = self
            .decls
            .function(name)
            .cloned()
            .ok_or_else(|| FFIError::UnknownFunction(name.to_owned()))?;

        if args.len() < sig.params.len() {
            return Err(FFIError::InvalidParameters(
                name.to_owned(),
                format!(
                    "Expected {} parameters, got {}",
                    sig.params.len(),
                    args.len()
                ),
            ));
        }
        if args.len() > sig.params.len() && !sig.var_arg {
            return Err(FFIError::InvalidParameters(
                name.to_owned(),
                "Too many arguments".to_string(),
            ));
        }

        // what each argument has to be, one by one: a parameter's type is the
        // declaration's, and what follows it in a variadic call is whatever the
        // value already is
        let mut owned: Vec<Value> = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            match sig.params.get(idx) {
                Some((_, ty)) => owned.push(arg.clone().coerce(&self.decls.resolve(ty)?)?),
                None => owned.push(arg.clone().promote()),
            }
        }

        let cif = if sig.var_arg {
            self.layouts.variadic_cif(&sig, &owned, &self.decls)?
        } else {
            match self.layouts.fn_cif(name, &self.decls)? {
                Some((cif, _)) => cif,
                None => return Err(FFIError::UnknownFunction(name.to_owned())),
            }
        };

        let ptr = self
            .get_fn(name, lib)
            .ok_or_else(|| FFIError::UnknownFunction(name.to_owned()))?;
        let arg_refs = owned
            .iter()
            .map(|v| v.to_arg())
            .collect::<Result<Vec<_>, _>>()?;
        let fun = CodePtr::from_ptr(ptr);
        let ret = self.decls.resolve(&sig.ret)?;

        Ok(unsafe {
            match ret {
                CType::Base(base) => match base {
                    CBaseType::Void => {
                        cif.call::<()>(fun, &arg_refs);
                        Value::Void
                    }
                    CBaseType::Bool => Value::Int(cif.call::<i32>(fun, &arg_refs)),
                    CBaseType::Char(sign) => match sign {
                        Sign::Unsigned => Value::UChar(cif.call(fun, &arg_refs)),
                        _ => Value::Char(cif.call(fun, &arg_refs)),
                    },
                    CBaseType::Short(sign) => match sign {
                        Sign::Unsigned => Value::UShort(cif.call(fun, &arg_refs)),
                        _ => Value::Short(cif.call(fun, &arg_refs)),
                    },
                    CBaseType::Int(sign) => match sign {
                        Sign::Unsigned => Value::UInt(cif.call(fun, &arg_refs)),
                        _ => Value::Int(cif.call(fun, &arg_refs)),
                    },
                    CBaseType::Long(sign) => match sign {
                        Sign::Unsigned => Value::ULong(cif.call(fun, &arg_refs)),
                        _ => Value::Long(cif.call(fun, &arg_refs)),
                    },
                    CBaseType::LongLong(sign) => match sign {
                        Sign::Unsigned => Value::ULongLong(cif.call(fun, &arg_refs)),
                        _ => Value::LongLong(cif.call(fun, &arg_refs)),
                    },
                    CBaseType::Float => Value::Float(cif.call(fun, &arg_refs)),
                    CBaseType::Double => Value::Double(cif.call(fun, &arg_refs)),
                    CBaseType::LongDouble => {
                        return Err(FFIError::Unsupported(
                            "\"long double\" isn't supported yet".to_owned(),
                        ));
                    }
                },
                CType::Ptr(_) | CType::Array(..) | CType::Function(_) => {
                    Value::Ptr(cif.call(fun, &arg_refs))
                }
                CType::Enum(e) => match crate::bridge::enum_kind(&e)? {
                    crate::bridge::EnumKind::I32 => Value::Int(cif.call(fun, &arg_refs)),
                    crate::bridge::EnumKind::U32 => Value::UInt(cif.call(fun, &arg_refs)),
                    crate::bridge::EnumKind::Long => {
                        Value::Long(cif.call::<std::os::raw::c_long>(fun, &arg_refs) as i64)
                    }
                },
                CType::Struct(_) | CType::Union(_) => {
                    return Err(FFIError::Unsupported(
                        "returning a struct or union by value isn't supported".to_owned(),
                    ));
                }
                CType::TagRef(_) | CType::TypeRef(_) => {
                    return Err(FFIError::UnknownType(format!("{ret:?}")));
                }
            }
        })
    }

    /// The value a C variable holds right now.
    pub fn get_var(&mut self, lib: Option<LibID>, name: &str) -> Result<Value, FFIError> {
        let declared = self
            .decls
            .variable(name)
            .cloned()
            .ok_or_else(|| FFIError::UnknownSymbol(name.to_owned()))?;
        // a variable has to be a type the bridge can read, which is the same set
        // the CIF knows: asking for the layout is what says so
        let _ = self.layouts.var_type(name, &self.decls)?;
        let ty = self.decls.resolve(&declared)?;
        let ptr = self
            .get_var_ptr(name, lib)
            .ok_or_else(|| FFIError::UnknownSymbol(name.to_owned()))?;
        read_c_value(ptr as *const std::ffi::c_void, &ty)
    }

    /// Writes a C variable, the value read as the declared type.
    pub fn set_var(&mut self, lib: Option<LibID>, name: &str, value: Value) -> Result<(), FFIError> {
        let declared = self
            .decls
            .variable(name)
            .cloned()
            .ok_or_else(|| FFIError::UnknownSymbol(name.to_owned()))?;
        let _ = self.layouts.var_type(name, &self.decls)?;
        let ty = self.decls.resolve(&declared)?;
        let ptr = self
            .get_var_ptr(name, lib)
            .ok_or_else(|| FFIError::UnknownSymbol(name.to_owned()))?;
        write_c_value(ptr, &ty, value)
    }

    pub fn close(&mut self, id: LibID) -> bool {
        self.libs.get_mut(id).and_then(|o| o.take()).is_some()
    }
    pub fn load(&mut self, filename: &str) -> Result<LibID, FFIError> {
        let lib = unsafe { Library::new(filename).map_err(FFIError::LoadLib)? };
        let id = self.libs.len();
        self.libs.push(Some(Symbols::bind(&self.decls, lib)));
        Ok(id)
    }

    pub fn rebind(&mut self) {
        self.libs.retain_mut(|i| {
            if let Some(s) = i {
                s.rebind(&self.decls);
            }
            true
        });
    }

    pub fn define(&mut self, defs: &str) -> Result<(), FFIError> {
        let decls = parse_ffis(defs)?;
        let emp = decls.no_var_func();
        self.decls.merge(decls);
        // a type written again may have a different layout, and every CIF built
        // from the old one is stale
        self.layouts.clear_cache();
        if !emp {
            self.rebind();
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            decls: CDecls::default(),
            layouts: Layouts::new(),
            libs: vec![],
        }
    }
}

/// Calls against the platform's C library, which is the one library every
/// target has. A target whose `dlopen` name differs simply runs no call tests:
/// they are about the bridge, not about finding a library.
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::ffi::CString;

    fn libc() -> Option<FFI> {
        let mut ffi = FFI::new();
        ffi.load("libc.so.6").ok()?;
        Some(ffi)
    }

    #[test]
    fn a_declared_function_is_called_through_libffi() {
        let Some(mut ffi) = libc() else {
            return;
        };
        ffi.define("int getpid(void);").unwrap();
        let Value::Int(pid) = ffi.call(None, "getpid", &[]).unwrap() else {
            panic!("getpid hands back an int");
        };
        assert!(pid > 0);
    }

    #[test]
    fn a_variadic_call_reads_the_arguments_it_was_given() {
        let Some(mut ffi) = libc() else {
            return;
        };
        ffi.define("typedef unsigned long size_t; int snprintf(char *s, size_t n, const char *f, ...);")
            .unwrap();
        let mut buffer = vec![0u8; 32];
        let format = CString::new("%d-%s").unwrap();
        let tail = CString::new("ok").unwrap();
        let out = ffi
            .call(
                None,
                "snprintf",
                &[
                    Value::Ptr(buffer.as_mut_ptr().cast()),
                    Value::ULong(buffer.len() as u64),
                    Value::Ptr(format.as_ptr() as *mut _),
                    Value::Int(42),
                    Value::Ptr(tail.as_ptr() as *mut _),
                ],
            )
            .unwrap();
        assert!(matches!(out, Value::Int(5)), "{out:?}");
        assert_eq!(std::str::from_utf8(&buffer[..5]).unwrap(), "42-ok");
    }

    #[test]
    fn a_struct_return_is_an_error_rather_than_a_panic() {
        let Some(mut ffi) = libc() else {
            return;
        };
        ffi.define("struct div_t { int quot; int rem; }; struct div_t div(int, int);")
            .unwrap();
        let err = ffi
            .call(None, "div", &[Value::Int(7), Value::Int(2)])
            .unwrap_err();
        assert!(matches!(err, FFIError::Unsupported(_)), "{err}");
    }

    #[test]
    fn a_c_variable_is_read_and_written_by_name() {
        let Some(mut ffi) = libc() else {
            return;
        };
        ffi.define("extern int optind;").unwrap();
        let before = ffi.get_var(None, "optind").expect("optind is a C global");
        assert!(matches!(before, Value::Int(_)), "{before:?}");
        ffi.set_var(None, "optind", Value::Int(3)).unwrap();
        let after = ffi.get_var(None, "optind").unwrap();
        assert!(matches!(after, Value::Int(3)), "{after:?}");
    }

    #[test]
    fn a_typedef_return_is_read_as_the_type_behind_it() {
        let Some(mut ffi) = libc() else {
            return;
        };
        ffi.define("typedef int pid_t; pid_t getpid(void);").unwrap();
        assert!(matches!(ffi.call(None, "getpid", &[]).unwrap(), Value::Int(_)));
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;

    #[test]
    fn a_declared_function_is_called_through_libffi() {
        let mut ffi = FFI::new();
        ffi.load("kernel32.dll").unwrap();
        ffi.define("unsigned long GetCurrentProcessId(void);").unwrap();
        let Value::ULong(pid) = ffi.call(None, "GetCurrentProcessId", &[]).unwrap() else {
            panic!("GetCurrentProcessId hands back a DWORD");
        };
        assert!(pid > 0);
    }
}
