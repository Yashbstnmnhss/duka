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
    @(catt::DECLARE): "@declare", "Declare types of function, constant, object";
    // @(catt::HIGHTLIGHT): "@highlight(lang: string)", "Hints editor to highlight this string";
    @(catt::INLINE): "@inline", "Available for: function \n\nHints the generator to make this function **inline** if possible";
    @(catt::CONST): "@const", "Available for: variable \n\nMarks a variable to be a constant. This variable will be immutable";
    @(catt::CLOSE): "@close", "Available for: variable \n\nMarks a variable to be closed automatically";
    @(catt::DATA): "@data(frozen: bool = false)", "Available for: object \n\nAutomatically generate `init()`, `__eq`, `__tostring` based on properties defined";
    @(catt::RETURNS): "@returns(...)", "Available for: function \n\nSays what the return slots stand for. `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned";
    @(catt::KEYWORDISH): "@keywordish", "Available for: function \n\nThe declaration reads as a **keyword** rather than as a name"
}
doc! {
    for type: "type", "# Type Context\n\n See docs for details";
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
    for goto: "goto", "Jump to a visible label\n\n```lua\n::A::\n--...\ngoto A\n```";
    for export: "export", "Mark a function, variable or object to be exported, see `require()`";
    for extends: "extends", "Declare its parent object";
    for local: "local", "Make a function, variable or object local";
    for global: "global", "Make a function, variable or object global";

    for and: "and", "Logical AND operator & AND for patterns in `match`";
    for or: "or", "Logical OR operator & OR for patterns in `match`";
    for xor: "xor", "Logical XOR operator & XOR for patterns in `match`";
    for not: "not", "Logical NOT operator & NOT for single pattern in `match`";

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

/// What the return slots of a declaration stand for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Returns {
    /// `@returns(result)`: the first slot says whether the call succeeded and
    /// the rest are the values it produced.
    Result,
    /// `@returns(exit)`: nothing comes back, and nothing after the call means
    /// anything either, so there is no question of what it returned.
    Exit,
}

/// What an attribute does to the thing it is written on. An attribute the
/// language gives no meaning to is documentation rather than semantics, so an
/// unknown one is left alone instead of being guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribute {
    /// `@returns(...)`: marks the result as something special
    Returns(Returns),
    /// `@keywordish`: the declaration reads as a keyword rather than as a name.
    Keywordish,
}

impl Attribute {
    /// Reads one `@name(value)` pair. The value is what tells two meanings of
    /// the same attribute apart, so an attribute that takes one and was not
    /// given it means nothing.
    pub fn of(name: &str, value: Option<&str>) -> Option<Attribute> {
        Some(match name {
            catt::KEYWORDISH => Attribute::Keywordish,
            catt::RETURNS => Attribute::Returns(match value? {
                v if v == catt::RESULT => Returns::Result,
                v if v == catt::EXIT => Returns::Exit,
                _ => return None,
            }),
            _ => return None,
        })
    }

    /// Reads one off the metadata, where an attribute is a flag rather than a
    /// pair. A flag with several values is read as the first one that means
    /// something, which is the same as a runtime declaring it twice.
    pub fn of_flag(name: &str, values: &[&str]) -> Option<Attribute> {
        Attribute::of(
            name,
            values
                .iter()
                .copied()
                .find(|v| *v == catt::RESULT || *v == catt::EXIT || name == catt::KEYWORDISH),
        )
    }

    /// What the attribute does to the return slots, if anything.
    pub fn returns(self) -> Option<Returns> {
        match self {
            Attribute::Returns(returns) => Some(returns),
            Attribute::Keywordish => None,
        }
    }

    /// Whether the declaration should read as a keyword.
    pub fn is_keywordish(self) -> bool {
        matches!(self, Attribute::Keywordish)
    }
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
        let declared = match &self.info {
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
        };
        self.apply_returns(declared)
    }

    /// A return protocol says what the return slots are, so it wins over
    /// whatever the parameter list claims to return.
    fn apply_returns(&self, ty: Type) -> Type {
        let Some(attribute) = self
            .flags
            .iter()
            .find_map(|(key, values)| Attribute::of_flag(key, values))
        else {
            return ty;
        };
        let (returns, return_var_arg) = match attribute.returns() {
            Some(Returns::Result) => ([Type::Bool].into(), true),
            Some(Returns::Exit) => ([].into(), false),
            None => return ty,
        };
        match ty {
            Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
                returns,
                return_var_arg,
                ..ft
            })),
            other => other,
        }
    }

    /// The attribute this declaration carries, if the language gives one a
    /// meaning. A runtime declares it the same way it declares its documentation,
    /// so a builtin and a function written by hand are read the same way.
    pub fn attribute(&self) -> Option<Attribute> {
        self.flags
            .iter()
            .find_map(|(key, values)| Attribute::of_flag(key, values))
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
