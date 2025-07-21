use std::collections::VecDeque;

use crate::{
    frontend::{
        ast::{Block, Expr, ExprKind, Path, Stmt, StmtKind},
        token::{Token, TokenKind},
    },
    shared::{
        error::{DukaError, DukaLexerError, DukaParserError, Span},
        types::{DukaLexer, Spanned},
        utils::TryDo,
        value::Value,
    },
};

/// ## Marker []
/// optional
macro_rules! opt {
    [$e: expr] => {
        $e
    };
}
/// ## Marker {}
/// none or many
macro_rules! many {
    {$e: expr} => {
        $e
    };
    // {$m: ident, $c: expr, separated by $s: expr, allow tail} => {
    //     let res = vec![];
    //     if let Some(t) =
    // };
}
/// ## Marker ()
/// one of them
macro_rules! oneof {
    ($e: expr) => {
        $e
    };
}
/// ## Marker ()
/// must be exactly
macro_rules! must {
    ($e: expr, $self: ident, $msg: expr) => {
        $e?.ok_or($self.expecting($msg))
    };
}

type RefToken<'a> = Spanned<&'a TokenKind>;

#[derive(Debug)]
pub struct Parser<Lexer: DukaLexer<Token>> {
    lexer: Lexer,
    lookahead: VecDeque<Result<Token, DukaError>>,

    current_span: Span,
}

impl<Lexer: DukaLexer<Token>> Parser<Lexer> {
    pub fn new(lexer: Lexer) -> Self {
        Self {
            lexer,
            lookahead: VecDeque::new(),

            current_span: Span::EMPTY,
        }
    }

    pub fn parse(&mut self) -> Result<Block, DukaError> {
        self.chunk()
    }

    fn chunk(&mut self) -> Result<Block, DukaError> {
        self.block()
    }

    fn block(&mut self) -> Result<Block, DukaError> {
        let mut stmts: Vec<Stmt> = vec![];

        many! {
            while let Some(stmt) = self.stmt()? {
                stmts.push(stmt)
            }
        };

        let ret_stmt = opt![self.ret_stmt()]?;

        Ok(Block(stmts, ret_stmt.map(|v| Box::new(v))))
    }

    fn ret_stmt(&mut self) -> TryDo<Stmt, DukaError> {
        if let Some((_, start_span)) = self.expect_token(TokenKind::Return)? {
            self.next_token()?;

            opt![self.then(TokenKind::SemiColon)]?;

            Ok(Some(self.span_end(StmtKind::Return(vec![]), start_span)))
        } else {
            Ok(None)
        }
    }

    fn stmt(&mut self) -> TryDo<Stmt, DukaError> {
        let (tk, start_span) = self.span_start()?;
        let kind = oneof!(match tk {
            TokenKind::SemiColon => StmtKind::Empty,
            TokenKind::Ident(_) => {
                self.when_ident()?;
                let tk = self.next_token()?;
                StmtKind::Expr(ExprKind::Call(
                    Box::new((ExprKind::Access(tk.into()), self.current_span)),
                    vec![self.exp()?.unwrap()],
                ))
            }
            TokenKind::DoubleColon => {
                let (_, start_span) = self.next_token()?;
                let ident = self.must_ident()?;
                self.must_token(TokenKind::DoubleColon)?;
                todo!()
            }
            TokenKind::Break => {
                self.next_token()?;
                todo!()
            }
            TokenKind::Continue => {
                self.next_token()?;
                todo!()
            }
            TokenKind::Goto => {
                self.next_token()?;
                todo!()
            }
            TokenKind::Local => {
                self.next_token()?;
                todo!()
            }
            TokenKind::Function => {
                todo!()
            }
            _ => return Ok(None),
        });
        Ok(Some(self.span_end(kind, start_span)))
    }

    fn when_ident(&mut self) -> Result<Stmt, DukaError> {
        todo!()
    }

