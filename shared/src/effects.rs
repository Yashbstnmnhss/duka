use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Effect {
    Mutable = 1,
    External = 1 << 1,
    Throw = 1 << 2,
    Diverge = 1 << 3,
    Nondet = 1 << 4,
    Control = 1 << 5,
}

impl Effect {
    pub const ALL: u8 = Self::Mutable as u8
        | Self::External as u8
        | Self::Throw as u8
        | Self::Diverge as u8
        | Self::Nondet as u8
        | Self::Control as u8;

    pub fn name(self) -> &'static str {
        use Effect::*;
        match self {
            Mutable => "mutable",
            External => "external",
            Throw => "throw",
            Diverge => "diverge",
            Nondet => "nondet",
            Control => "control",
        }
    }

    pub fn parse(name: &str) -> Option<Effect> {
        Some(match name {
            "mutable" => Effect::Mutable,
            "external" => Effect::External,
            "throw" => Effect::Throw,
            "diverge" => Effect::Diverge,
            "nondet" => Effect::Nondet,
            "control" => Effect::Control,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct EffectSet {
    pub yes: u8,
    pub no: u8,
}

impl EffectSet {
    pub const fn pure() -> Self {
        Self {
            yes: 0,
            no: Effect::ALL,
        }
    }

    pub const fn is_pure(&self) -> bool {
        self.no == Effect::ALL
    }

    pub const fn yes_has(&self, e: Effect) -> bool {
        self.yes & e as u8 != 0
    }

    pub const fn no_has(&self, e: Effect) -> bool {
        self.no & e as u8 != 0
    }

    pub const fn knows(&self, e: Effect) -> bool {
        (self.yes | self.no) & e as u8 != 0
    }

    pub fn with(&mut self, e: Effect) -> Result<(), Effect> {
        if self.no & e as u8 != 0 {
            return Err(e);
        }
        self.yes |= e as u8;
        Ok(())
    }

    pub fn deny(&mut self, e: Effect) -> Result<(), Effect> {
        if self.yes & e as u8 != 0 {
            return Err(e);
        }
        self.no |= e as u8;
        Ok(())
    }

    pub fn merge(&mut self, other: &Self) {
        self.yes |= other.yes;
        self.no |= other.no;
        self.no &= !self.yes;
    }

    pub fn accepts(&self, other: &Self) -> bool {
        other.yes & !self.yes == 0
            && self.no & (Effect::ALL & !(other.yes | other.no)) == 0
    }
}
