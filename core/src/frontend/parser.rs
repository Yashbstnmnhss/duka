use std::collections::VecDeque;

use crate::{
    frontend::{
        ast::{
            Attr, AttrName, Block, Expr, ExprKind, Field, FuncBody, IfClause, Name, Param, Path,
            PathSuffix, Stmt, StmtKind, UnOp, get_binop_info,
        },
        token::{Token, TokenKind},
    },
    shared::{
        error::{DukaError, DukaLexerError, DukaParserError, Span},
        types::{DukaLexer, DukaParser, Spanned},
        utils::TryDo,
        value::Value,
    },
};

/// ## Marker []
/// optional
macro_rules! opt {
    [$($input:tt)*] => {
        $($input)*
    };
}
/// ## Marker {}
/// none or many
macro_rules! many {
    {loop: $($input:tt)*} => {
        loop {
            $($input)*
        }
    };
    {[$($input1:tt)*], loop: $($input2:tt)*} => {
        {$($input1)*;
        loop {
            $($input2)*
        }}
    };
    {$($input:tt)*} => {
        {$($input)*}
    };
}
/// ## Marker ()
/// one of them
macro_rules! oneof {
    ($($input:tt)*) => {
        {$($input)*}
    };
}
/// ## Marker ()
/// must be exactly
macro_rules! must {
    ($e: expr, $self: ident, $msg: expr) => {
        $e?.ok_or($self.expected($msg))
    };
    ($self: ident . $func: ident (), $msg: expr) => {
        must!($self.$func(), $self, $msg)
    };
    ($self: ident . $func: ident ()) => {
        must!($self.$func(), $self, concat!("<", stringify!($func), ">"))
    };
}
/// ## Marker ()
/// delimited between left and right
macro_rules! between {
    ($self: ident : try $inside: expr; in $left: expr, $right: expr) => {
        if $self.then($left)? {
            let start = $self.current_span;
            if let Some(tk) = $inside {
                $self.must_token($right)?;
                Some((tk.0, $self.current_span + start))
            } else {
                None
            }
        } else {
            None
        }
    };
    ($self: ident : opt($default: expr) $inside: expr; in $left: expr, $right: expr) => {{
        $self.must_token($left)?;
        if $self.then($right)? {
            $default
        } else {
            let i = $inside?;
            $self.must_token($right)?;
            i
        }
    }};
}

type RefToken<'a> = Spanned<&'a TokenKind>;

#[derive(Debug)]
pub struct Parser<Lexer: DukaLexer<Token>> {
    lexer: Lexer,
    lookahead: VecDeque<Result<Token, DukaError>>,

    current_span: Span,
}

#[derive(Debug)]
enum VarRes {
    Call(StmtKind),
    Var(Path),
}

impl<Lexer: DukaLexer<Token>> Parser<Lexer> {
    pub fn new(lexer: Lexer) -> Self {
        Self {
            lexer,
            lookahead: VecDeque::new(),

            current_span: Span::EMPTY,
        }
    }

    pub fn parse_chunk(&mut self) -> Result<Block, DukaError> {
        self.chunk()
    }

    fn chunk(&mut self) -> Result<Block, DukaError> {
        self.block(TokenKind::terminator())
    }

    fn block(&mut self, end_with: TokenKind) -> Result<Block, DukaError> {
        let mut stmts = vec![];

        Ok(many! {
            loop:
            if self.expect(|t| *t == end_with)?.is_some() {
                break Block(stmts, None)
            }

            if self.then(TokenKind::Return)? {
                let ret = self.ret_stmt()?;
                self.must(|t| *t == end_with, end_with.name())?;

                break Block(stmts, Some(Box::new(ret)))
            }

            let stmt = must!(self.stmt())?;
            stmts.push(stmt)
        })
    }

