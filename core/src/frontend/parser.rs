use std::collections::VecDeque;

use crate::{
    frontend::{
        ast::{BlockStmt, Expr, Stmt},
        token::{Token, TokenKind},
    },
    shared::{
        error::{DukaError, DukaParserError, Span},
        types::{DukaLexer, Spanned},
    },
};

enum ExprDesc {}

#[derive(Debug)]
pub struct Parser<Lexer: DukaLexer<Token>> {
    lexer: Lexer,
    lookahead: VecDeque<Result<Token, DukaError>>,
}

impl<Lexer: DukaLexer<Token>> Parser<Lexer> {
    pub fn new(lexer: Lexer) -> Self {
        Self {
            lexer,
            lookahead: VecDeque::new(),
        }
    }

    pub fn parse(&mut self) -> Result<BlockStmt, DukaError> {
        self.parse_function_call().map(|e| BlockStmt {
            stmts: vec![Stmt::Expr(e)],
        })
    }

    fn parse_chunk(&mut self) -> Result<BlockStmt, DukaError> {
        self.parse_block()
    }

    fn parse_block(&mut self) -> Result<BlockStmt, DukaError> {
        let mut stmts: Vec<Stmt> = vec![];
        loop {
            match self.next_token()? {
                (TokenKind::EOF, _) => break,
                (TokenKind::SemiColon, _) => continue,
                tk @ (TokenKind::Ident(_), _) | tk @ (TokenKind::LParen, _) => {
                    let prefix = self.parse_prefix(tk)?;
                }
                (TokenKind::Local, _) => stmts.push(self.parse_local()?),
                _ => return Err(self.err(DukaParserError::UnexpectedToken)),
            }
        }
        Ok(BlockStmt { stmts })
    }

    fn parse_prefix(&mut self, ahead: Token) -> Result<Stmt, DukaError> {
        todo!()
    }

    fn parse_local(&mut self) -> Result<Stmt, DukaError> {
        todo!()
    }

    fn parse_function_call(&mut self) -> Result<Expr, DukaError> {
        match self.next_token()? {
            (TokenKind::Ident(id), id_span) => {
                let args = self.parse_args()?;
                Ok(Expr::Call {
                    callee: Box::new(Expr::Ident { name: id.into() }),
                    args: vec![args],
                })
            }
            _ => Err(DukaError {
                kind: DukaParserError::UnexpectedToken.into(),
                span: self.span(),
            }),
        }
    }

    fn parse_args(&mut self) -> Result<Expr, DukaError> {
        match self.next_token()? {
            (TokenKind::String(str), str_span) => Ok(Expr::Literal { value: str.into() }),
            _ => unimplemented!(),
        }
    }

    #[inline(always)]
    fn span(&self) -> Span {
        self.lexer.span()
    }
    #[inline(always)]
    fn err(&self, kind: DukaParserError) -> DukaError {
        DukaError {
            kind: kind.into(),
            span: self.span(),
        }
    }

    fn expect(&mut self, predicate: fn(&TokenKind) -> bool) -> Result<(), DukaError> {
        match self.peek_token(0) {
            Ok(k) if predicate(k) => Ok(()),
            Err(e) => Err(e),
            _ => Err(DukaError {
                kind: DukaParserError::UnexpectedToken.into(),
                span: self.span(),
            }),
        }
    }

    #[inline(always)]
    fn move_to_peek_pos(&mut self) {
        self.lookahead.clear();
    }

    fn peek_token(&mut self, n: usize) -> Result<&TokenKind, DukaError> {
        while self.lookahead.len() <= n {
            match self.lexer.next() {
                Err(e) => return Err(e),
                Ok(t) if t.0.eof() => break,
                item => self.lookahead.push_back(item),
            }
        }
        // error won't reach there
        // use unwrap() freely
        Ok(self
            .lookahead
            .get(n)
            .unwrap()
            .as_ref()
            .map(|r| &r.0)
            .unwrap())
    }
    #[inline]
    fn next_token(&mut self) -> Result<Spanned<TokenKind>, DukaError> {
        if let Some(item) = self.lookahead.pop_front() {
            item
        } else {
            self.lexer.next()
        }
    }
}
