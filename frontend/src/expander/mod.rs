use std::collections::HashMap;
use std::fmt;

use crate::analyzer::{VisitMut, VisitorMut};
use crate::lexer::token::Token;
use crate::parser::Parser;
use crate::parser::ast::{BangCollected, DukaChunk, Expr, NodeKind, Stmt};

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
    fn expand_expr(&self, _node: &BangCollected) -> Result<NodeKind, BangExpanderError> {
        Ok(NodeKind::Empty)
    }
    fn expand_stmt(&self, _node: &BangCollected) -> Result<NodeKind, BangExpanderError> {
        Ok(NodeKind::Empty)
    }
}

#[derive(Clone)]
/// The names it answers to, without the expanders themselves, so a caller that
/// only has to hand a list to a parser need not share one.
pub struct BangNames(pub Vec<String>);

impl BangNames {
    /// Registers each name with a parser, so it collects `name! { .. }` whole
    /// instead of rejecting the keyword outright.
    pub fn register_with(&self, mut parser: Parser<Token>) -> Parser<Token> {
        for name in &self.0 {
            parser = parser
                .register_bang_collected_expr(name)
                .register_bang_collected_stmt(name);
        }
        parser
    }
}

pub struct BangExpanderRegistry {
    registry: HashMap<String, Box<dyn BangExpander>>,
}

impl Default for BangExpanderRegistry {
    fn default() -> Self {
        Self::new()
    }
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

    /// The names `ui` and friends answer to.
    ///
    /// The parser collects a `!` body before this stage ever sees it, so a name
    /// it holds but the parser was never told about reads as `UnknownBang`.
    pub fn names(&self) -> BangNames {
        BangNames(self.registry.keys().cloned().collect())
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
    fn visit_node(&mut self, stmt: &mut Stmt) {
        if self.result.is_err() {
            return;
        }
        let NodeKind::BangCollected(node) = &stmt.0 else {
            return;
        };
        let name = node.name.clone();
        let Some(expander) = self.expander.registry.get(&name) else {
            self.result = Err(BangExpanderError::UnknownMacro(name));
            return;
        };
        // `stmt!` and `expr!` are the same collection and expand to the same
        // node, so which of the two to call is the only thing the position was
        // ever for.
        let expanded = expander
            .expand_stmt(node)
            .or_else(|_| expander.expand_expr(node));
        match expanded {
            Ok(new_node) => stmt.0 = new_node,
            Err(e) => {
                self.result = Err(BangExpanderError::ExpansionError {
                    macro_name: name,
                    detail: e.to_string(),
                });
            }
        }
    }
}
