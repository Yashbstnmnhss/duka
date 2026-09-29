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

/// Whether the members of a module are registered as globals or are reached
/// through the module name. Only the runtime knows which, so it says so rather
/// than leaving the compiler to guess from the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaScope {
    Global,
    Module,
}

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
            MetaItemInfo::Constant { ty, .. } => ty.clone().into(),
            MetaItemInfo::Function { returns, params } => {
                // the rest parameter says how many arguments may follow, it is
                // not a slot of its own, so it never enters the list
                let var_arg = params.iter().any(|p| p.var_arg);
                // a parameter with a default may be left out, and the language
                // has no syntax for one, so a function type cannot say which of
                // its parameters those are. Reading it as open is the loosest
                // reading that still never rejects a call the runtime accepts;
                // the parameters themselves are listed by the documentation.
                let open = params.iter().any(|p| p.optional || p.default.is_some());
                let fixed = params
                    .iter()
                    .filter(|p| !p.var_arg)
                    .position(|p| p.optional || p.default.is_some())
                    .unwrap_or(usize::MAX);
                let params = params
                    .iter()
                    .filter(|p| !p.var_arg)
                    .take(fixed)
                    .map(|p| {
                        let ty: Type = p.ty.clone().into();
                        if p.optional { ty.nilable() } else { ty }
                    })
                    .collect();
                Type::Function(Some(FunctionType {
                    var_arg: var_arg || open,
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
        ty: DocType,
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
    Array(&'static DocType),
    Table(Option<&'static DocType>, Option<&'static DocType>),
    Function(&'static [DocType], &'static [DocType]),
    VarArg,
}
impl From<DocType> for Type {
    fn from(value: DocType) -> Self {
        match value {
            DocType::Base(t) => t,
            DocType::PreserveNumber => Type::Float,
            DocType::Bytes => Type::String,
            DocType::VarArg => Type::Any,
            DocType::Union(ts) => Type::Union(ts.iter().map(|i| i.clone().into()).collect()),
            DocType::Array(t) => Type::Array(Some(Box::new((*t).clone().into()))),
            DocType::Table(k, v) => Type::Table(
                k.map(|k| Box::new((*k).clone().into())),
                v.map(|v| Box::new((*v).clone().into())),
            ),
            DocType::Function(params, returns) => {
                let split = |list: &[DocType]| -> (Vec<Type>, bool) {
                    let mut items = list.iter().peekable();
                    let mut out = vec![];
                    while let Some(t) = items.next() {
                        if matches!(t, DocType::VarArg) && items.peek().is_none() {
                            return (out, true);
                        }
                        out.push(t.clone().into());
                    }
                    (out, false)
                };
                let (params, var_arg) = split(params);
                let (returns, return_var_arg) = split(returns);
                Type::Function(Some(FunctionType {
                    params: params.into(),
                    var_arg,
                    returns: returns.into(),
                    return_var_arg,
                }))
            }
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
                DocType::Array(t) => format!("array<{t}>"),
                DocType::VarArg => "...".to_owned(),
                DocType::Table(k, v) => match (k, v) {
                    (None, None) => "table".to_owned(),
                    _ => {
                        let k = k.map(|t| t.to_string()).unwrap_or_else(|| "any".to_owned());
                        let v = v.map(|t| t.to_string()).unwrap_or_else(|| "any".to_owned());
                        format!("table<{k}, {v}>")
                    }
                },
                DocType::Function(params, returns) => {
                    let params = params
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if returns.is_empty() {
                        format!("fn({params})")
                    } else {
                        let returns = returns
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("fn({params}) -> {returns}")
                    }
                }
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_function_carries_vararg_flags() {
        let doc = DocType::Function(
            &[DocType::Base(Type::Int), DocType::VarArg],
            &[DocType::VarArg],
        );
        let Type::Function(Some(ft)) = Type::from(doc) else {
            panic!("expected a function type");
        };
        assert!(ft.var_arg);
        assert!(ft.return_var_arg);
        assert_eq!(ft.params.as_ref(), &[Type::Int]);
        assert!(ft.returns.is_empty());
    }

    #[test]
    fn doc_type_vararg_displays_as_ellipsis() {
        assert_eq!(DocType::VarArg.to_string(), "...");
        assert_eq!(Type::from(DocType::VarArg), Type::Any);
        assert_eq!(
            DocType::Function(&[DocType::VarArg], &[]).to_string(),
            "fn(...)"
        );
    }
}
