//! Documents
//!
//!
//!

use std::fmt::Display;

use crate::{
    constants::ctype,
    dtype::{FunctionType, Type},
};

macro_rules! doc {
    ($title: literal, $content: literal) => {
        Doc {
            title: $title,
            content: $content,
            example: None,
        }
    };
    ($title: literal, $content: literal, $example: literal) => {
        Doc {
            title: $title,
            content: $content,
            example: Some($example),
        }
    };
    ($(for $for: ident: $title: literal, $content: literal $(, $example: literal)?);*) => {
        pub const KEYWORD_DOCS: &'static [KeywordDoc] = &[
            $(KeywordDoc::Keyword {
                doc: doc!($title, $content $(, $example)?),
                keyword: stringify!($for)
            }),*
        ];
    };
    ($(type $type: ident: $title: literal, $content: literal $(, $example: literal)?);*) => {
        pub const TYPE_DOCS: &'static [KeywordDoc] = &[
            $(KeywordDoc::Type {
                doc: doc!($title, $content $(, $example)?),
                ty: Type::$type
            }),*
        ];
    };
}

doc! {
    for if: "If", "If";
    for for: "for", "For";
    for function: "function", "Define a function"
}
doc! {
    type Int: "Integer", "Alias: int";
    type Float: "Float", "Alias: number";
    type Bool: "Bool", "Alias: boolean";
    type String: "String", "Alias: str";
    type Nil: "Nil", "";
    type Table: "Table", "`{}`";
    type Any: "Any", "Accpets all"
}

#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub title: &'static str,
    pub content: &'static str,
    pub example: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeywordDoc {
    Keyword { doc: Doc, keyword: &'static str },
    Type { doc: Doc, ty: Type },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetaInfo {
    pub module: &'static str,
    pub name: &'static str,
    pub doc: &'static str,
    pub example: Option<&'static str>,
    pub info: MetaItemInfo,
}

impl MetaInfo {
    pub fn get_type(&self) -> Type {
        match &self.info {
            MetaItemInfo::Constant { ty, .. } => ty.clone(),
            MetaItemInfo::Function { returns, params } => {
                let mut var_arg = false;
                let params = params
                    .iter()
                    .map(|p| {
                        if p.var_arg {
                            var_arg = true
                        }
                        let ty = p.ty.get_type();
                        if p.optional { ty.nilable() } else { ty }
                    })
                    .collect();
                Type::Function(Some(FunctionType {
                    var_arg,
                    return_var_arg: returns.var_arg,
                    params,
                    returns: returns.tys.into(),
                }))
            }
        }
    }
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
    pub var_arg: bool,
    pub tys: &'static [Type],
}

impl Display for ReturnMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}",
            if self.var_arg {
                "...".to_owned()
            } else {
                self.tys.len().to_string()
            },
            self.text
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamMeta {
    pub name: &'static str,
    pub ty: ParamType,
    pub optional: bool,
    pub default: Option<&'static str>,
    pub var_arg: bool,
    pub doc: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamType {
    Base(Type),
    PreserveNumber,
    Bytes,
    Union(&'static [ParamType]),
}
impl ParamType {
    pub fn get_type(&self) -> Type {
        match self {
            Self::Base(t) => t.clone(),
            Self::PreserveNumber => Type::Float,
            Self::Bytes => Type::String,
            Self::Union(ts) => Type::Union(ts.iter().map(|i| i.get_type()).collect()),
        }
    }
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
                ParamType::Union(items) => items
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
            }
        )
    }
}
