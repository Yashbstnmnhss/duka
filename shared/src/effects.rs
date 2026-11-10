use std::fmt::Display;

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

pub const EFFECT_ATOMS: &[Effect] = &[
    Effect::Control,
    Effect::Diverge,
    Effect::External,
    Effect::Mutable,
    Effect::Nondet,
    Effect::Throw,
];

impl Effect {
    pub const ALL: u8 = Self::Mutable as u8
        | Self::External as u8
        | Self::Throw as u8
        | Self::Diverge as u8
        | Self::Nondet as u8
        | Self::Control as u8;

    pub const fn name(self) -> &'static str {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(Default)]
pub struct EffectSet {
    pub yes: u8,
    pub no: u8,
}


impl EffectSet {
    #[inline(always)]
    pub const fn pure() -> Self {
        Self {
            yes: 0,
            no: Effect::ALL,
        }
    }
    #[inline]
    pub const fn is_pure(&self) -> bool {
        self.no == Effect::ALL
    }
    #[inline]
    pub const fn yes_has(&self, e: Effect) -> bool {
        self.yes & e as u8 != 0
    }
    #[inline]
    pub const fn no_has(&self, e: Effect) -> bool {
        self.no & e as u8 != 0
    }
    #[inline]
    pub const fn knows(&self, e: Effect) -> bool {
        (self.yes | self.no) & e as u8 != 0
    }

    /// The effects a policy leaves no room for: everything it did not enable,
    /// plus everything it denied outright. An effect that is both enabled and
    /// denied is denied.
    ///
    /// A project config uses this to say what the whole file may do: `yes` is
    /// what is enabled, and an effect it never mentions was not enabled.
    #[inline]
    pub const fn denied_mask(&self) -> u8 {
        (Effect::ALL & !self.yes) | self.no
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
        other.yes & !self.yes == 0 && self.no & (Effect::ALL & !(other.yes | other.no)) == 0
    }
}

impl Display for EffectSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.yes == 0 && self.no == 0 {
            return Ok(());
        }
        if self.is_pure() {
            return write!(f, "@effects(pure)");
        }
        let held = EFFECT_ATOMS
            .iter()
            .filter(|e| self.yes_has(**e))
            .map(|e| e.name())
            .collect::<Vec<_>>()
            .join(", ");
        let denied = EFFECT_ATOMS
            .iter()
            .filter(|e| self.no_has(**e))
            .map(|e| e.name())
            .collect::<Vec<_>>()
            .join(", ");
        let body = if held.is_empty() {
            format!("deny({denied})")
        } else if denied.is_empty() {
            held
        } else {
            format!("{held}, deny({denied})")
        };
        write!(f, "@effects({body})")
    }
}
