use std::collections::HashMap;

use crate::parser::CDefParserError;

#[derive(Debug, thiserror::Error)]
pub enum FFIError {
    #[error("Unknown function \"{0}\" was called")]
    UnknownFunction(String),
    #[error("Invalid parameters for calling \"{0}\": {1}")]
    InvalidParameters(String, String),
    #[error("[Library] {0}")]
    LoadLib(libloading::Error),
    #[error("{0}")]
    Unsupported(String),
    #[error("[Parser] {0}")]
    Parser(CDefParserError),
    #[error("\"{0}\" has incomplete type")]
    IncompleteType(String),
    #[error("\"{0}\" is unknown")]
    UnknownType(String),
    #[error("\"{0}\" is duplicated")]
    DuplicatedDef(String),
    #[error("{0}")]
    InvalidType(String),
    #[error("Cannot pass {0} as {1}")]
    Mismatch(String, String),
    #[error("\"{0}\" was not found in any loaded library")]
    UnknownSymbol(String),
}
impl From<CDefParserError> for FFIError {
    fn from(value: CDefParserError) -> Self {
        FFIError::Parser(value)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Sign {
    #[default]
    Default,
    Signed,
    Unsigned,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CBaseType {
    Void,
    Bool,
    Char(Sign),
    Short(Sign),
    Int(Sign),
    Long(Sign),
    LongLong(Sign),
    Float,
    Double,
    LongDouble,
}

impl CBaseType {
    /// Size in bytes on the target this crate assumes, `void` is zero sized
    ///
    /// ```
    /// use duka_ffi::cdef::{CBaseType, Sign};
    ///
    /// assert_eq!(CBaseType::Void.size_of(), 0);
    /// assert_eq!(CBaseType::Bool.size_of(), 1);
    /// assert_eq!(CBaseType::Char(Sign::Signed).size_of(), 1);
    /// assert_eq!(CBaseType::Short(Sign::Unsigned).size_of(), 2);
    /// assert_eq!(CBaseType::Int(Sign::Signed).size_of(), 4);
    /// assert_eq!(CBaseType::Double.size_of(), 8);
    ///
    /// // alignment follows the size, so a double is 8 byte aligned too
    /// assert_eq!(CBaseType::Double.align_of(), 8);
    /// ```
    #[inline]
    pub const fn size_of(&self) -> usize {
        match self {
            CBaseType::Void => 0,
            CBaseType::Bool => 1,
            CBaseType::Char(_) => 1,
            CBaseType::Short(_) => 2,
            CBaseType::Int(_) => 4,
            CBaseType::Long(_) => 8,
            CBaseType::LongLong(_) => 8,
            CBaseType::Float => 4,
            CBaseType::Double => 8,
            CBaseType::LongDouble => 16,
        }
    }
    #[inline]
    pub const fn align_of(&self) -> usize {
        self.size_of()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CStruct(pub Option<String>, pub Vec<(String, CType)>);

#[derive(Debug, Clone, PartialEq)]
pub struct CEnum(pub Option<String>, pub Vec<(String, Option<isize>)>);

#[derive(Debug, Clone, PartialEq)]
pub struct CUnion(pub Option<String>, pub Vec<(String, CType)>);

#[derive(Debug, Clone, PartialEq)]
pub enum CType {
    Base(CBaseType),
    Ptr(Box<CType>),
    Array(Box<CType>, Option<usize>),
    Struct(CStruct),
    Enum(CEnum),
    Union(CUnion),
    Function(Box<CFnSig>),

    TagRef(usize),
    TypeRef(usize),
}
impl CType {
    pub const fn is_ref(&self) -> bool {
        matches!(self, Self::TypeRef(..) | Self::TagRef(..))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CTag {
    Struct(CStruct),
    Enum(CEnum),
    Union(CUnion),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CDecls {
    pub(crate) variables: HashMap<String, CType>,
    pub(crate) functions: HashMap<String, CFnSig>,
    pub(crate) typedefs: Vec<(String, CType)>,
    pub(crate) tags: Vec<CTag>,
    pub(crate) tag_mapper: HashMap<String, usize>,
}

impl CDecls {
    #[inline]
    pub(crate) fn no_var_func(&self) -> bool {
        self.variables.is_empty() && self.functions.is_empty()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
            && self.functions.is_empty()
            && self.tags.is_empty()
            && self.typedefs.is_empty()
    }

    pub fn merge(&mut self, mut other: CDecls) {
        let CDecls {
            variables,
            functions,
            typedefs,
            tags,
            tag_mapper,
        } = self;

        variables.extend(other.variables);
        functions.extend(other.functions);

        for (n, t) in other.typedefs {
            if let Some(ty) = typedefs
                .iter_mut()
                .find_map(|i| (i.0 == n).then_some(&mut i.1))
            {
                *ty = t;
            } else {
                typedefs.push((n, t));
            }
        }

        for (n, i) in other.tag_mapper {
            let ty = std::mem::replace(
                other.tags.get_mut(i).expect("Checked"),
                CTag::Struct(CStruct(None, vec![])),
            );
            if let Some(id) = tag_mapper.get(&n) {
                tags.insert(*id, ty);
            } else {
                let id = tags.len();
                tag_mapper.insert(n, id);
                tags.insert(id, ty);
            }
        }
    }

    pub fn declare_tag(&mut self, name: String, tag: CTag) -> Result<usize, FFIError> {
        if self.tag_mapper.contains_key(&name) {
            return Err(FFIError::DuplicatedDef(name));
        }
        let id = self.tags.len();
        self.tag_mapper.insert(name, id);
        self.tags.push(tag);
        Ok(id)
    }
    pub fn declare_typedef(&mut self, name: String, ty: CType) -> usize {
        self.typedefs.push((name, ty));
        self.typedefs.len() - 1
    }
    pub fn declare_variable(&mut self, name: String, ty: CType) -> Result<(), FFIError> {
        if self.variables.contains_key(&name) {
            return Err(FFIError::DuplicatedDef(name));
        }
        self.variables.insert(name, ty);
        Ok(())
    }
    pub fn declare_function(&mut self, name: String, fs: CFnSig) -> Result<(), FFIError> {
        if self.functions.contains_key(&name) {
            return Err(FFIError::DuplicatedDef(name));
        }
        self.functions.insert(name, fs);
        Ok(())
    }

    #[inline]
    pub fn get_tag(&self, name: &str) -> Option<&CTag> {
        self.get_tag_by_id(*self.tag_mapper.get(name)?)
    }
    #[inline]
    pub fn get_tag_by_id(&self, id: usize) -> Option<&CTag> {
        self.tags.get(id)
    }
    #[inline]
    pub fn get_type(&self, id: usize) -> Option<&CType> {
        self.typedefs.get(id).map(|(_, b)| b)
    }

    /// The declaration written under `name`, if the file declared it.
    #[inline]
    pub fn function(&self, name: &str) -> Option<&CFnSig> {
        self.functions.get(name)
    }

    /// The variable declared under `name`, if the file declared it.
    #[inline]
    pub fn variable(&self, name: &str) -> Option<&CType> {
        self.variables.get(name)
    }

    /// A type with the typedefs and tags it names written out.
    ///
    /// A declaration refers to a typedef or a tag by id, and what a call has to
    /// know is the shape behind it: a parameter that is `Handler` is a pointer
    /// like any other once the name has been followed.
    pub fn resolve(&self, ty: &CType) -> Result<CType, FFIError> {
        self.resolve_with(ty, 0)
    }

    fn resolve_with(&self, ty: &CType, depth: usize) -> Result<CType, FFIError> {
        // a typedef may name another one, and a header that loops already failed
        // to be written; the bound keeps this from recursing forever anyway
        if depth > 64 {
            return Err(FFIError::IncompleteType(format!("{ty:?}")));
        }
        match ty {
            CType::TypeRef(id) => {
                let named = self
                    .get_type(*id)
                    .ok_or_else(|| FFIError::UnknownType("unknown typedef".to_owned()))?;
                self.resolve_with(named, depth + 1)
            }
            CType::TagRef(id) => match self.get_tag_by_id(*id) {
                Some(CTag::Enum(e)) => Ok(CType::Enum(e.clone())),
                Some(CTag::Struct(s)) => Ok(CType::Struct(s.clone())),
                Some(CTag::Union(u)) => Ok(CType::Union(u.clone())),
                None => Err(FFIError::UnknownType("unknown tag".to_owned())),
            },
            other => Ok(other.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CFnSig {
    pub ret: CType,
    pub params: Vec<(Option<String>, CType)>,
    pub var_arg: bool,
}