    fn attr(&mut self) -> TryDo<String, DukaError> {
        if self.then(TokenKind::Less)? {
            if let Some((TokenKind::Ident(id), _)) = self.expect_ident()? {
                self.must_token(TokenKind::Greater)?;
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn label(&mut self) -> TryDo<String, DukaError> {
        if self.then(TokenKind::DoubleColon)? {
            if let Some((TokenKind::Ident(id), _)) = self.expect_ident()? {
                self.must_token(TokenKind::DoubleColon)?;
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn prefix_exp(&mut self) -> TryDo<ExprKind, DukaError> {
        //oneof!();
        todo!()
    }

    fn var(&mut self) -> TryDo<ExprKind, DukaError> {
        oneof!(
            if let Some((TokenKind::Ident(name), span)) = self.expect_ident()? {
            } else if let Some(x) = self.prefix_exp()? {
                if self.then(TokenKind::Dot)? {
                    let name = self.must_ident()?;
                } else {
                    self.must_token(TokenKind::LBracket)?;
                    let exp = must!(self.exp(), self, "expression")?;
                    self.must_token(TokenKind::RBracket)?;
                }
            } else {
                todo!()
            }
        );
        todo!()
    }

    fn func_name(&mut self) -> TryDo<(), DukaError> {
        if let Some((TokenKind::Ident(base), start_span)) = self.expect_ident()? {
            //let mut parts = vec![];

            many! {
                while self.then(TokenKind::Dot)?
                    && let Some((TokenKind::Ident(base), _)) = self.expect_ident()?
                {
                    self.next_token()?;
                }
            }

            opt![if self.then(TokenKind::Colon)?
                && let (TokenKind::Ident(last), _) = self.must_ident()?
            {}];

            todo!()
        } else {
            Ok(None)
        }
    }

    fn exp(&mut self) -> TryDo<Expr, DukaError> {
        let (tk, start_span) = self.span_start()?;
        let kind = match tk {
            TokenKind::Nil => {
                self.next_token()?;
                ExprKind::Literal(Value::Nil)
            }
            TokenKind::True => {
                self.next_token()?;
                ExprKind::Literal(Value::Bool(true))
            }
            TokenKind::False => {
                self.next_token()?;
                ExprKind::Literal(Value::Bool(false))
            }
            TokenKind::Float(f) => {
                let k = ExprKind::Literal(Value::Float(*f));
                self.next_token()?;
                k
            }
            TokenKind::Int(i) => {
                let k = ExprKind::Literal(Value::Int(*i));
                self.next_token()?;
                k
            }
            TokenKind::String(v) => {
                let k =
                    ExprKind::Literal(v.try_into().map_err(|kind: DukaLexerError| DukaError {
                        kind: kind.into(),
                        span: start_span,
                    })?);
                self.next_token()?;
                k
            }
            TokenKind::Dots => {
                self.next_token()?;
                ExprKind::VarArg
            }
            TokenKind::LBrace => {
                self.table_constructor()?;
                todo!();
            }
            _ => return Ok(None),
        };
        Ok(Some(self.span_end(kind, start_span)))
    }

    fn table_constructor(&mut self) -> TryDo<Expr, DukaError> {
        self.next_token()?; // already checked

        if self.then(TokenKind::RBrace)? {
            todo!()
        }

        self.field()?;

        many! {
            loop {
                self.must_token(TokenKind::Comma)?;
                if let Some(e) = self.field()? {

                    break;
                } else {
                    return Err(self.expecting("table field"))
                }
            }
        }

        self.then(TokenKind::Comma)?;

        self.must_token(TokenKind::RBrace)?;
        todo!();
    }

    fn field(&mut self) -> TryDo<Expr, DukaError> {
        todo!()
    }

    fn name_list(&mut self) -> Result<Expr, DukaError> {
        let (first, start_span) = self.must_ident()?;

        many! {
            while self.then(TokenKind::Comma)? {
                let (ident, _) = self.must_ident()?;
                todo!()
            }
        }

        todo!()
    }

    fn attr_name_list(&mut self) -> Result<Expr, DukaError> {
        let (first, start_span) = self.must_ident()?;
        let attr = opt![self.attr()]?;

        many! {
            while self.then(TokenKind::Comma)? {
                let (ident, _) = self.must_ident()?;
                let attr = opt![self.attr()]?;
                todo!()
            }
        }

        todo!()
    }

    fn exp_list(&mut self) -> Result<Expr, DukaError> {
        let first = self.exp()?;
        many! {
            while self.then(TokenKind::Comma)? {

            }
        }
        todo!()
    }

    fn par_list(&mut self) -> Result<Expr, DukaError> {
        oneof!(if self.then(TokenKind::Dots)? {
            todo!()
        } else {
            let ident = self.name_list()?;
            let dots = opt![self.expect_token(TokenKind::Dots)]?;
            todo!()
        })
    }
}

impl<Lexer: DukaLexer<Token>> Parser<Lexer> {
    #[inline(always)]
    fn err(&self, kind: DukaParserError) -> DukaError {
        DukaError {
            kind: kind.into(),
            span: self.current_span,
        }
    }

    #[inline(always)]
    fn span_start(&mut self) -> Result<RefToken, DukaError> {
        let (tk, sp) = self.peek_token(0)?;
        Ok((tk, *sp))
    }

    #[inline(always)]
    fn span_end<T>(&self, val: T, start: Span) -> Spanned<T> {
        (val, start + self.current_span)
    }

    #[inline(always)]
    fn then(&mut self, token: TokenKind) -> Result<bool, DukaError> {
        Ok(matches!(self.expect_token(token)?, Some(..)))
    }

    #[inline(always)]
    fn expect_ident(&mut self) -> TryDo<Token, DukaError> {
        self.expect(|t| matches!(t, TokenKind::Ident(..)))
    }

    #[inline(always)]
    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaError> {
        self.expect(|t| *t == token)
    }

    #[inline(always)]
    fn expecting(&mut self, msg: &str) -> DukaError {
        DukaError {
            kind: DukaParserError::UnexpectedToken(msg.to_string()).into(),
            span: self.peek_token(0).unwrap().1,
        }
    }

    #[inline]
    fn expect<T: FnOnce(&TokenKind) -> bool>(&mut self, predicate: T) -> TryDo<Token, DukaError> {
        match self.peek_token(0) {
            Ok((tk, _)) if predicate(tk) => {
                let (tk, sp) = self.next_token()?;
                Ok(Some((tk, sp)))
            }
            Ok(_) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[inline(always)]
    fn must_ident(&mut self) -> Result<Token, DukaError> {
        self.must(|t| matches!(t, TokenKind::Ident(..)), "ident")
    }

    #[inline(always)]
    fn must_token(&mut self, token: TokenKind) -> Result<Token, DukaError> {
        self.must(|t| *t == token, token.name())
    }

    #[inline]
    fn must<T: FnOnce(&TokenKind) -> bool>(
        &mut self,
        predicate: T,
        msg: &str,
    ) -> Result<Token, DukaError> {
        self.expect(predicate)?.ok_or(self.expecting(msg))
    }

    #[inline]
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaError> {
        while self.lookahead.len() <= n {
            match self.lexer.next() {
                Err(e) => return Err(e),
                Ok(t) if t.0.is_end() => break,
                item => self.lookahead.push_back(item),
            }
        }
        // error won't reach there
        // use unwrap() freely
        match self.lookahead.get(n) {
            Some(Ok(tk)) => Ok(tk),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(&(TokenKind::EOF, Span::EMPTY)),
        }
    }

    #[inline]
    fn next_token(&mut self) -> Result<Token, DukaError> {
        let res = if let Some(item) = self.lookahead.pop_front() {
            item
        } else {
            self.lexer.next()
        };
        res.inspect(|t| self.current_span = t.1)
    }
}
