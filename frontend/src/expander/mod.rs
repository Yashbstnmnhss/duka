use std::collections::HashMap;
use std::fmt;

use crate::analyzer::{VisitMut, VisitorMut};
use crate::parser::ast::{BangMacroNode, DukaChunk, Expr, ExprKind};

#[cfg(feature = "ui")]
pub mod ui;

#[derive(Debug)]
pub enum BangExpanderError {
    UnknownMacro(String),
    ParseError(String),
    ExpansionError { macro_name: String, detail: String },
}

impl fmt::Display for BangExpanderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BangExpanderError::UnknownMacro(name) => write!(f, "unknown bang macro: {name}"),
            BangExpanderError::ParseError(msg) => write!(f, "parse error: {msg}"),
            BangExpanderError::ExpansionError { macro_name, detail } => {
                write!(f, "expansion error for '{macro_name}': {detail}")
            }
        }
    }
}

impl std::error::Error for BangExpanderError {}

pub trait BangExpander: Send + Sync {
    fn expand(&self, node: &BangMacroNode) -> Result<ExprKind, BangExpanderError>;
}

pub struct BangExpanderRegistry {
    registry: HashMap<String, Box<dyn BangExpander>>,
}

impl BangExpanderRegistry {
    pub fn new() -> Self {
        let mut expander = Self {
            registry: HashMap::new(),
        };
        #[cfg(feature = "ui")]
        expander.register("ui", Box::new(ui::UIExpanderAdapter));
        expander
    }

    pub fn register(&mut self, name: impl Into<String>, expander: Box<dyn BangExpander>) {
        self.registry.insert(name.into(), expander);
    }

    pub fn expand_chunk(&self, chunk: &mut DukaChunk) -> Result<(), BangExpanderError> {
        let mut visitor = BangExpansionVisitor {
            expander: self,
            result: Ok(()),
        };
        chunk.visit_mut(&mut visitor);
        visitor.result
    }
}

struct BangExpansionVisitor<'a> {
    expander: &'a BangExpanderRegistry,
    result: Result<(), BangExpanderError>,
}

impl<'a> VisitorMut for BangExpansionVisitor<'a> {
    fn visit_expr(&mut self, expr: &mut Expr) {
        if self.result.is_err() {
            return;
        }
        if let ExprKind::BangMacro(node) = &expr.0 {
            let name = node.name.clone();
            if let Some(expander) = self.expander.registry.get(&name) {
                match expander.expand(node) {
                    Ok(new_expr) => {
                        expr.0 = new_expr;
                    }
                    Err(e) => {
                        self.result = Err(BangExpanderError::ExpansionError {
                            macro_name: name,
                            detail: e.to_string(),
                        });
                    }
                }
            } else {
                self.result = Err(BangExpanderError::UnknownMacro(name));
            }
        }
    }
}
