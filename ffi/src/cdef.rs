use std::collections::HashMap;

use crate::parser::CDefParserError;

#[derive(Debug, Clone, thiserror::Error)]
pub enum CDefError {
    #[error("[Parser] {0}")]
    Parser(CDefParserError),
    #[error("\"{0}\" has incomplete type")]
    IncompleteType(String),
    #[error("\"{0}\" is unknown")]
    UnknownType(String),
    #[error("\"{0}\" is duplicated")]
    DuplicatedDef(String),
}
impl From<CDefParserError> for CDefError {
    fn from(value: CDefParserError) -> Self {
        CDefError::Parser(value)
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
    variables: HashMap<String, CType>,
    functions: HashMap<String, CFnSig>,
    pub(crate) typedefs: Vec<(String, CType)>,
    pub(crate) tags: Vec<CTag>,
    pub(crate) tag_mapper: HashMap<String, usize>,
}

impl CDecls {
    pub fn declare_tag(&mut self, name: String, tag: CTag) -> Result<usize, CDefError> {
        if self.tag_mapper.contains_key(&name) {
            return Err(CDefError::DuplicatedDef(name));
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
    pub fn declare_variable(&mut self, name: String, ty: CType) -> Result<(), CDefError> {
        if self.variables.contains_key(&name) {
            return Err(CDefError::DuplicatedDef(name));
        }
        self.variables.insert(name, ty);
        Ok(())
    }
    pub fn declare_function(&mut self, name: String, fs: CFnSig) -> Result<(), CDefError> {
        if self.functions.contains_key(&name) {
            return Err(CDefError::DuplicatedDef(name));
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct CFnSig {
    pub ret: CType,
    pub params: Vec<(Option<String>, CType)>,
    pub var_arg: bool,
}
