use std::collections::HashMap;

use crate::{
    constants::catt,
    docs::MetaInfoFlag,
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
        if self.0.iter().any(|i| i.id == attr.id)
            && let Some((_, def)) = ATTRIBUTES.get(attr.id)
            && def.repeatable
        {
            self.0.push(attr);
            return Ok(());
        }
        Err(AttrError::Repeat)
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
    ] => {
        ($name, AttrDef::new()
            $(.used_at(AttrUsage::$ats))*
            $(.set_stage(AttrStage::$stage))?
            $(.set_repeatable($r))?
            $(.params(attr!(#params $($p)*)))?
        )
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

pub const ATTRIBUTES: &'static [(&'static str, AttrDef)] = &[
    attr!(@(catt::KEYWORDISH)() for Function in Analysis),
    attr!(@(catt::CONST)() for Variable in Compilation),
    attr!(@(catt::CLOSE)() for Variable in Compilation),
    attr!(@(catt::INLINE)(always = AttrValue::Single(ConstValue::Bool(false))) for Function in Adapting),
    attr!(
        @(catt::DATA)(frozen: Bool = AttrValue::Single(ConstValue::Bool(false)))
        for Object in Adapting
        repeat = false
    ),
    attr!(
        @(catt::DECLARE)()
        for Function, Object, Variable in Parsing
        repeat = false
    ),
    attr!(
        @(catt::RETURNS)
        for Function in Analysis
        repeat = false
    ),
];