    fn stmt(&mut self) -> TryDo<Stmt, DukaError> {
        let (tk, start_span) = self.span_start()?;
        let kind = oneof!(match tk {
            TokenKind::SemiColon => {
                self.next_token()?;
                StmtKind::Empty
            }
            // functionCall or varlist
            TokenKind::Ident(_) => {
                self.when_ident()?
            }
            TokenKind::DoubleColon => {
                self.next_token()?; // consume "::"
                let (label, _) = self.must_ident()?;
                self.must_token(TokenKind::DoubleColon)?;
                StmtKind::Label(label)
            }
            TokenKind::Break => {
                self.next_token()?;
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.next_token()?;
                StmtKind::Continue
            }
            TokenKind::Goto => {
                self.next_token()?;
                let (label, _) = self.must_ident()?;
                StmtKind::Goto(label)
            }
            TokenKind::Local => {
                self.next_token()?;
                if self.then(TokenKind::Function)? {
                    self.function(true)?
                } else {
                    self.local_var()?
                }
            }
            TokenKind::Function => {
                self.next_token()?;
                self.function(false)?
            }
            TokenKind::If => {
                self.next_token()?;
                self.if_stmt()?
            }
            TokenKind::For => {
                self.next_token()?;
                self.for_stmt()?
            }
            TokenKind::While => {
                self.next_token()?;

                let cond = must!(self.exp())?;
                self.must_token(TokenKind::Do)?;
                let body = self.block(TokenKind::End)?;

                StmtKind::While(cond, body)
            }
            TokenKind::Do => {
                self.next_token()?;
                StmtKind::Do(self.block(TokenKind::End)?)
            }
            _ => return Ok(None),
        });
        Ok(Some(self.span_end(kind, start_span)))
    }

    fn function(&mut self, local: bool) -> Result<StmtKind, DukaError> {
        let name = must!(self.func_name())?;
        let body = self.func_body()?;
        Ok(StmtKind::Function(name, body, local))
    }

    fn local_var(&mut self) -> Result<StmtKind, DukaError> {
        let vars: Vec<AttrName> = self.attr_name_list()?;

        if self.then(TokenKind::Assign)? {
            let mut vals = vec![must!(self.exp())?];

            many! {
                while self.then(TokenKind::Comma)? {
                    vals.push(must!(self.exp())?)
                }
            }

            return Ok(StmtKind::Local(vars, vals));
        }

        Ok(StmtKind::Local(vars, vec![]))
    }

    fn ret_stmt(&mut self) -> Result<Stmt, DukaError> {
        let start_span = self.current_span;

        let exps = if self.then(TokenKind::SemiColon)? {
            vec![]
        } else {
            let r = opt![self.exp_list()]?
                .unwrap_or(vec![])
                .into_iter()
                .map(|f| f.0);
            opt![self.then(TokenKind::SemiColon)]?;
            r.collect()
        };

        Ok(self.span_end(StmtKind::Return(exps), start_span))
    }

    /// along with stmt()
    fn if_stmt(&mut self) -> Result<StmtKind, DukaError> {
        let cond = must!(self.exp())?.0;
        self.must_token(TokenKind::Then)?;

        let body = self.if_clause()?;

        let mut else_if_arms = vec![];
        many! {
            while self.then(TokenKind::Elseif)? {
                let cond = must!(self.exp())?.0;
                self.must_token(TokenKind::Then)?;
                let body = self.if_clause()?;

                else_if_arms.push(IfClause(body, cond));
            }
        }

        Ok(StmtKind::If(
            IfClause(body, cond),
            else_if_arms,
            opt![if self.then(TokenKind::Else)? {
                let else_body = self.block(TokenKind::End)?;
                Some(else_body)
            } else {
                self.must_token(TokenKind::End)?;
                None
            }],
        ))
    }
    /// along with if_stmt()
    fn if_clause(&mut self) -> Result<Block, DukaError> {
        Ok(many! {
            [let mut stmts = vec![]],
            loop:
            if self.lookahead_token(TokenKind::Else)?
                || self.lookahead_token(TokenKind::Elseif)?
                || self.lookahead_token(TokenKind::End)? {
                break Block(stmts, None);
            }
            if self.then(TokenKind::Return)? {
                let ret = self.ret_stmt()?;
                break Block(stmts, Some(Box::new(ret)));
            }

            let stmt = must!(self.stmt())?;
            stmts.push(stmt);
        })
    }

