use std::fmt::Display;

use crate::{constants::ctype, dtype::Type};

#[derive(Debug, Clone, PartialEq)]
pub struct MetaInfo {
    pub module: &'static str,
    pub name: &'static str,
    pub doc: &'static str,
    pub example: Option<&'static str>,
    pub info: MetaItemInfo,
}
#[derive(Debug, Clone, PartialEq)]
pub enum MetaItemInfo {
    Function {
        returns: ReturnMeta,
        params: &'static [ParamMeta],
    },
    Constant {
        ty: Type,
        val: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnMeta {
    pub text: &'static str,
    pub arity: ReturnArity,
}

impl Display for ReturnMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.arity, self.text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnArity {
    Zero,
    One,
    Many(usize),
    Dynamic,
}
impl Display for ReturnArity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ReturnArity::Zero => "None".to_owned(),
                ReturnArity::One => "1".to_owned(),
                ReturnArity::Many(u) => u.to_string(),
                ReturnArity::Dynamic => "...".to_owned(),
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamMeta {
    pub name: &'static str,
    pub ty: ParamType,
    pub optional: bool,
    pub default: Option<&'static str>,
    pub vararg: bool,
    pub doc: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    Base(Type),
    PreserveNumber,
    Bytes,
}
impl Display for ParamType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ParamType::Base(t) => t.to_string(),
                ParamType::PreserveNumber => ctype::FLO.to_owned(),
                ParamType::Bytes => ctype::STR.to_owned(),
            }
        )
    }
}
