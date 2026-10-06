use libffi::low::CodePtr;
use libloading::Library;

use crate::{
    bridge::{Layouts, Symbols, Value},
    cdef::{CDecls, FFIError},
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
        _args: &[Value],
    ) -> Result<Value, FFIError> {
        let cif = self.layouts.fn_cif(name, &self.decls)?;
        let ptr = self.get_fn(name, lib);

        if let Some(f) = cif
            && let Some(ptr) = ptr
        {
            unsafe {
                f.call_return_into(CodePtr::from_ptr(ptr), &[], todo!());
            }
        }

        Ok(todo!())
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
