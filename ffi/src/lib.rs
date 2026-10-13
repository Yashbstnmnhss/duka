use libffi::low::CodePtr;
use libloading::Library;

use crate::{
    bridge::{Layouts, Symbols, Value},
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

    pub fn call(
        &mut self,
        lib: Option<LibID>,
        name: &str,
        args: &[Value],
    ) -> Result<Value, FFIError> {
        let cif = self.layouts.fn_cif(name, &self.decls)?;
        let ptr = self.get_fn(name, lib);

        if let Some((f, sig)) = cif
            && let Some(ptr) = ptr
        {
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

            let args = args
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if i < sig.params.len() {
                        let ty = &sig.params[i].1;
                        if !v.check_type(ty) {
                            return Err(FFIError::InvalidParameters(
                                name.to_owned(),
                                format!("Expected parameter with type \"{ty:?}\" at {i}"),
                            ));
                        }
                    } else if !sig.var_arg {
                        return Err(FFIError::InvalidParameters(
                            name.to_owned(),
                            "Too many arguments".to_string(),
                        ));
                    }
                    v.to_arg()
                })
                .collect::<Result<Vec<_>, _>>()?;

            let fun = CodePtr::from_ptr(ptr);
            Ok(unsafe {
                match sig.ret {
                    CType::Base(base) => match base {
                        CBaseType::Void => {
                            f.call::<()>(fun, &args);
                            Value::Void
                        }
                        CBaseType::Bool => Value::Int(f.call::<i32>(fun, &args)),
                        CBaseType::Char(sign) => match sign {
                            Sign::Unsigned => Value::UChar(f.call(fun, &args)),
                            _ => Value::Char(f.call(fun, &args)),
                        },
                        CBaseType::Short(sign) => match sign {
                            Sign::Unsigned => Value::UShort(f.call(fun, &args)),
                            _ => Value::Short(f.call(fun, &args)),
                        },
                        CBaseType::Int(sign) => match sign {
                            Sign::Unsigned => Value::UInt(f.call(fun, &args)),
                            _ => Value::Int(f.call(fun, &args)),
                        },
                        CBaseType::Long(sign) => match sign {
                            Sign::Unsigned => Value::ULong(f.call(fun, &args)),
                            _ => Value::Long(f.call(fun, &args)),
                        },
                        CBaseType::LongLong(_) => todo!(),
                        CBaseType::Float => Value::Float(f.call::<f32>(fun, &args)),
                        CBaseType::Double => Value::Double(f.call::<f64>(fun, &args)),
                        CBaseType::LongDouble => todo!(),
                    },
                    CType::Ptr(_) => Value::Ptr(f.call(fun, &args)),
                    CType::Array(..) => Value::Ptr(f.call(fun, &args)),
                    _ => unimplemented!(),
                }
            })
        } else {
            Err(FFIError::UnknownFunction(name.to_owned()))
        }
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
