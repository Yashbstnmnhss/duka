use std::collections::HashMap;

use crate::{
    constants::catt,
    docs::{Doc, KeywordDoc, MetaInfoFlag},
    dtype::Type,
    utils::OrError,
    value::{AttrValue, ConstValue},
};

#[derive(Debug, Clone)]
pub struct Attribute {
    id: usize,
    pub values: HashMap<String, AttrValue>,
}

impl Attribute {
    pub fn merge_values(&mut self, other: Attribute) -> Result<(), AttrError> {
        if other.id != self.id {
            return Ok(());
        }
        for (k, v) in other.values {
            self.set(&k, v)?;
        }
        Ok(())
    }

    #[inline]
    pub fn new(name: &str) -> Option<Self> {
        ATTRIBUTES.iter().position(|i| i.0 == name).map(|id| Self {
            id,
            values: HashMap::new(),
        })
    }
    #[inline]
    pub fn get_def(&self) -> &AttrDef {
        assert!(self.id < ATTRIBUTES.len());
        &ATTRIBUTES[self.id].1
    }
    pub fn get(&self, key: &str) -> Option<&AttrValue> {
        self.values.get(key)
    }
    pub fn set(&mut self, key: &str, val: AttrValue) -> Result<(), AttrError> {
        self.get_def()
            .params
            .is_some_and(|p| p.is_empty())
            .then_error(|| AttrError::NoParamNeed)?;
        let val = if let Some(i) = self.values.get_mut(key) {
            std::mem::take(i).merge(val)
        } else {
            val
        };
        self.get_def()
            .check_param(key, &val)
            .map_err(|desc| AttrError::Param(key.to_owned(), desc))?;
        self.values.insert(key.to_owned(), val);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrReturns {
    Result,
    Exit,
}

#[inline]
pub fn get_returns(flags: &'static [MetaInfoFlag]) -> Option<AttrReturns> {
    flags.iter().find_map(|a| {
        (a.0 == catt::RETURNS)
            .then(|| match *a.1.first()? {
                catt::RETURNS_EXIT => Some(AttrReturns::Exit),
                catt::RETURNS_RESULT => Some(AttrReturns::Result),
                _ => None,
            })
            .flatten()
    })
}

#[derive(Debug, Clone, Default)]
pub struct Attributes(
    Vec<Attribute>,
    bool, /* declare */
    bool, /* keywordish */
);

#[derive(Debug, Clone)]
pub enum AttrError {
    BadUsage,
    Repeat,
    Undefined,
    NoParamNeed,
    Param(String, String),
}

impl Attributes {
    #[inline(always)]
    pub fn has_declare(&self) -> bool {
        self.1
    }
    #[inline(always)]
    pub fn has_keywordish(&self) -> bool {
        self.2
    }

    /// The `doc` attribute's text, with several runs joined in the order they
    /// were written.
    pub fn get_doc(&self) -> Option<Box<str>> {
        let id = ATTRIBUTES.iter().position(|i| i.0 == catt::DOC)?;
        let attr = self.0.iter().find(|i| i.id == id)?;
        let strings: Vec<&[u8]> = match attr.get("value")? {
            AttrValue::Single(ConstValue::String(s)) => vec![s.as_ref()],
            AttrValue::Many(vs) => vs
                .iter()
                .filter_map(|v| match v {
                    ConstValue::String(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect(),
            _ => return None,
        };
        if strings.is_empty() {
            return None;
        }
        let mut doc = Vec::new();
        for (i, s) in strings.iter().enumerate() {
            if i > 0 {
                doc.push(b'\n');
            }
            doc.extend_from_slice(s);
        }
        String::from_utf8(doc).ok().map(Box::from)
    }

    pub fn from_meta_flags(info: &'static [MetaInfoFlag]) -> Self {
        Self(
            info.iter()
                .filter_map(|i| {
                    let mut attr = Attribute::new(i.0)?;
                    for k in i.1 {
                        attr.set(k, AttrValue::Single(ConstValue::Bool(true)))
                            .ok()?; // ignore
                    }
                    Some(attr)
                })
                .collect(),
            info.iter().any(|i| i.0 == catt::DECLARE),
            info.iter().any(|i| i.0 == catt::KEYWORDISH),
        )
    }

    pub const fn new() -> Self {
        Self(vec![], false, false)
    }

    pub fn get(&self, id: usize) -> Result<Attribute, AttrError> {
        let values = self.0.iter().filter(|i| i.id == id).fold(
            HashMap::<String, AttrValue>::new(),
            |mut a, b| {
                for (k, v) in &b.values {
                    if let Some(i) = a.get_mut(k) {
                        *i = std::mem::take(i).merge(v.clone());
                    } else {
                        a.insert(k.to_owned(), v.clone());
                    }
                }
                a
            },
        );
        Ok(Attribute { id, values })
    }
    pub fn append(&mut self, attr: Attribute) -> Result<(), AttrError> {
        if attr.id >= ATTRIBUTES.len() {
            return Err(AttrError::Undefined);
        }
        let name = ATTRIBUTES[attr.id].0;
        if let Some(who) = self.0.iter_mut().find(|i| i.id == attr.id)
            && let Some((_, def, _)) = ATTRIBUTES.get(attr.id)
        {
            if def.repeatable {
                who.merge_values(attr)?;
            } else {
                return Err(AttrError::Repeat);
            }
        } else {
            self.0.push(attr);
        }
        // the two flags are the attributes a reader asks about by name rather
        // than by id, so they are kept beside the list as well
        if name == catt::DECLARE {
            self.1 = true;
        } else if name == catt::KEYWORDISH {
            self.2 = true;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum AttrStage {
    Parsing,
    Analysis,
    Adapting,
    Compilation,
}

#[derive(Debug, Clone)]
pub struct AttrParam(
    pub &'static str,
    pub Option<Type>,
    pub Option<AttrValue>,
    pub Option<fn(&AttrValue) -> Option<&'static str>>,
);

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum AttrUsage {
    Function = AttrDef::FUNCTION,
    Object = AttrDef::OBJECT,
    Variable = AttrDef::VARIABLES,
    Type = AttrDef::TYPEDECL,
}

#[derive(Debug, Clone)]
pub struct AttrDef {
    usage: u8,
    repeatable: bool,
    params: Option<&'static [AttrParam]>,
    stage: AttrStage,
}

impl AttrDef {
    pub const FUNCTION: u8 = 1 << 0;
    pub const OBJECT: u8 = 1 << 1;
    pub const VARIABLES: u8 = 1 << 2;
    pub const TYPEDECL: u8 = 1 << 3;

    pub const fn all() -> Self {
        Self {
            usage: u8::MAX - 1,
            repeatable: true,
            params: None,
            stage: AttrStage::Adapting,
        }
    }
    #[inline]
    pub const fn new() -> Self {
        Self {
            repeatable: true,
            usage: 0,
            params: None,
            stage: AttrStage::Adapting,
        }
    }

    pub fn check_param(&self, key: &str, val: &AttrValue) -> Result<(), String> {
        let Some(params) = self.params else {
            return Ok(()); // ignore
        };
        if let Some(p) = params.iter().find(|i| i.0 == key) {
            if let Some(ty) = &p.1 {
                match val {
                    AttrValue::Many(_) => matches!(ty, Type::Array(..))
                        .or_else_error(|| format!("Expected {ty}, got array"))?,
                    AttrValue::Single(s) => ty
                        .accepts(&s.type_of())
                        .or_else_error(|| format!("Expected {ty}, got {}", s.type_of()))?,
                }
            }

            if let Some(f) = &p.3
                && let Some(msg) = f(val)
            {
                return Err(msg.to_owned());
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    #[inline]
    pub const fn params(mut self, params: &'static [AttrParam]) -> Self {
        self.params = Some(params);
        self
    }

    #[inline]
    pub const fn set_stage(mut self, stage: AttrStage) -> Self {
        self.stage = stage;
        self
    }

    #[inline]
    pub const fn set_repeatable(mut self, val: bool) -> Self {
        self.repeatable = val;
        self
    }
    #[inline]
    pub const fn repeatable(&self) -> bool {
        self.repeatable
    }
    #[inline]
    pub const fn check(&self, at: AttrUsage) -> bool {
        (self.usage & (at as u8)) != 0
    }
    #[inline]
    pub const fn used_at(mut self, at: AttrUsage) -> Self {
        self.usage |= at as u8;
        self
    }
}

macro_rules! attr {
    [@($name: expr)$(($($p: tt)*))?
        for $($ats: ident),*
        $(in $stage: ident)?
        $(repeat = $r: expr)?
        ;
        name = $dname: literal,
        doc = $doc: literal
    ] => {
        ($name, AttrDef::new()
            $(.used_at(AttrUsage::$ats))*
            $(.set_stage(AttrStage::$stage))?
            $(.set_repeatable($r))?
            $(.params(attr!(#params $($p)*)))?,
            &KeywordDoc::Attribute {
                doc: Doc {
                    title: concat!(attr!(#doc $dname), attr!(#params_doc $($($p)*)?)),
                    content: $doc,
                    example: None
                },
                attr: $name
            }
        )
    };
    (#doc $name: literal) => {
        concat!("@", $name)
    };
    (#if ($($_: tt)+) ($($me: tt)*) ($($__: tt)*)) => {
        $($me)*
    };
    (#if () ($($_: tt)*) ($($me: tt)*)) => {
        $($me)*
    };
    (#params_doc) => {""};
    (#params_doc $($k: ident $(: $ty: ident)? $(with($c: expr))? $(= $de: expr)?),*) => {
        concat!("(", $(stringify!($k), attr!(#if ($($de)?) ("?") (""))),+ , ")")
    };
    (#option) => {None};
    (#option $($t: tt)+) => {Some($($t)+)};
    (#params) => {
        &[]
    };
    (#params $($k: ident $(: $ty: ident)? $(with($c: expr))? $(= $de: expr)?),+) => {
        &[
            $(AttrParam(
                stringify!($k),
                attr!(#option $(Type::$ty)?),
                attr!(#option $($de)?),
                attr!(#option $($c)?)
            )),+
        ]
    }
}

pub const ATTRIBUTES: &[(&str, AttrDef, &KeywordDoc)] = &[
    attr!(@(catt::DOC)(value: String) for Function, Object, Variable, Type in Parsing;
        name = "doc", doc = "Documentation attached to a declaration. `---` lines are sugar for this attribute"),
    attr!(@(catt::KEYWORDISH)() for Function in Analysis; 
        name = "keywordish", doc = "The declaration reads as a **keyword** rather than as a name"),
    attr!(@(catt::CONST)() for Variable in Compilation; name = "const", doc = "Marks a variable to be a constant. This variable will be immutable"),
    attr!(@(catt::CLOSE)() for Variable in Compilation; name = "close", doc = "Marks a variable to be closed automatically"),
    attr!(@(catt::INLINE)(always = AttrValue::Single(ConstValue::Bool(false))) for Function in Adapting;
        name = "inline", doc = "Hints the generator to make this function **inline** if possible"
    ),
    attr!(
        @(catt::DATA)(frozen: Bool = AttrValue::Single(ConstValue::Bool(false)))
        for Object in Adapting
        repeat = false;
        name = "data", doc = "Automatically generate `init()`, `__eq`, `__tostring` based on properties defined"
    ),
    attr!(
        @(catt::DECLARE)()
        for Function, Object, Variable, Type in Parsing
        repeat = false;
        name = "declare", doc =
r#"Says what exists rather than what to build. 
The signature is written down and nothing is generated, so this is a note the type checker and every reader of the code get to see; the thing itself is expected to be there already, put there by the host, or by another module. 
A declared function takes no body and a declared value takes no value, because there would be nothing to do with either. 
A declaration with no annotation says nothing rather than saying `nil`, and one with no return annotation returns nothing

- `@declare export` is how a module hands a promise on: the importer gets the signature and calls it as a function, while the module itself writes no `export` for it and the name is `nil` there. 
- `RequireType(\"m\").f` asks a declaration for the shape it promised, the way it asks a type alias for a type -- and reads through as far as you like, each dot checked on its own
"#
    ),
    attr!(
        @(catt::RETURNS)
        for Function in Analysis
        repeat = false;
        name = "returns", doc =
r#"Says what the return slots stand for. 
- `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. 
- `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned.
"#
    ),
];

pub fn attr_doc(name: &str) -> Option<&'static Doc> {
    ATTRIBUTES
        .iter()
        .find_map(|i| (i.0 == name).then_some(i.2.get_doc()))
}