    fn for_stmt(&mut self) -> Result<StmtKind, DukaError> {
        Ok(oneof!(
            if matches!(self.peek_token(1)?, (TokenKind::Assign, _)) {
                let var = Path::Base(must!(self.simple_name())?);
                self.must_token(TokenKind::Assign)?;
                let init = must!(self.exp())?;

                self.must_token(TokenKind::Comma)?;
                let cond = must!(self.exp())?;

                let step = opt![if self.then(TokenKind::Comma)? {
                    Some(must!(self.exp())?)
                } else {
                    None
                }];

                self.must_token(TokenKind::Do)?;
                let body = self.block(TokenKind::End)?;

                StmtKind::ForNumberic(var, init, cond, step, body)
            } else {
                let vars = self
                    .name_list()?
                    .into_iter()
                    .map(|i| Path::Base(i))
                    .collect();

                self.must_token(TokenKind::In)?;

                let exps = must!(self.exp_list())?;

                self.must_token(TokenKind::Do)?;
                let body = self.block(TokenKind::End)?;

                StmtKind::ForGeneric(vars, exps, body)
            }
        ))
    }

    #[inline]
    fn when_ident(&mut self) -> Result<StmtKind, DukaError> {
        Ok(oneof!(match self.var()? {
            VarRes::Call(s) => s,
            VarRes::Var(name) => {
                let mut vars = vec![name];
                many! {
                    while self.then(TokenKind::Comma)? {
                        vars.push(match self.var()? {
                            VarRes::Var(var) => var,
                            _ => return Err(self.expected("<var>")),
                        });
                    }
                }

                self.must_token(TokenKind::Assign)?;

                let exps = must!(self.exp_list())?;

                StmtKind::Assign(vars, exps)
            }
        }))
    }

    #[inline]
    fn attr(&mut self) -> TryDo<Attr, DukaError> {
        let attr = between!(self: try self.expect_ident()?; in TokenKind::Less, TokenKind::Greater);
        Ok(attr)
    }

    #[inline(always)]
    fn simple_name(&mut self) -> TryDo<Name, DukaError> {
        self.expect_ident()
    }

    #[inline]
    fn attr_name(&mut self) -> Result<AttrName, DukaError> {
        let (name, span) = must!(self.simple_name(), "<identifier>")?;
        Ok((((name, span), self.attr()?), span))
    }

    // 涉及左递归
    fn prefix_exp(&mut self) -> TryDo<Expr, DukaError> {
        let (tk, start_span) = self.span_start()?;
        let mut res = oneof!(match tk {
            TokenKind::Function => {
                self.next_token()?;
                let func = self.func_body()?;
                self.span_end(ExprKind::Function(func), start_span)
            }
            TokenKind::LBrace => {
                let table = must!(self.table_constructor())?;
                self.span_end(table, start_span)
            }
            TokenKind::LParen => {
                self.next_token()?;
                let exp = must!(self.exp())?;
                self.must_token(TokenKind::RParen)?;
                exp
            }
            TokenKind::Ident(..) => {
                let name = self.must_ident()?;
                self.span_end(ExprKind::Access(Path::Base(name)), start_span)
            }
            _ => return Ok(None),
        });

        fn chain(former: Expr, new: PathSuffix, end: Span) -> Expr {
            let (kind, start) = former;
            (
                ExprKind::Access(if let ExprKind::Access(base) = kind {
                    base + new
                } else {
                    Path::Expr(Box::new((kind, start))) + new
                }),
                start + end,
            )
        }

        many! {
            loop:
            res = oneof! {
                if self.then(TokenKind::LBracket)? {
                    let exp = must!(self.exp())?;
                    self.must_token(TokenKind::RBracket)?;

                    chain(res, PathSuffix::Index(Box::new(exp)), self.current_span)
                } else if self.then(TokenKind::Dot)? {
                    let name = self.must_ident()?;

                    chain(res, PathSuffix::Dot(name), self.current_span)
                } else if self.then(TokenKind::Colon)? {
                    let name = self.must_ident()?;
                    let args = must!(self.args())?;
                    let func = chain(res, PathSuffix::Colon(name), self.current_span);

                    self.span_end(ExprKind::Call(Box::new(func), args), start_span)
                } else if let Some(args) = self.args()? {
                    self.span_end(ExprKind::Call(Box::new(res), args), start_span)
                } else {
                    break
                }
            }
        }

        Ok(Some(res))
    }

