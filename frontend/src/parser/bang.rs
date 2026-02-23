//! Bang `!` Handler
//!

use std::{collections::HashMap, fmt::Debug, sync::Arc};

use duka_shared::{
    errors::DukaSpannedError,
    types::{BangName, Spanned},
    utils::TryDo,
};

use crate::{
    lexer::token::{Token, TokenKind},
    parser::ast::{ExprKind, StmtKind},
};

#[derive(Default)]
pub struct BangHandlers {
    expr_handlers: HashMap<BangName, Arc<dyn BangExprHandler>>,
    stmt_handlers: HashMap<BangName, Arc<dyn BangStmtHandler>>,
}
impl BangHandlers {
    pub fn register_expr(&mut self, keyword: impl Into<String>, handler: Arc<dyn BangExprHandler>) {
        self.expr_handlers.insert(keyword.into(), handler);
    }
    pub fn register_stmt(&mut self, keyword: impl Into<String>, handler: Arc<dyn BangStmtHandler>) {
        self.stmt_handlers.insert(keyword.into(), handler);
    }

    pub fn get_expr(&self, keyword: &str) -> Option<Arc<dyn BangExprHandler>> {
        self.expr_handlers.get(keyword).cloned()
    }
    pub fn get_stmt(&self, keyword: &str) -> Option<Arc<dyn BangStmtHandler>> {
        self.stmt_handlers.get(keyword).cloned()
    }
}
impl Debug for BangHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BangHandlers").finish()
    }
}

/// Public interface of parser for bang handlers
pub trait ParserAPI {
    /// Must be the given keyword, or throw error
    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError>;
    /// If next is the given keyword, return true and consume it
    fn then_keyword(&mut self, kw: &str) -> Result<bool, DukaSpannedError>;
    /// If next is the given token, return true and consume it
    fn then(&mut self, token: TokenKind) -> Result<bool, DukaSpannedError>;
    /// Peek nth token ahead and match it, not consuming it
    fn lookahead_token(&mut self, token: TokenKind, pos: usize) -> Result<bool, DukaSpannedError>;
    /// Try to match an identifier token
    fn expect_ident(&mut self) -> TryDo<Spanned<String>, DukaSpannedError>;
    /// Try to match the given token
    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaSpannedError>;
    /// Create an `Unexpected` error with got and expected message
    fn expected(&mut self, got: &str, expected: &str) -> DukaSpannedError;
    /// Must be an identifier, or throw error
    fn must_ident(&mut self) -> Result<Spanned<String>, DukaSpannedError>;
    /// Must be the given token, or throw error
    fn must_token(&mut self, token: TokenKind) -> Result<Token, DukaSpannedError>;
    /// Peek nth token ahead, return its reference
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError>;
    /// Get next token (consume), returning `TokenKind::terminator()` means the end of input
    fn next_token(&mut self) -> Result<Token, DukaSpannedError>;
}

pub trait BangStmtHandler {
    fn handle(&self, parser: &mut dyn ParserAPI) -> Result<StmtKind, DukaSpannedError>;
}
pub trait BangExprHandler {
    fn handle(&self, parser: &mut dyn ParserAPI) -> Result<ExprKind, DukaSpannedError>;
}

pub struct StickWoodHandler;
impl BangExprHandler for StickWoodHandler {
    fn handle(&self, _: &mut dyn ParserAPI) -> Result<ExprKind, DukaSpannedError> {
        Ok(ExprKind::Empty)
    }
}
