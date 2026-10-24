//! Documents
//!
//!
//!

use std::fmt::Display;

use crate::{
    attributes::{self, AttrReturns, Attributes},
    constants::ctype,
    dtype::{FunctionType, Type, TypeVar},
    value::ConstValue,
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

// Moved to attributes mod
// doc! {
//     @(catt::DECLARE): "@declare", "Available for: function, variable, object \n\nSays what exists rather than what to build. The signature is written down and nothing is generated, so this is a note the type checker and every reader of the code get to see; the thing itself is expected to be there already, put there by the host, or by another module. A declared function takes no body and a declared value takes no value, because there would be nothing to do with either. A declaration with no annotation says nothing rather than saying `nil`, and one with no return annotation returns nothing\n\n`@declare export` is how a module hands a promise on: the importer gets the signature and calls it as a function, while the module itself writes no `export` for it and the name is `nil` there. `RequireType(\"m\").f` asks a declaration for the shape it promised, the way it asks a type alias for a type -- and reads through as far as you like, each dot checked on its own", "@declare function strlen(s: string): int\n\n@declare global HOSTNAME: string\n@declare @const MAX_PATH: int\n\n@declare object Point\n    x: int\n    y: int\nend\n\n-- in `m.duka`\n@declare export function strlen(s: string): int\n\n-- elsewhere\nlocal m = require(\"m\")\nlocal n: int = m.strlen(\"s\")\nlocal g: RequireType(\"m\").strlen = some_function";
//     @(catt::INLINE): "@inline", "Available for: function \n\nHints the generator to make this function **inline** if possible";
//     @(catt::CONST): "@const", "Available for: variable \n\nMarks a variable to be a constant. This variable will be immutable";
//     @(catt::CLOSE): "@close", "Available for: variable \n\nMarks a variable to be closed automatically";
//     @(catt::DATA): "@data(frozen: bool = false)", "Available for: object \n\nAutomatically generate `init()`, `__eq`, `__tostring` based on properties defined";
//     @(catt::RETURNS): "@returns(...)", "Available for: function \n\nSays what the return slots stand for. `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned";
//     @(catt::KEYWORDISH): "@keywordish", "Available for: function \n\nThe declaration reads as a **keyword** rather than as a name"
// }

doc! {
    for provide: "provide(...)", "## Type Context\n\n Provide constraints for **parameters** in current type function";
    for ensure: "ensure", "## Type Context\n\n Mark a function with type guard\n```lua\nfunction not_nil<T>(t: T): ensure NonNil(T)\n```";
    for type: "type", "## Type Context\n\n Used to\n- Define type alias\n- Define type function\n- Turn an expression into its type";
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
    for return: "return", "Return value(s) from function\nIt may be written anywhere in a block; whatever follows it is unreachable";
    for do: "do", "Do block, see `for` `while`\nYou can make an IIFE by `do...end`\nA `break` written inside leaves the block, and `continue` skips it for the loop around it";
    for end: "end", "Mark the end of a block";
    for then: "then", "Then block, see `if` `match`";
    for break: "break", "Exit the nearest `do` block or `for`/`while` loop. `break 1, 2` leaves with values, which only a `do` block has somewhere to put";
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
    type Nil: "Nil", "A null value";
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
impl KeywordDoc {
    pub const fn get_doc(&self) -> &Doc {
        match self {
            KeywordDoc::Keyword { doc, .. } => doc,
            KeywordDoc::Type { doc, .. } => doc,
            KeywordDoc::Attribute { doc, .. } => doc,
        }
    }
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

pub type MetaInfoFlag = (&'static str, &'static [&'static str]);

pub type BuiltinInfo = (MetaScope, MetaInfo);

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
        self.get_type_with(&|_, _| None)
    }

    /// The type this is, with the names it holds resolved by `resolve`.
    ///
    /// A builtin may name a type the language declares, and the scope that
    /// says what the name is is not the scope the metadata is read in, so the
    /// reading is handed in.
    pub fn get_type_with(&self, resolve: &dyn Fn(&str, &[Type]) -> Option<Type>) -> Type {
        let declared = match &self.info {
            MetaItemInfo::TypeFunction { .. } => Type::Any,
            MetaItemInfo::Static { inner, .. } => inner.get_type_with(resolve),
            MetaItemInfo::Module { .. } => Type::Table(None, None),
            MetaItemInfo::UserData { .. } => Type::Table(None, None),
            MetaItemInfo::Constant { ty, .. } => ty.clone().into_type_with(resolve),
            MetaItemInfo::Function {
                returns,
                params,
                type_params,
                ..
            } => {
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
                        let ty: Type = p.ty.clone().into_type_with(resolve);
                        if p.optional { ty.nilable() } else { ty }
                    })
                    .collect();
                Type::Function(Some(FunctionType {
                    var_arg: var_arg || open,
                    return_var_arg: returns.var_arg,
                    params,
                    returns: returns
                        .tys
                        .iter()
                        .cloned()
                        .map(|i| i.into_type_with(resolve))
                        .collect(),
                    type_vars: Self::type_vars_of(type_params, resolve),
                }))
            }
        };
        self.apply_returns(declared)
    }

    /// A return protocol says what the return slots are, so it wins over
    /// whatever the parameter list claims to return.
    fn apply_returns(&self, ty: Type) -> Type {
        let (returns, return_var_arg) = match attributes::get_returns(self.flags) {
            Some(AttrReturns::Result) => ([Type::Bool].into(), true),
            Some(AttrReturns::Exit) => ([].into(), false),
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

    /// The type parameters a builtin declares, as the signature carries them.
    ///
    /// A builtin writes its variables where a file writes them, and the file's
    /// own declaration is not there to be read, so the metadata is the only
    /// place they exist.
    #[inline]
    fn type_vars_of(
        vars: &'static [VarMeta],
        resolve: &dyn Fn(&str, &[Type]) -> Option<Type>,
    ) -> Box<[TypeVar]> {
        vars.iter()
            .map(|v| TypeVar {
                name: v.name.into(),
                bound: v.bound.clone().map(|b| Box::new(b.into_type_with(resolve))),
                default: v
                    .default
                    .clone()
                    .map(|d| Box::new(d.into_type_with(resolve))),
                literal: v.literal,
                pack: v.pack,
            })
            .collect()
    }

    #[inline]
    pub fn attributes(&self) -> Attributes {
        Attributes::from_meta_flags(self.flags)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetaItemInfo {
    TypeFunction {
        param_count: usize,
        // TODO
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
        /// `T` in `as.<T>(v: any): T`. A variable the caller fills in, and the
        /// only way a builtin signature can answer in terms of what it was given.
        type_params: &'static [VarMeta],
        /// The `where` list a builtin writes, the clauses a call has to
        /// discharge before it counts.
        where_clauses: &'static [WhereMeta],
        /// The constraints a builtin's values have to satisfy, the postcondition
        /// a call leaves behind.
        constraints: &'static [ConstraintMeta],
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

/// A `where` clause a builtin writes.
///
/// The clauses a file writes are read by the parser and hold expressions; the
/// ones a builtin writes are held as data, and the frontend turns them into
/// the same thing the parser would have produced.
#[derive(Debug, Clone, PartialEq)]
pub enum WhereMeta {
    /// `bound(T: int)` -- what a type parameter has to accept
    Bound { var: &'static str, ty: DocType },
    /// `bind(V: int)` -- a type-level binding, the `type V = T` of a `where`
    Bind { var: &'static str, ty: DocType },
    /// `concept(Name, args...)` -- a concept to discharge
    ///
    /// A concept is a type-level expression read for its truth, and a builtin
    /// names one and gives it its arguments rather than writing the
    /// expression out.
    Concept {
        name: &'static str,
        args: &'static [DocType],
    },
}

/// A constraint a builtin's values have to satisfy.
///
/// The same atoms the flow works with, written as data: a builtin says what
/// has to be true of its arguments, and the frontend turns each one into the
/// constraint the flow applies.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintMeta {
    /// `is(arg, T)` -- the argument has to be of that type
    Is { arg: usize, ty: DocType },
    /// `is_not(arg, T)` -- the argument must not be of that type
    IsNot { arg: usize, ty: DocType },
    /// `sub(arg, T)` -- the argument has to be a subtype of that type
    Sub { arg: usize, ty: DocType },
    /// `has(arg, "field")` -- the argument has to have that member
    Has { arg: usize, field: &'static str },
    /// `instance(arg, T)` -- the argument has to be an instance of that type
    Instance { arg: usize, ty: DocType },
    /// `never(arg)` -- the argument must not be there at all
    Never { arg: usize },
}

/// A type parameter a builtin declares, the `T` of `as.<T>(v: any): T`
#[derive(Debug, Clone, PartialEq)]
pub struct VarMeta {
    pub name: &'static str,
    /// `T: int`, what the variable has to accept once the caller has filled it
    /// in. Nothing is checked when there is no bound.
    pub bound: Option<DocType>,
    /// `T = int`, what the variable is when nothing else determines it
    pub default: Option<DocType>,
    /// `literal T`: the variable is filled in from what the argument *is*, not
    /// from the type the argument widens to. `as_const(1)` answers `1` this way
    /// where an ordinary variable would answer `int`.
    pub literal: bool,
    /// `...Ts`: the variable takes the rest of the type arguments as a list
    pub pack: bool,
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
    /// This is special for compile-time
    StringLiteral(&'static str),
    /// A type parameter the signature declares, written as the bare name. A
    /// builtin that answers in terms of what it was given names it here.
    Param(&'static str),
    /// A userdata type, named. An empty name is a userdata of any kind.
    UserData(&'static str),
    /// A type the language declares, named: the types the type prelude defines.
    ///
    /// The name is the whole of it here, because the scope that says what the
    /// name is is not the scope that reads it.
    ///
    /// The arguments are what the name is applied to, the way a type function
    /// is applied in a type position: `type(Optional, int)` is `int?`.
    Named(&'static str, &'static [DocType]),
    Bytes,
    Union(&'static [DocType]), // SPECIAL, THIS IS FOR CONSTANT!
    Array(&'static DocType),
    Table(Option<&'static DocType>, Option<&'static DocType>),
    Function(&'static [DocType], &'static [DocType], &'static [VarMeta]),
    VarArg,
}
impl From<DocType> for Type {
    fn from(value: DocType) -> Self {
        // a name that says nothing here is `any`: whoever knows the scope is
        // the one that can say what the name is
        value.into_type_with(&|_, _| None)
    }
}
impl DocType {
    /// The type this is, with the names it holds resolved by `resolve`.
    ///
    /// A named type is the one thing here that cannot be read without a
    /// scope, so the reading is handed in rather than guessed at.
    pub fn into_type_with(self, resolve: &dyn Fn(&str, &[Type]) -> Option<Type>) -> Type {
        match self {
            DocType::StringLiteral(sl) => Type::Literal(ConstValue::String(
                sl.to_owned().into_bytes().into_boxed_slice(),
            )),
            DocType::Base(t) => t,
            DocType::Param(name) => Type::Param((*name).into()),
            DocType::UserData(name) => Type::UserData {
                name: (*name).into(),
            },
            DocType::Named(name, args) => {
                let args: Vec<Type> = args
                    .iter()
                    .cloned()
                    .map(|a| a.into_type_with(resolve))
                    .collect();
                resolve(name, &args).unwrap_or(Type::Any)
            }
            DocType::Bytes => Type::String,
            DocType::VarArg => Type::Any,
            DocType::Union(ts) => Type::Union(
                ts.iter()
                    .map(|i| i.clone().into_type_with(resolve))
                    .collect(),
            ),
            DocType::Array(t) => Type::Array(Some(Box::new((*t).clone().into_type_with(resolve)))),
            DocType::Table(k, v) => Type::Table(
                k.map(|k| Box::new((*k).clone().into_type_with(resolve))),
                v.map(|v| Box::new((*v).clone().into_type_with(resolve))),
            ),
            DocType::Function(params, returns, type_vars) => {
                let split = |list: &[DocType]| -> (Vec<Type>, bool) {
                    let mut items = list.iter().peekable();
                    let mut out = vec![];
                    while let Some(t) = items.next() {
                        if matches!(t, DocType::VarArg) && items.peek().is_none() {
                            return (out, true);
                        }
                        out.push(t.clone().into_type_with(resolve));
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
                    // the variables a signature declares travel with it, so a
                    // call site can solve for them without knowing where the
                    // signature was written
                    type_vars: type_vars
                        .iter()
                        .map(|v| TypeVar {
                            name: v.name.into(),
                            bound: v.bound.clone().map(|b| Box::new(b.into_type_with(resolve))),
                            default: v
                                .default
                                .clone()
                                .map(|d| Box::new(d.into_type_with(resolve))),
                            literal: v.literal,
                            pack: v.pack,
                        })
                        .collect(),
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
                DocType::StringLiteral(sl) => format!("{sl:?}"),
                DocType::Base(t) => t.to_string(),
                DocType::Param(name) => (*name).to_owned(),
                DocType::Named(name, _) => (*name).to_owned(),
                DocType::UserData(name) => {
                    if name.is_empty() {
                        "userdata".to_owned()
                    } else {
                        (*name).to_owned()
                    }
                }
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
                DocType::Function(params, returns, type_vars) => {
                    let params = params
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    // the variables are written the way a signature writes them
                    let vars = if type_vars.is_empty() {
                        String::new()
                    } else {
                        let vars = type_vars
                            .iter()
                            .map(|v| {
                                if v.pack {
                                    format!("...{}", v.name)
                                } else {
                                    v.name.to_owned()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("<{vars}>")
                    };
                    if returns.is_empty() {
                        format!("fn{vars}({params})")
                    } else {
                        let returns = returns
                            .iter()
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("fn{vars}({params}) -> {returns}")
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
            &[],
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
            DocType::Function(&[DocType::VarArg], &[], &[]).to_string(),
            "fn(...)"
        );
    }

    /// A function type carries the variables it declares, so a call site can
    /// solve for them without knowing where the type was written.
    #[test]
    fn a_function_type_carries_the_variables_it_declares() {
        static VARS: &[VarMeta] = &[VarMeta {
            name: "T",
            bound: Some(DocType::Base(Type::Int)),
            default: None,
            literal: false,
            pack: false,
        }];
        let doc = DocType::Function(&[DocType::Param("T")], &[DocType::Param("T")], VARS);
        let Type::Function(Some(ft)) = Type::from(doc) else {
            panic!("expected a function type");
        };
        assert_eq!(ft.type_vars.len(), 1);
        assert_eq!(ft.type_vars[0].name.as_ref(), "T");
        assert_eq!(ft.type_vars[0].bound.as_deref(), Some(&Type::Int));
        assert_eq!(DocType::Function(&[], &[], VARS).to_string(), "fn<T>()");
    }

    /// A named type is read by whoever knows the scope: the name is all the
    /// metadata holds, and the reading is handed in.
    #[test]
    fn a_named_type_is_read_by_the_scope_that_knows_it() {
        let doc = DocType::Named("CoStatus", &[]);
        // a scope that does not know the name says nothing about it
        assert_eq!(Type::from(doc.clone()), Type::Any);
        let resolved = doc.into_type_with(&|name, args| {
            assert_eq!(name, "CoStatus");
            assert!(args.is_empty());
            Some(Type::String)
        });
        assert_eq!(resolved, Type::String);
    }

    /// A named type applied to arguments is applied the way a type position
    /// applies it.
    #[test]
    fn a_named_type_carries_what_it_is_applied_to() {
        let doc = DocType::Named("Optional", &[DocType::Base(Type::Int)]);
        let resolved = doc.into_type_with(&|name, args| {
            assert_eq!(name, "Optional");
            assert_eq!(args, &[Type::Int]);
            Some(Type::Union(vec![Type::Int, Type::Nil].into()))
        });
        assert_eq!(resolved, Type::Union(vec![Type::Int, Type::Nil].into()));
    }

    /// The variables a builtin declares travel with its signature, so what it
    /// says about them -- what they have to accept, what they are when nothing
    /// else says, and whether one of them takes the rest -- reaches the
    /// checker rather than staying in the documentation.
    #[test]
    fn a_type_parameter_carries_its_bound_its_default_and_its_shape() {
        static TYPE_PARAMS: &[VarMeta] = &[VarMeta {
            name: "T",
            bound: Some(DocType::Base(Type::Int)),
            default: Some(DocType::Base(Type::String)),
            literal: false,
            pack: true,
        }];
        static INFO: MetaInfo = MetaInfo {
            name: "f",
            doc: "",
            example: None,
            info: MetaItemInfo::Function {
                returns: ReturnMeta {
                    text: "",
                    tys: &[],
                    var_arg: false,
                },
                params: &[],
                type_params: TYPE_PARAMS,
                where_clauses: &[],
                constraints: &[],
            },
            flags: &[],
        };
        let Type::Function(Some(ft)) = INFO.get_type() else {
            panic!("expected a function type");
        };
        assert_eq!(ft.type_vars.len(), 1);
        let var = &ft.type_vars[0];
        assert_eq!(var.name.as_ref(), "T");
        assert_eq!(var.bound.as_deref(), Some(&Type::Int));
        assert_eq!(var.default.as_deref(), Some(&Type::String));
        assert!(var.pack);
    }
}