    fn var(&mut self) -> Result<VarRes, DukaError> {
        let mut base = oneof!(if self.then(TokenKind::LParen)? {
            let exp = must!(self.exp())?;
            self.must_token(TokenKind::RParen)?;

            let suffix = must!(self.var_suffix(), "., [], etc")?;
            let base = Path::Expr(Box::new(exp));
            base + suffix
        } else {
            let name = self.must_ident()?;
            Path::Base(name)
        });

        many! {
            loop:
            if let Some(suffix) = self.var_suffix()? {
                if let PathSuffix::Colon(_) = suffix {
                    let args = must!(self.args())?;

                    match self.var_func_suffix(base + suffix, args)? {
                        t @ VarRes::Call(_) => return Ok(t),
                        VarRes::Var(p) => base = p,
                    }
                } else {
                    base = base + suffix
                }
            }
            else if let Some(args) = self.args()? {
                match self.var_func_suffix(base, args)? {
                    t @ VarRes::Call(_) => return Ok(t),
                    VarRes::Var(p) => base = p,
                }
            } else {
                break
            }
        }

        Ok(if let Some(args) = self.args()? {
            let span = self.current_span;
            VarRes::Call(StmtKind::Call(
                self.span_end(ExprKind::Access(base), span),
                args,
            ))
        } else {
            VarRes::Var(base)
        })
    }
    fn var_suffix(&mut self) -> TryDo<PathSuffix, DukaError> {
        Ok(Some(oneof! {
            if self.then(TokenKind::LBracket)? {
                let exp = must!(self.exp())?;
                self.must_token(TokenKind::RBracket)?;

                PathSuffix::Index(Box::new(exp))
            } else if self.then(TokenKind::Dot)? {
                let name = self.must_ident()?;

                PathSuffix::Dot(name)
            } else if self.then(TokenKind::Colon)? {
                let name = self.must_ident()?;

                PathSuffix::Colon(name)
            } else {
                return Ok(None);
            }
        }))
    }
    fn var_func_suffix(&mut self, base: Path, args: Vec<Expr>) -> Result<VarRes, DukaError> {
        let span = self.current_span;
        Ok(if let Some(suffix) = self.var_suffix()? {
            let call = ExprKind::Call(Box::new(self.span_end(ExprKind::Access(base), span)), args);

            VarRes::Var(Path::Expr(Box::new(self.span_end(call, span))) + suffix)
        } else {
            VarRes::Call(StmtKind::Call(
                self.span_end(ExprKind::Access(base), span),
                args,
            ))
        })
    }

    fn func_name(&mut self) -> TryDo<Path, DukaError> {
        if let Some((base, start_span)) = self.expect_ident()? {
            let mut base = Path::Base((base, start_span));

            many! {
                while self.then(TokenKind::Dot)?
                {
                    let name = self.must_ident()?;
                    base = base + PathSuffix::Dot(name);
                }
            }

            opt![if self.then(TokenKind::Colon)? {
                let last = self.must_ident()?;
                base = base + PathSuffix::Colon(last);
            }];

            Ok(Some(base))
        } else {
            Ok(None)
        }
    }

    fn func_body(&mut self) -> Result<FuncBody, DukaError> {
        let params =
            between!(self: opt(vec![]) self.par_list(); in TokenKind::LParen, TokenKind::RParen);
        let body = self.block(TokenKind::End)?;

        Ok(FuncBody(params, body))
    }

    fn exp(&mut self) -> TryDo<Expr, DukaError> {
        self.exp_limit(0)
    }

