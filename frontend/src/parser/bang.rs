use std::{collections::HashMap, fmt::Debug, sync::Arc};

use duka_shared::{
    ast::{ExprKind, StmtKind},
    error::DukaSpannedError,
    token::{Token, TokenKind},
    types::{BangName, Spanned},
    utils::TryDo,
};

use crate::parser::RefToken;

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

pub trait ParserAPI {
    fn span_start(&mut self) -> Result<RefToken<'_>, DukaSpannedError>;
    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError>;
    fn then_keyword(&mut self, kw: &str) -> Result<bool, DukaSpannedError>;
    fn then(&mut self, token: TokenKind) -> Result<bool, DukaSpannedError>;
    fn lookahead_token(&mut self, token: TokenKind, pos: usize) -> Result<bool, DukaSpannedError>;
    fn expect_ident(&mut self) -> TryDo<Spanned<String>, DukaSpannedError>;
    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaSpannedError>;
    fn expected(&mut self, got: &str, expected: &str) -> DukaSpannedError;
    fn must_ident(&mut self) -> Result<Spanned<String>, DukaSpannedError>;
    fn must_token(&mut self, token: TokenKind) -> Result<Token, DukaSpannedError>;
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError>;
    fn next_token(&mut self) -> Result<Token, DukaSpannedError>;
}

pub trait BangStmtHandler {
    fn handle(&self, parser: &mut dyn ParserAPI) -> Result<StmtKind, DukaSpannedError>;
}
pub trait BangExprHandler {
    fn handle(&self, parser: &mut dyn ParserAPI) -> Result<ExprKind, DukaSpannedError>;
}
