//! Documents
//!
//!
//!

use std::fmt::Display;

use crate::{
    constants::{catt, ctype},
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
                ty: stringify!($type)
            }),*
        ];
    };
    ($(@($attr: expr): $title: literal, $content: literal $(, $example: literal)?);*) => {
        pub const ATTR_DOCS: &'static [KeywordDoc] = &[
            $(KeywordDoc::Attribute {
                doc: doc!($title, $content $(, $example)?),
                attr: $attr
            }),*
        ];
    };
}

doc! {
    @(catt::INLINE): "@inline", "Available for: function \nHints the generator to make this function **inline** if possible";
    @(catt::CONST): "@const", "Available for: variable \nMarks a variable to be a constant. This variable will be immutable";
    @(catt::CLOSE): "@close", "JUST A PLACEHOLDER";
    @(catt::DATA): "@data(frozen: bool)", "Available for: object \nAutomatically generate `init()`, `__eq`, `__tostring` based on properties defined"
}
doc! {
    for type: "type", "# Type Context\n See docs for details";
    for if: "if", "Evaluate a block if a condition holds";
    for else: "else", "What expression to evaluate when an `if` condition evaluates to `false`";
    for elseif: "elseif", "What expression to evaluate when an `if` or `elseif` condition evaluates to `false` and current condition evaluates to `true`";
    for for: "for", "Iteration with `in`(generic) or numerical";
    for while: "while", "Loop while a condition is upheld";
    for function: "function", "Define a function";
    for fn: "lambda function", "A lambda expression";
    for object: "object", "Define a object";
    for in: "in", "Used in `for` loop and `linq!`";
    for match: "match", "Control flow based on pattern matching";
    for return: "return", "Return value(s) from function\nThis statement must be the **last** statement in block";
    for do: "do", "Do block, see `for` `while`\nYou can make an IIFE by `do...end`";
    for end: "end", "Mark the end of a block";
    for then: "then", "Then block, see `if` `match`";
    for break: "break", "Exit early from a loop";
    for continue: "continue", "Skip to the next iteration of a loop";
    for goto: "goto", "Jump to visible label";
    for export: "export", "Mark a function, variable or object to be exported, see `require()`";
    for extends: "extends", "Declare its parent object";
    for local: "local", "Make a function, variable or object local";
    for global: "global", "Make a function, variable or object global";

    for and: "and", "";
    for or: "or", "";
    for xor: "xor", "";
    for not: "not", "";

    for true: "true", "A value of type `bool` representing logical `true`";
    for false: "false", "A value of type `bool` representing logical `false`";
    for nil: "nil", "A value represents empty, `null`"
}
doc! {
    type Never: "Never", "Accepts nothing";
    type Int: "Integer", "Alias: int";
    type Float: "Float", "Alias: number";
    type Bool: "Bool", "Alias: boolean";
    type String: "String", "Alias: str";
    type Nil: "Nil", "";
    type Table: "Table", "`{}`";
    type Array: "Array", "Alias: list\n`[]`";
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
    Type { doc: Doc, ty: &'static str },
    Attribute { doc: Doc, attr: &'static str },
}

pub fn keyword_doc(name: &str) -> Option<&'static Doc> {
    KEYWORD_DOCS.iter().find_map(|d| match d {
        KeywordDoc::Keyword { doc, keyword } if *keyword == name => Some(doc),
        _ => None,
    })
}

pub fn type_doc(ty: &Type) -> Option<&'static Doc> {
    let name = match ty {
        Type::Nil => "Nil",
        Type::Bool => "Bool",
        Type::Int => "Int",
        Type::Float => "Float",
        Type::String => "String",
        Type::Array(_) => "Array",
        Type::Table(..) => "Table",
        Type::Any => "Any",
        Type::Never => "Never",
        _ => return None,
    };
    TYPE_DOCS.iter().find_map(|d| match d {
        KeywordDoc::Type { doc, ty: n } if *n == name => Some(doc),
        _ => None,
    })
}

pub fn attr_doc(name: &str) -> Option<&'static Doc> {
    ATTR_DOCS.iter().find_map(|d| match d {
        KeywordDoc::Attribute { doc, attr } if *attr == name => Some(doc),
        _ => None,
    })
}

pub type MetaInfoFlag = (&'static str, &'static [&'static str]);

#[derive(Debug, Clone, PartialEq)]
pub struct MetaInfo {
    pub name: &'static str,
    pub doc: &'static str,
    pub example: Option<&'static str>,
    pub info: MetaItemInfo,
    pub flags: &'static [MetaInfoFlag],
}

impl MetaInfo {
    pub fn get_flag(&self, key: &str) -> Option<&'static [&'static str]> {
        self.flags.iter().find(|i| i.0 == key).map(|i| i.1)
    }
    pub fn has_flag_value(&self, key: &str, val: &str) -> bool {
        self.get_flag(key).is_some_and(|i| i.contains(&val))
    }
    pub fn get_type(&self) -> Type {
        match &self.info {
            MetaItemInfo::TypeFunction { .. } => Type::Any,
            MetaItemInfo::Static { inner, .. } => inner.get_type(),
            MetaItemInfo::Module { .. } => Type::Table(None, None),
            MetaItemInfo::UserData { .. } => Type::Table(None, None),
            MetaItemInfo::Constant { ty, .. } => ty.clone(),
            MetaItemInfo::Function { returns, params } => {
                let mut var_arg = false;
                let params = params
                    .iter()
                    .map(|p| {
                        if p.var_arg {
                            var_arg = true
                        }
                        let ty: Type = p.ty.clone().into();
                        if p.optional { ty.nilable() } else { ty }
                    })
                    .collect();
                Type::Function(Some(FunctionType {
                    var_arg,
                    return_var_arg: returns.var_arg,
                    params,
                    returns: returns.tys.iter().cloned().map(|i| i.into()).collect(),
                }))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetaItemInfo {
    TypeFunction {
        param_count: usize,
    },
    Static {
        inner: &'static MetaInfo,
    },
    Module {
        inner: &'static [MetaInfo],
    },
    Function {
        returns: ReturnMeta,
        params: &'static [ParamMeta],
    },
    Constant {
        ty: Type,
        val: &'static str,
    },
    UserData {
        ty_name: &'static str,
        methods: &'static [MetaInfo],
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnMeta {
    pub text: &'static str,
    pub var_arg: bool,
    pub tys: &'static [DocType],
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
    pub ty: DocType,
    pub optional: bool,
    pub default: Option<&'static str>,
    pub var_arg: bool,
    pub doc: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DocType {
    Base(Type),
    PreserveNumber,
    Bytes,
    Union(&'static [DocType]), // SPECIAL, THIS IS FOR CONSTANT!
}
impl From<DocType> for Type {
    fn from(value: DocType) -> Self {
        match value {
            DocType::Base(t) => t,
            DocType::PreserveNumber => Type::Float,
            DocType::Bytes => Type::String,
            DocType::Union(ts) => Type::Union(ts.iter().map(|i| i.clone().into()).collect()),
        }
    }
}
impl Display for DocType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                DocType::Base(t) => t.to_string(),
                DocType::PreserveNumber => ctype::FLO.to_owned(),
                DocType::Bytes => ctype::STR.to_owned(),
                DocType::Union(items) => items
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
            }
        )
    }
}