    #[inline]
    fn exp_limit(&mut self, limit: u8) -> TryDo<Expr, DukaError> {
        let (mut exp, start_span) = match self.atom_exp()? {
            Some(e) => e,
            None => return Ok(None),
        };

        Ok(Some(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_binop() {
                break self.span_end(exp, start_span)
            }

            if let Some((op, (l, r))) = get_binop_info(tk)
            {
                if l <= limit {
                    break (exp, start_span)
                }

                // consume op
                self.next_token()?;
                let right = match self.exp_limit(r)? {
                    Some(r) => r,
                    None => return Err(self.expected("<exp>")),
                };
                exp = ExprKind::Binary(Box::new(self.span_end(exp, start_span)), Box::new(right), op)
            } else {
                return Err(
                    DukaError {
                        kind: DukaParserError::UnknownOperator(tk.name().to_owned()).into(),
                        span: self.current_span,
                    }
                )
            }
        }))
    }

    fn atom_exp(&mut self) -> TryDo<Expr, DukaError> {
        oneof!(if let Some(res) = self.prefix_exp()? {
            Ok(Some(res))
        } else {
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
                    let k = ExprKind::Literal(v.try_into().map_err(|kind: DukaLexerError| {
                        DukaError {
                            kind: kind.into(),
                            span: start_span,
                        }
                    })?);
                    self.next_token()?;
                    k
                }
                TokenKind::Dots => {
                    self.next_token()?;
                    ExprKind::VarArg
                }
                TokenKind::LBrace => must!(self.table_constructor())?,
                TokenKind::Function => {
                    self.next_token()?;
                    ExprKind::Function(self.func_body()?)
                }
                t if t.is_unop() => self.unop_exp()?,
                _ => return Ok(None),
            };
            Ok(Some(self.span_end(kind, start_span)))
        })
    }

    fn unop_exp(&mut self) -> Result<ExprKind, DukaError> {
        let tk = self.next_token()?.0;
        Ok(ExprKind::Unary(
            Box::new(must!(self.exp())?),
            match tk {
                TokenKind::Minus => UnOp::Minus,
                TokenKind::Not => UnOp::Not,
                TokenKind::Length => UnOp::Length,
                TokenKind::BitTilde => UnOp::BitNot,
                _ => unreachable!(),
            },
        ))
    }

    fn args(&mut self) -> TryDo<Vec<Expr>, DukaError> {
        let (tk, start_span) = self.span_start()?;
        Ok(Some(oneof!(match tk {
            TokenKind::LParen =>
                between!(self: opt(Some(vec![])) self.exp_list(); in TokenKind::LParen, TokenKind::RParen)
                    .unwrap_or(vec![]),
            TokenKind::LBrace => {
                let table = must!(self.table_constructor())?;
                vec![self.span_end(table, start_span)]
            }
            TokenKind::String(val) => {
                let str = ExprKind::Literal(val.try_into().map_err(|kind: DukaLexerError| {
                    DukaError {
                        kind: kind.into(),
                        span: start_span,
                    }
                })?);
                self.next_token()?;
                vec![self.span_end(str, start_span)]
            }
            _ => return Ok(None),
        })))
    }

    /// already checked
    fn table_constructor(&mut self) -> TryDo<ExprKind, DukaError> {
        self.next_token()?; // already checked
        let mut fields = vec![];

        if !self.then(TokenKind::RBrace)? {
            self.field()?.and_then(|f| {
                fields.push(f);
                Some(())
            });

            many! {
                while self.then(TokenKind::Comma)?
                    || self.then(TokenKind::SemiColon)?
                {
                    // {...,}
                    if self.lookahead_token(TokenKind::RBrace)? {
                        break
                    } else {
                        let f = must!(self.field())?;
                        fields.push(f)
                    }
                }
            }

            self.must_token(TokenKind::RBrace)?;
        }

        Ok(Some(ExprKind::Table(fields)))
    }

    fn field(&mut self) -> TryDo<Field, DukaError> {
        Ok(oneof!(if self.then(TokenKind::LBracket)? {
            let key = must!(self.exp())?;

            self.must_token(TokenKind::RBracket)?;
            self.must_token(TokenKind::Assign)?;

            let val = must!(self.exp())?;

            Some(Field::KeyValue(key, val))
        } else if let Some((key, start_span)) = self.expect_ident()? {
            self.must_token(TokenKind::Assign)?;

            let val = must!(self.exp())?;

            Some(Field::NameValue((key, start_span), val))
        } else {
            self.exp()?.map(Field::Value)
        }))
    }

    fn name_list(&mut self) -> Result<Vec<Name>, DukaError> {
        let first = self.must_ident()?;
        let mut res = vec![first];

        many! {
            while self.then(TokenKind::Comma)? {
                let name = self.must_ident()?;
                if res.iter().any(|i| i.0.eq(&name.0)) {
                    return Err(self.err(DukaParserError::DuplicatedName(name.0)));
                }
                res.push(name)
            }
        }

        Ok(res)
    }

    fn attr_name_list(&mut self) -> Result<Vec<AttrName>, DukaError> {
        let mut res = vec![self.attr_name()?];

        many! {
            while self.then(TokenKind::Comma)? {
                res.push(self.attr_name()?);
            }
        }

        Ok(res)
    }

    fn exp_list(&mut self) -> TryDo<Vec<Expr>, DukaError> {
        let first = match self.exp()? {
            None => return Ok(None),
            Some(e) => e,
        };
        let mut res = vec![first];

        many! {
            while self.then(TokenKind::Comma)? {
                let expr = must!(self.exp())?;
                res.push(expr)
            }
        }

        Ok(Some(res))
    }

    fn par_list(&mut self) -> Result<Vec<Param>, DukaError> {
        Ok(oneof!(if self.then(TokenKind::Dots)? {
            vec![Param::Var(self.current_span)]
        } else {
            let mut res: Vec<Param> = self.name_list()?.into_iter().map(Param::Name).collect();

            opt![if self.then(TokenKind::Dots)? {
                res.push(Param::Var(self.current_span))
            }];

            res
        }))
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
        Ok(self.expect_token(token)?.is_some())
    }

    #[inline(always)]
    fn lookahead_token(&mut self, token: TokenKind) -> Result<bool, DukaError> {
        Ok(matches!(self.peek_token(0)?, (tk, _) if *tk == token))
    }

    #[inline(always)]
    fn expect_ident(&mut self) -> TryDo<Spanned<String>, DukaError> {
        self.expect(|t| matches!(t, TokenKind::Ident(..))).map(|t| {
            if let Some((TokenKind::Ident(ident), span)) = t {
                Some((ident, span))
            } else {
                None
            }
        })
    }

    #[inline(always)]
    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaError> {
        self.expect(|tk| *tk == token)
    }

    #[inline(always)]
    fn expected(&mut self, msg: &str) -> DukaError {
        DukaError {
            kind: DukaParserError::UnexpectedToken(msg.to_string()).into(),
            span: self.peek_token(0).unwrap().1,
        }
    }

    #[inline]
    fn expect<T: FnOnce(&TokenKind) -> bool>(&mut self, predicate: T) -> TryDo<Token, DukaError> {
        Ok(match self.peek_token(0)? {
            (tk, _) if predicate(tk) => Some(self.next_token()?),
            _ => None,
        })
    }

    #[inline(always)]
    fn must_ident(&mut self) -> Result<Spanned<String>, DukaError> {
        match self.peek_token(0)? {
            (TokenKind::Ident(..), _) => {
                if let (TokenKind::Ident(ident), span) = self.next_token()? {
                    Ok((ident, span))
                } else {
                    unreachable!()
                }
            }
            (tk, span) => Err(DukaError {
                kind: DukaParserError::UnexpectedToken(if tk.is_keyword() {
                    format!("<identifier>, found keyword {}", tk.name())
                } else {
                    "<identifier>".to_string()
                })
                .into(),
                span: *span,
            }),
        }
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
        self.expect(predicate)?.ok_or(self.expected(msg))
    }

    #[inline]
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaError> {
        const MAX_DEPTH: usize = 3;
        if n > MAX_DEPTH {
            panic!("Do not use too many peek")
        }

        while self.lookahead.len() <= n {
            match self.lexer.next() {
                Err(e) => return Err(e),
                Ok(t) if t.0.is_terminator() => break,
                item => self.lookahead.push_back(item),
            }
        }
        self.lookahead
            .get(n)
            .map(|r| r.as_ref())
            .transpose()
            .map(|o| o.unwrap_or(&(TokenKind::EOF, Span::EMPTY)))
            .map_err(|e| e.clone())
    }

    #[inline]
    fn next_token(&mut self) -> Result<Token, DukaError> {
        self.lookahead
            .pop_front()
            .unwrap_or_else(|| self.lexer.next())
            .inspect(|t| self.current_span = t.1)
    }
}

impl<Lexer: DukaLexer<Token>> DukaParser for Parser<Lexer> {
    fn parse(&mut self) -> Result<Block, DukaError> {
        self.parse_chunk()
    }
}
