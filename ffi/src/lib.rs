use libloading::Library;

use crate::{
    bridge::{Layouts, Symbols},
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
    pub fn close(&mut self, id: LibID) -> bool {
        self.libs.get_mut(id).and_then(|o| o.take()).is_some()
    }
    pub fn load(&mut self, filename: &str) -> Result<LibID, FFIError> {
        let lib = unsafe { Library::new(filename).map_err(FFIError::LoadLib)? };
        let id = self.libs.len();
        self.libs.push(Some(Symbols::bind(&self.decls, lib)));
        Ok(id)
    }
    pub fn define(&mut self, defs: &str) -> Result<(), FFIError> {
        let decls = parse_ffis(defs)?;
        self.decls.merge(decls);
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
