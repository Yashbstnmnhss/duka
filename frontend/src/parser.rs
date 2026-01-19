use std::{cell::RefCell, rc::Rc, u8};

use duka_shared::{
    ast::{
        AttrName, Attrs, Block, Expr, ExprKind, Field, FieldPattern, FuncBody, If, IfClause, Linq,
        LinqClause, Match, MatchClause, Name, ObjectDef, Param, Path, PathSuffix, PatternArrayTerm,
        PatternTerm, Stmt, StmtKind, UnOp, get_binop_info, get_patop_info,
    },
    constants::{clex, cpar},
    error::{DukaLexerError, DukaParserError, DukaSpannedError, Span},
    token::{EMPTY_TOKEN, Token, TokenKind},
    types::{
        DukaChunk, DukaLexer, DukaParser, Fact, Goal, LogicDatabase, LogicOp, RawToken, Rule,
        Spanned, Term, get_logicop_info,
    },
    utils::{MultiPeekable, MultiPeekableExtension, OrError, TryDo},
    value::{ArrayMap, ConstValue},
};

/// ## Marker []
/// optional
macro_rules! opt {
    [$self: ident then $tk: ident: {$($input:tt)*} else: $($input2:tt)*] => {
        if $self.then(TokenKind::$tk)? {
            $($input)*
        } else {
            $($input2)*
        }
    };
    [$self: ident then $tk: ident: $($input:tt)*] => {
        if $self.then(TokenKind::$tk)? {
            $($input)*
        }
    };
    [$self: ident then $tk: ident] => {
        $self.then(TokenKind::$tk)?;
    };
    [$($input:tt)*] => {
        $($input)*
    };
}
/// ## Marker {}
/// none or many
macro_rules! many {
    {
        $self: ident then $tk: ident $(or $tks: ident)*:
        $($input:tt)*
    } => {
        while $self.then(TokenKind::$tk)?
        $(|| $self.then(TokenKind::$tks)?)* {
            $($input)*
        }
    };
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
    (if:
        case $c: expr => { $($if:tt)* },
        $(case $cs: expr => {$($ifs:tt)*}),*
    $(else: $($else:tt)*)?
    ) => {
        if $c {
            $($if)*
        }
        $(else if $cs { $($ifs)* })*
        $(else {
            $($else)*
        })?
    };
    (try match $target: expr => {$($input:tt)*} else: $($input2:tt)*) => {
        match $target {$($input)*, _ => {$($input2)*}}
    };
    (try match $target: expr => $($input:tt)*) => {
        oneof!(try match $target => { $($input)* } else: return Ok(None))
    };
    (err match $target: expr; $self: ident($e: expr) => $($input:tt)*) => {
        match $target {
            $($input)*,
            _ => return Err(
                $self.err($e)
            )
        }
    };
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
    ($self: ident . $func: ident ($($p: expr),*), $msg: expr) => {
        must!($self.$func($($p),*), $self, $msg)
    };
    ($self: ident . $func: ident ($($p: expr),*)) => {
        must!($self.$func($($p),*), $self, concat!("<", stringify!($func), ">"))
    };
}
/// ## Marker ()
/// delimited between left and right
macro_rules! between {
    ($self: ident : try[$default: expr] nonempty($inside: expr) in $left: ident, $right: ident) => {
        if $self.then(TokenKind::$left)? {
            let i = $inside?;
            $self.must_token(TokenKind::$right)?;
            i
        } else {
            $default
        }
    };
    ($self: ident : must opt($inside: expr)[$default: expr] in $left: ident, $right: ident) => {{
        $self.must_token(TokenKind::$left)?;
        if $self.then(TokenKind::$right)? {
            $default
        } else {
            let i = $inside?;
            $self.must_token(TokenKind::$right)?;
            i
        }
    }};
    ($self: ident : must nonempty($inside: expr) in $left: ident, $right: ident) => {{
        $self.must_token(TokenKind::$left)?;
        let i = $inside?;
        $self.must_token(TokenKind::$right)?;
        i
    }};
}
/// ## Marker ()
/// several items separated by separator
macro_rules! list {
    ($self: ident:
        by $tk: ident separate ($inside: expr)
        nonempty
    ) => {{
        let first = $inside?;
        let mut res = vec![first];

        many! {
            $self then $tk:
            let item = $inside?;
            res.push(item)
        }

        res
    }};
    ($self: ident:
        by $tk: ident separate ($_: ident . $func: ident ($($p: expr),*))
        empty[None]
    ) => {{
        if let Some(first) = $self.$func($($p),*)? {
            let mut res = vec![first];

            many! {
                $self then $tk:
                let expr = must!($self.$func($($p),*))?;
                res.push(expr)
            }

            Some(res)
        } else {
            None
        }
    }};
}

type RefToken<'a> = Spanned<&'a TokenKind>;

#[derive(Debug)]
pub struct Parser<I>
where
    I: Iterator<Item = RawToken<Token>>,
{
    tokens: MultiPeekable<I>,
    // lookahead: VecDeque<RawToken<T>>,
    current_span: Span,

    logic: LogicDatabase,
}

#[derive(Debug)]
enum VarRes {
    Call(StmtKind),
    Var(Path),
}

/// main duka
impl<I: Iterator<Item = RawToken<Token>>> Parser<I> {
    pub fn new(tokens: I) -> Self {
        Self {
            tokens: tokens.multi_peekable(),
            // lookahead: VecDeque::new(),
            current_span: Span::default(),

            logic: LogicDatabase::default(),
        }
    }

    pub fn parse_chunk(&mut self) -> Result<Block, DukaSpannedError> {
        self.chunk()
    }

    fn chunk(&mut self) -> Result<Block, DukaSpannedError> {
        self.block([TokenKind::terminator()])
    }

    fn block<const C: usize>(
        &mut self,
        end_withs: [TokenKind; C],
    ) -> Result<Block, DukaSpannedError> {
        let mut stmts = vec![];

        Ok(many! {
            loop:
            if self.expect(|t| end_withs.contains(t))?.is_some() {
                break Block(stmts, None)
            }

            if self.then(TokenKind::Return)? {
                let ret = self.ret_stmt()?;
                self.must(|t| end_withs.contains(t), end_withs[0].name())?;

                break Block(stmts, Some(Box::new(ret)))
            }

            let stmt = must!(self.stmt())?;
            stmts.push(stmt)
        })
    }

    fn stmt(&mut self) -> TryDo<Stmt, DukaSpannedError> {
        let (tk, start_span) = self.span_start()?;
        let kind = oneof!(
            try match tk =>
            TokenKind::LParen => {
                self.next_token()?;
                let expr = must!(self.exp())?;
                self.must_token(TokenKind::RParen)?;
                StmtKind::Expr(expr)
            }
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
            TokenKind::Local | TokenKind::Global => {
                let tk = self.next_token()?.0;
                let global = matches!(tk, TokenKind::Global);
                if self.then(TokenKind::Function)? {
                    self.function(global)?
                } else {
                    self.attr_var(global)?
                }
            }
            TokenKind::Function => {
                self.next_token()?;
                self.function(false)?
            }
            TokenKind::If => {
                self.next_token()?;
                StmtKind::If(self.if_block(false)?)
            }
            TokenKind::Match => {
                self.next_token()?;
                StmtKind::Match(self.match_block(false)?)
            }
            TokenKind::For => {
                self.next_token()?;
                self.for_stmt()?
            }
            TokenKind::While => {
                self.next_token()?;

                let cond = must!(self.exp())?;
                self.must_token(TokenKind::Do)?;
                let body = self.block([TokenKind::End])?;

                StmtKind::While(cond, body)
            }
            TokenKind::Do => {
                self.next_token()?;
                StmtKind::Do(self.block([TokenKind::End])?)
            }
            TokenKind::Object => {
                self.next_token()?;
                StmtKind::Object(self.object()?)
            }
        );
        Ok(Some(self.stmt_end(kind, start_span)))
    }

    fn match_block(&mut self, must_else: bool) -> Result<Match, DukaSpannedError> {
        let target = must!(self.exp())?;

        self.must_token(TokenKind::Then)?;

        let mut clauses = vec![];
        many! {
            loop:

            if self.then(TokenKind::End)? {
                must_else.then_error(||
                    self.err(
                        DukaParserError::UnexpectedToken(TokenKind::Else.name().to_owned())
                    )
                )?;
                return Ok(Match(Box::new(target), clauses, None))
            }
            if self.then(TokenKind::Else)? {
                break
            }

            let clause = self.match_clause()?;
            clauses.push(clause);
        }

        let else_clause = self.block([TokenKind::End])?;

        Ok(Match(Box::new(target), clauses, Some(else_clause)))
    }

    fn match_clause(&mut self) -> Result<MatchClause, DukaSpannedError> {
        let pattern = self.match_pattern(0)?;

        let guard = opt![
            self then If: {
                let expr = must!(self.exp())?;
                Some(expr)
            }
            else: None
        ];

        self.must_token(TokenKind::Arrow)?;

        let block = oneof!(if:
            case self.then(TokenKind::Do)? => {
                let block = self.block([TokenKind::End])?;
                opt![self then SemiColon];
                block
            },
            else:
                let Expr(expr, span) = must!(self.exp())?;
                self.must_token(TokenKind::SemiColon)?;
                let stmt = StmtKind::Return(vec![Expr(expr, span)]);
                Block(vec![], Some(Box::new(Stmt(stmt, span))))
        );

        Ok(MatchClause((pattern, guard), block))
    }

    fn match_pattern(&mut self, limit: u8) -> Result<PatternTerm, DukaSpannedError> {
        let mut pattern = self.match_atom_pattern()?;

        Ok(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_patop() {
                break pattern
            }

            if let Some((op, (l, r))) = get_patop_info(tk)
            {
                if l <= limit {
                    break pattern
                }

                // consume op
                self.next_token()?;
                let right = self.match_pattern(r)?;
                pattern = PatternTerm::Compound(Box::new(pattern), Box::new(right), op);
            } else {
                return Err(
                    DukaSpannedError {
                        kind: DukaParserError::UnknownOperator(tk.name().to_owned()).into(),
                        span: self.current_span,
                    }
                )
            }
        })
    }

    fn match_atom_pattern(&mut self) -> Result<PatternTerm, DukaSpannedError> {
        Ok(oneof!(
            try match self.peek_token(0)?.0 => {
                TokenKind::Pipeline => {
                    self.next_token()?;
                    let func = must!(self.atom_exp())?;
                    PatternTerm::Call(Box::new(func))
                },
                TokenKind::LParen => between!(self:
                    must nonempty(self.match_pattern(0))
                    in LParen, RParen
                ),
                TokenKind::LBrace => between!(self:
                    must opt(self.match_atom_table_pattern())[PatternTerm::Table(vec![])]
                    in LBrace, RBrace
                ),
                TokenKind::Not => {
                    self.next_token()?;
                    let inner = self.match_pattern(u8::MAX)?;
                    PatternTerm::Not(Box::new(inner))
                },
                TokenKind::Local => {
                    self.next_token()?;
                    let name = self.must_ident()?;
                    PatternTerm::Bind(name)
                }

                ref t if t.is_compare() => {
                    let tk = self.next_token()?;
                    let Some((op, _)) = get_binop_info(&tk.0) else {
                        return Err(DukaSpannedError {
                            kind: DukaParserError::UnknownOperator(tk.0.name().to_owned()).into(),
                            span: self.current_span,
                        })
                    };
                    let right = must!(self.atom_exp())?;
                    PatternTerm::Compare(op, Box::new(right))
                }
            } else:
                let expr = must!(self.atom_exp())?;
                PatternTerm::Constant(Box::new(expr))
        ))
    }

    fn match_atom_table_pattern(&mut self) -> Result<PatternTerm, DukaSpannedError> {
        let mut fields = vec![];

        fields.push(self.match_field_pattern()?);

        many! {
            self then Comma or SemiColon:
            fields.push(self.match_field_pattern()?);
        }

        Ok(PatternTerm::Table(fields))
    }

    fn match_field_pattern(&mut self) -> Result<FieldPattern, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::LBracket)? {
            let key = must!(self.exp())?;

            self.must_token(TokenKind::RBracket)?;
            self.must_token(TokenKind::Assign)?;

            let pattern = self.match_pattern(0)?;

            FieldPattern::Expr(key, pattern)
        } else if self.lookahead_token(TokenKind::Assign, 1)? {
            let key = self.must_ident()?;
            print!("{}", key.0);
            self.must_token(TokenKind::Assign)?;

            let pattern = self.match_pattern(0)?;

            FieldPattern::Named(key, pattern)
        } else {
            let pattern = oneof!(if self.then(TokenKind::Dots)? {
                PatternArrayTerm::DiscardMany
            } else if self
                .expect(|t| matches!(t, TokenKind::Ident(id) if id == "_"))?
                .is_some()
            {
                PatternArrayTerm::Discard(opt![
                    self then Multiply: {
                        let TokenKind::Int(times) = self
                            .must(|t| matches!(t, TokenKind::Int(..)), cpar::INT)?
                            .0
                        else {
                            unreachable!()
                        };
                        times as usize
                    } else: 1 ])
            } else {
                let term = self.match_pattern(0)?;
                PatternArrayTerm::Term(term)
            });
            FieldPattern::Array(pattern)
        }))
    }

    // TODO
    fn object(&mut self) -> Result<ObjectDef, DukaSpannedError> {
        let name = self.must_ident()?;

        // object A: B
        //     property = 1;
        //     do ... end
        //     function A() end
        //     function self:A() end
        // end

        let base = opt![
            self then Colon:
            { Some(self.must_ident()?) }
            else: None
        ];

        self.must_token(TokenKind::End)?;

        todo!()
    }
    fn object_item(&mut self) -> Result<(), DukaSpannedError> {
        todo!()
    }

    fn function(&mut self, global: bool) -> Result<StmtKind, DukaSpannedError> {
        let attrs = self.attrs()?;
        let name = must!(self.func_name())?;
        let body = self.func_body()?;
        Ok(StmtKind::Function(name, attrs, body, global))
    }

    fn attr_var(&mut self, global: bool) -> Result<StmtKind, DukaSpannedError> {
        let vars: Vec<AttrName> = self.attr_name_list()?;

        Ok(StmtKind::Define(
            vars,
            opt![
                self then Assign: {
                    let mut vals = vec![must!(self.exp())?];

                    many! {
                        self then Comma:
                        vals.push(must!(self.exp())?)
                    }

                    vals
                }
                else: vec![]
            ],
            global,
        ))
    }

    fn ret_stmt(&mut self) -> Result<Stmt, DukaSpannedError> {
        let start_span = self.current_span;

        let exps = if self.then(TokenKind::SemiColon)? {
            vec![]
        } else {
            let result = opt![self.exp_list()]?.unwrap_or_default();
            opt![self then SemiColon];
            result
        };

        Ok(self.stmt_end(StmtKind::Return(exps), start_span))
    }

    /// along with stmt(), expr()
    fn if_block(&mut self, must_else: bool) -> Result<If, DukaSpannedError> {
        let cond = must!(self.exp())?;
        self.must_token(TokenKind::Then)?;

        let body = self.block([TokenKind::Else, TokenKind::Elseif, TokenKind::End])?;

        let mut else_if_arms = vec![];
        many! {
            self then Elseif:
            let cond = must!(self.exp())?;
            self.must_token(TokenKind::Then)?;
            let body = self.block([TokenKind::Else, TokenKind::Elseif, TokenKind::End])?;

            else_if_arms.push(IfClause(body, Box::new(cond)));
        }

        Ok(If(
            IfClause(body, Box::new(cond)),
            else_if_arms,
            opt![self then Else: {
                let else_body = self.block([TokenKind::End])?;
                Some(else_body)
            } else:
                must_else.then_error(||
                    self.err(
                        DukaParserError::UnexpectedToken(TokenKind::Else.name().to_owned())
                    )
                )?;
                self.must_token(TokenKind::End)?;
                None
            ],
        ))
    }

    fn for_stmt(&mut self) -> Result<StmtKind, DukaSpannedError> {
        Ok(oneof!(if:
        case self.lookahead_token(TokenKind::Assign, 1)? => {
            let var = Path::Base(must!(self.simple_name())?);
            self.must_token(TokenKind::Assign)?;
            let init = must!(self.exp())?;

            self.must_token(TokenKind::Comma)?;
            let cond = must!(self.exp())?;

            let step = opt![self then Comma: {
                Some(must!(self.exp())?)
            }
            else: None];

            self.must_token(TokenKind::Do)?;
            let body = self.block([TokenKind::End])?;

            StmtKind::ForNumberic(var, init, cond, step, body)
        },
        else:
            let vars = self
                .name_list()?
                .into_iter()
                .map(|i| Path::Base(i))
                .collect();

            self.must_token(TokenKind::In)?;

            let exps = must!(self.exp_list())?;

            self.must_token(TokenKind::Do)?;
            let body = self.block([TokenKind::End])?;

            StmtKind::ForGeneric(vars, exps, body)
        ))
    }

    fn bang_stmt(&mut self, name: Name) -> Result<StmtKind, DukaSpannedError> {
        self.must_token(TokenKind::LBrace)?;
        let res = oneof!(
            err match name.0.as_str();
                self(DukaParserError::UnexpectedToken("logic".to_owned()))
            =>
            "logic" => {
                self.logic_block()?;
                StmtKind::Extern
            }
        );
        self.must_token(TokenKind::RBrace)?;
        Ok(res)
    }
    fn bang_expr(&mut self, name: Name) -> Result<ExprKind, DukaSpannedError> {
        self.must_token(TokenKind::LParen)?;
        let res = oneof!(
            err match name.0.as_str();
                self(DukaParserError::UnexpectedToken("logic, linq".to_owned()))
            =>
            "logic" => {
                ExprKind::Empty
            }
            "linq" => {
                ExprKind::Linq(
                    self.linq_expr()?
                )
            }
        );
        self.must_token(TokenKind::RParen)?;
        Ok(res)
    }

    fn linq_expr(&mut self) -> Result<Linq, DukaSpannedError> {
        self.must_keyword("from")?;
        let name = must!(self.simple_name())?;
        self.then(TokenKind::In)?;
        let expr = must!(self.exp())?;

        let from = LinqClause::From(name, Box::new(expr));
        let mut clauses = vec![from];

        many! {
            loop:
            let Some(clause) = self.linq_clause()? else { break };
            clauses.push(clause)
        }

        self.must_keyword("select")?;
        let select = must!(self.exp())?;

        Ok(Linq(clauses, Box::new(select)))
    }
    fn linq_clause(&mut self) -> TryDo<LinqClause, DukaSpannedError> {
        Ok(Some(oneof!(if:
            case self.then_keyword("from")? => {
                let name = must!(self.simple_name())?;
                self.then(TokenKind::In)?;
                let expr = must!(self.exp())?;
                LinqClause::From(name, Box::new(expr))
            },
            case self.then_keyword("where")? => {
                let expr = must!(self.exp())?;
                LinqClause::Where(Box::new(expr))
            }
            else: return Ok(None)
        )))
    }

    #[inline]
    fn when_ident(&mut self) -> Result<StmtKind, DukaSpannedError> {
        oneof!(if self.lookahead_token(TokenKind::Bang, 1)? {
            let name = self.must_ident()?;
            self.must_token(TokenKind::Bang)?;
            self.bang_stmt(name)
        } else {
            let (res, span) = self.var()?;
            Ok(oneof!(match res {
                VarRes::Call(call) => call,
                VarRes::Var(name) => {
                    if let Some(op) = self.expect(TokenKind::is_binop)?
                        && let Some((binop, _)) = get_binop_info(&op.0)
                        && !binop.is_compare()
                    {
                        self.must_token(TokenKind::Assign)?;
                        let Expr(right, exp_span) = must!(self.exp())?;

                        StmtKind::Assign(
                            vec![name.clone()],
                            vec![Expr(
                                ExprKind::Binary(
                                    Box::new(Expr(ExprKind::Access(name), span)),
                                    Box::new(Expr(right, exp_span)),
                                    binop,
                                ),
                                span + exp_span,
                            )],
                        )
                    } else {
                        let mut vars = vec![name];
                        many! {
                            self then Comma:
                            vars.push(match self.var()?.0 {
                                VarRes::Var(var) => var,
                                _ => return Err(self.expected(cpar::VAR)),
                            });
                        }

                        self.must_token(TokenKind::Assign)?;

                        let exps = must!(self.exp_list())?;

                        StmtKind::Assign(vars, exps)
                    }
                }
            }))
        })
    }

    #[inline]
    fn attr_list(&mut self) -> Result<Attrs, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (must!(self.expect_ident()))
            nonempty
        ))
    }
    #[inline]
    fn attrs(&mut self) -> Result<Attrs, DukaSpannedError> {
        let attrs = between!(self:
            try[vec![]] nonempty(self.attr_list())
            in Less, Greater
        );

        Ok(attrs)
    }

    #[inline(always)]
    fn simple_name(&mut self) -> TryDo<Name, DukaSpannedError> {
        self.expect_ident()
    }

    #[inline]
    fn attr_name(&mut self) -> Result<AttrName, DukaSpannedError> {
        let (name, span) = must!(self.simple_name(), clex::ID)?;
        Ok((((name, span), self.attrs()?), span))
    }

    // 涉及左递归
    fn prefix_exp(&mut self) -> TryDo<Expr, DukaSpannedError> {
        let (tk, start_span) = self.span_start()?;
        let mut res = oneof!(
            try match tk =>
            TokenKind::Function => {
                self.next_token()?;
                let func = self.func_body()?;
                self.expr_end(ExprKind::Function(func), start_span)
            }
            TokenKind::LBrace => {
                let table = must!(self.table_constructor())?;
                self.expr_end(table, start_span)
            }
            TokenKind::LParen => {
                self.next_token()?;
                let exp = must!(self.exp())?;
                self.must_token(TokenKind::RParen)?;
                exp
            }
            TokenKind::Ident(..) => {
                let name = self.must_ident()?;

                if self.then(TokenKind::Bang)? {
                    let bang = self.bang_expr(name)?;
                    return Ok(Some(self.expr_end(bang, start_span)))
                }

                self.expr_end(ExprKind::Access(Path::Base(name)), start_span)
            }
        );

        fn chain(former: Expr, new: PathSuffix, end: Span) -> Expr {
            let Expr(kind, start) = former;
            Expr(
                ExprKind::Access(if let ExprKind::Access(base) = kind {
                    base + new
                } else {
                    Path::Expr(Box::new(Expr(kind, start))) + new
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

                    self.expr_end(ExprKind::Call(Box::new(func), args), start_span)
                } else if let Some(args) = self.args()? {
                    self.expr_end(ExprKind::Call(Box::new(res), args), start_span)
                } else {
                    break
                }
            }
        }

        Ok(Some(res))
    }

    fn var(&mut self) -> Result<Spanned<VarRes>, DukaSpannedError> {
        let start_span = self.current_span;
        let mut base = oneof!(if:
        case self.then(TokenKind::LParen)? => {
            let exp = must!(self.exp())?;
            self.must_token(TokenKind::RParen)?;

            let suffix = must!(self.var_suffix(), "'.', '[]' etc")?;
            let base = Path::Expr(Box::new(exp));
            base + suffix
        },
        else:
            let name = self.must_ident()?;
            Path::Base(name)
        );

        many! {
            loop:
            if let Some(suffix) = self.var_suffix()? {
                if let PathSuffix::Colon(_) = suffix {
                    let args = must!(self.args())?;

                    match self.var_func_suffix(base + suffix, args)? {
                        t @ VarRes::Call(_) => return Ok(
                            self.span_end(t, start_span)
                        ),
                        VarRes::Var(p) => base = p,
                    }
                } else {
                    base = base + suffix
                }
            }
            else if let Some(args) = self.args()? {
                match self.var_func_suffix(base, args)? {
                    t @ VarRes::Call(_) => return Ok(
                        self.span_end(t, start_span)
                    ),
                    VarRes::Var(p) => base = p,
                }
            } else {
                break
            }
        }

        let end_span = self.current_span;
        Ok(if let Some(args) = self.args()? {
            let callee = Expr(ExprKind::Access(base), start_span + end_span);
            (
                VarRes::Call(StmtKind::Call(callee, args)),
                start_span + end_span,
            )
        } else {
            (VarRes::Var(base), start_span + end_span)
        })
    }
    fn var_suffix(&mut self) -> TryDo<PathSuffix, DukaSpannedError> {
        Ok(Some(oneof! { if:
            case self.then(TokenKind::LBracket)? => {
                let exp = must!(self.exp())?;
                self.must_token(TokenKind::RBracket)?;

                PathSuffix::Index(Box::new(exp))
            },
            case self.then(TokenKind::Dot)? => {
                let name = self.must_ident()?;

                PathSuffix::Dot(name)
            },
            case self.then(TokenKind::Colon)? => {
                let name = self.must_ident()?;

                PathSuffix::Colon(name)
            }
            else: return Ok(None);
        }))
    }
    fn var_func_suffix(&mut self, base: Path, args: Vec<Expr>) -> Result<VarRes, DukaSpannedError> {
        let span = self.current_span;
        Ok(if let Some(suffix) = self.var_suffix()? {
            let call = ExprKind::Call(Box::new(self.expr_end(ExprKind::Access(base), span)), args);

            VarRes::Var(Path::Expr(Box::new(self.expr_end(call, span))) + suffix)
        } else {
            VarRes::Call(StmtKind::Call(
                self.expr_end(ExprKind::Access(base), span),
                args,
            ))
        })
    }

    fn func_name(&mut self) -> TryDo<Path, DukaSpannedError> {
        if let Some((base, start_span)) = self.expect_ident()? {
            let mut base = Path::Base((base, start_span));

            many! {
                self then Dot:
                let name = self.must_ident()?;
                base = base + PathSuffix::Dot(name);
            }

            opt![
                self then Colon:
                let last = self.must_ident()?;
                base = base + PathSuffix::Colon(last);
            ];

            Ok(Some(base))
        } else {
            Ok(None)
        }
    }

    fn func_body(&mut self) -> Result<FuncBody, DukaSpannedError> {
        let params = between!(self:
            must opt(self.par_list())[vec![]]
            in LParen, RParen
        );

        let body = oneof!(if:
            case self.then(TokenKind::Arrow)? => {
                let Expr(expr, span) = must!(self.exp())?;
                Block(vec![], Some(Box::new(
                    Stmt(StmtKind::Return(vec![Expr(expr, span)]), span)
                )))
            },
            else: self.block([TokenKind::End])?
        );

        Ok(FuncBody(params, body))
    }

    fn exp(&mut self) -> TryDo<Expr, DukaSpannedError> {
        self.exp_limit(0)
    }

    #[inline]
    fn exp_limit(&mut self, limit: u8) -> TryDo<Expr, DukaSpannedError> {
        let Expr(mut exp, start_span) = match self.atom_exp()? {
            Some(e) => e,
            None => return Ok(None),
        };

        Ok(Some(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_binop() {
                break self.expr_end(exp, start_span)
            }

            let Some((op, (l, r))) = get_binop_info(tk) else {
                return Err(
                    DukaSpannedError {
                        kind: DukaParserError::UnknownOperator(tk.name().to_owned()).into(),
                        span: self.current_span,
                    }
                )
            };

            if op.is_single() && l == limit {
                return Err(
                    DukaSpannedError {
                        kind: DukaParserError::InvalidOperator(tk.name().to_owned()).into(),
                        span: self.current_span,
                    }
                )
            }

            if l <= limit {
                break Expr(exp, start_span)
            }

            // consume op
            self.next_token()?;
            let Some(right) = self.exp_limit(r)? else {
                return Err(self.expected(cpar::EXP));
            };
            exp = ExprKind::Binary(Box::new(self.expr_end(exp, start_span)), Box::new(right), op)

        }))
    }

    fn atom_exp(&mut self) -> TryDo<Expr, DukaSpannedError> {
        oneof!(if let Some(res) = self.prefix_exp()? {
            Ok(Some(res))
        } else {
            let (tk, start_span) = self.span_start()?;
            let kind = oneof!(
                try match tk =>
                TokenKind::If => {
                    self.next_token()?;
                    ExprKind::If(self.if_block(true)?)
                }
                TokenKind::Match => {
                    self.next_token()?;
                    ExprKind::Match(self.match_block(true)?)
                }
                TokenKind::Do => {
                    let block = self.block([TokenKind::End])?;
                    ExprKind::Do(block)
                }
                TokenKind::Nil => {
                    self.next_token()?;
                    ExprKind::Literal(ConstValue::Nil)
                }
                TokenKind::True => {
                    self.next_token()?;
                    ExprKind::Literal(ConstValue::Bool(true))
                }
                TokenKind::False => {
                    self.next_token()?;
                    ExprKind::Literal(ConstValue::Bool(false))
                }
                TokenKind::Float(f) => {
                    let k = ExprKind::Literal(ConstValue::Float(*f));
                    self.next_token()?;
                    k
                }
                TokenKind::Int(i) => {
                    let k = ExprKind::Literal(ConstValue::Int(*i));
                    self.next_token()?;
                    k
                }
                TokenKind::String(..) => {
                    let TokenKind::String(v) = self.next_token()?.0 else {
                        unreachable!()
                    };
                    let k = ExprKind::Literal(
                        ConstValue::String(v)
                    );
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
                t if t.is_unop() => self.unop_exp()?
            );
            Ok(Some(self.expr_end(kind, start_span)))
        })
    }

    fn unop_exp(&mut self) -> Result<ExprKind, DukaSpannedError> {
        let tk = self.next_token()?.0;
        Ok(ExprKind::Unary(
            // ATTENTION! unary expression should be the max
            Box::new(must!(self.exp_limit(u8::MAX))?),
            match tk {
                TokenKind::Minus => UnOp::Minus,
                TokenKind::Not => UnOp::Not,
                TokenKind::Length => UnOp::Length,
                TokenKind::BitTilde => UnOp::BitNot,
                _ => unreachable!(),
            },
        ))
    }

    fn args(&mut self) -> TryDo<Vec<Expr>, DukaSpannedError> {
        let (tk, start_span) = self.span_start()?;
        Ok(Some(oneof!(
            try match tk =>
            TokenKind::LParen =>
                between!(self:
                    must opt(self.exp_list())[Some(vec![])]
                    in LParen, RParen
                ).unwrap_or_default(),
            TokenKind::LBrace => {
                let table = must!(self.table_constructor())?;
                vec![self.expr_end(table, start_span)]
            }
            TokenKind::String(..) => {
                let TokenKind::String(val) = self.next_token()?.0 else {
                    unreachable!()
                };
                let str = ExprKind::Literal(ConstValue::String(val));
                vec![self.expr_end(str, start_span)]
            }
        )))
    }

    /// already checked
    fn table_constructor(&mut self) -> TryDo<ExprKind, DukaSpannedError> {
        self.next_token()?; // already checked
        let mut fields = vec![];
        let mut is_const = true;

        if !self.then(TokenKind::RBrace)? {
            opt![if let Some(f) = self.field()? {
                is_const = f.is_const();
                fields.push(f);
            }];

            many! {
                self then Comma or SemiColon:
                // {...,}
                if self.lookahead_token(TokenKind::RBrace, 0)? {
                    break
                } else {
                    let f = must!(self.field())?;
                    if !f.is_const() {
                        is_const = false
                    }
                    fields.push(f)
                }
            }

            self.must_token(TokenKind::RBrace)?;
        }

        let table = if is_const {
            let mut table = ArrayMap::new();
            for field in fields {
                match field {
                    Field::KeyValue(
                        Expr(ExprKind::Literal(k), _),
                        Expr(ExprKind::Literal(v), _),
                    ) => {
                        table.map.insert(k, v);
                    }
                    Field::NameValue((k, _), Expr(ExprKind::Literal(v), _)) => {
                        table.map.insert(ConstValue::String(k.into_bytes()), v);
                    }
                    Field::Value(Expr(ExprKind::Literal(v), _)) => table.array.push(v),
                    _ => unreachable!(),
                }
            }

            ExprKind::Literal(ConstValue::ConstTable(Rc::new(RefCell::new(table))))
        } else {
            ExprKind::Table(fields)
        };
        Ok(Some(table))
    }

    fn field(&mut self) -> TryDo<Field, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::LBracket)? {
            let key = must!(self.exp())?;

            self.must_token(TokenKind::RBracket)?;
            self.must_token(TokenKind::Assign)?;

            let val = must!(self.exp())?;

            Some(Field::KeyValue(key, val))
        } else if self.lookahead_token(TokenKind::Assign, 1)? {
            let (key, start_span) = self.must_ident()?;
            self.must_token(TokenKind::Assign)?;

            let val = must!(self.exp())?;

            Some(Field::NameValue((key, start_span), val))
        } else {
            self.exp()?.map(Field::Value)
        }))
    }

    fn name_list(&mut self) -> Result<Vec<Name>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.must_ident())
            nonempty
        ))
    }

    fn attr_name_list(&mut self) -> Result<Vec<AttrName>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.attr_name())
            nonempty
        ))
    }

    fn exp_list(&mut self) -> TryDo<Vec<Expr>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.exp())
            empty[None]
        ))
    }

    fn par_list(&mut self) -> Result<Vec<Param>, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::Dots)? {
            vec![Param::Var(self.current_span)]
        } else {
            let mut res: Vec<Param> = self.name_list()?.into_iter().map(Param::Name).collect();

            opt![
                self then Dots:
                res.push(Param::Var(self.current_span))
            ];

            res
        }))
    }
}

/// external
impl<I: Iterator<Item = RawToken<Token>>> Parser<I> {
    fn logic_block(&mut self) -> Result<(), DukaSpannedError> {
        many! {
            loop:
            if self.lookahead_token(TokenKind::RBrace, 0)? {
                break
            }
            self.logic_clause()?;
        }
        Ok(())
    }
    fn logic_clause(&mut self) -> Result<(), DukaSpannedError> {
        oneof!(
            err match self.must_ident()?.0.as_str();
                self(DukaParserError::UnexpectedToken("fact, rule".to_owned()))
            =>
            "fact" => {
                let fact = self.logic_fact()?;
                self.logic.facts.push(fact);
            }
            "rule" => {
                let rule = self.logic_rule()?;
                self.logic.rules.push(rule);
            }
        );
        Ok(())
    }

    fn logic_fact(&mut self) -> Result<Fact, DukaSpannedError> {
        let name = self.must_ident()?.0;
        let terms = between!(self:
            must opt(self.logic_terms())[vec![]]
            in LParen, RParen
        );
        Ok(Fact(name, terms))
    }

    fn logic_term(&mut self) -> TryDo<Term, DukaSpannedError> {
        Ok(Some(oneof!(
            try match self.peek_token(0)?.0 =>
            TokenKind::Ident(_) => {
                let i = self.must_ident()?.0;
                oneof!(if:
                    case self.then(TokenKind::LParen)? => {
                        let terms = self.logic_terms()?;
                        let res = Term::Compound(i, terms);
                        self.must_token(TokenKind::RParen)?;
                        res
                    },
                    case i == "_" => {
                        Term::Anonymous
                    },
                    case i.chars()
                        .nth(0)
                        .is_some_and(|c| c.is_uppercase() || c == '_') => {
                        Term::Var(i)
                    }
                    else: Term::Atom(i)
                )
            }
            TokenKind::String(_) => {
                if let (TokenKind::String(vec), span) = self.next_token()? {
                    Term::Atom(
                        String::from_utf8(vec)
                            .map_err(|_| DukaSpannedError {
                                kind: DukaLexerError::InvalidUtf8.into(),
                                span
                            })?
                    )
                } else {
                    unreachable!()
                }
            }
            TokenKind::Int(i) => {
                self.next_token()?;
                Term::Number(i)
            }
        )))
    }

    fn logic_terms(&mut self) -> Result<Vec<Term>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.logic_term())
            empty[None]
        )
        .unwrap_or_default())
    }

    fn logic_rule(&mut self) -> Result<Rule, DukaSpannedError> {
        let name = self.must_ident()?.0;

        let terms = between!(self:
            must opt(self.logic_terms())[vec![]]
            in LParen, RParen
        );

        self.must_token(TokenKind::Assign)?;

        let goal = self.logic_goal(0)?;

        Ok(Rule(name, terms, goal))
    }

    // parent(A, B) :- father(A, B), mother(A, B).
    /// TODO: More goals
    fn logic_goal(&mut self, limit: u8) -> Result<Goal, DukaSpannedError> {
        let mut goals = vec![self.logic_goal_atom()?];
        let mut current_op = LogicOp::And;

        fn assemble(mut goals: Vec<Goal>, op: LogicOp) -> Goal {
            if goals.len() == 1 {
                goals.pop().unwrap()
            } else {
                match op {
                    LogicOp::And => Goal::And(goals),
                    LogicOp::Or => Goal::Or(goals),
                }
            }
        }

        Ok(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_logic_binop() {
                break assemble(goals, current_op)
            }

            if let Some((op, (l, r))) = get_logicop_info(tk)
            {
                if l <= limit {
                    break assemble(goals, current_op)
                }

                self.next_token()?;
                let right = self.logic_goal(r)?;

                if current_op != op {
                    goals = vec![assemble(goals, current_op), right];
                    current_op = op;
                }
                else {
                    goals.push(right);
                }
            } else {
                return Err(
                    DukaSpannedError {
                        kind: DukaParserError::UnknownOperator(tk.name().to_owned()).into(),
                        span: self.current_span,
                    }
                )
            }
        })
    }

    fn logic_goal_atom(&mut self) -> Result<Goal, DukaSpannedError> {
        Ok(oneof!(try match self.peek_token(0)?.0 => {
            TokenKind::Bang => {
                self.next_token()?;
                Goal::Cut
            }
            TokenKind::Not => {
                self.next_token()?;
                let inner = self.logic_goal(u8::MAX)?;
                Goal::Not(Box::new(inner))
            }
            TokenKind::LParen => {
                self.next_token()?;
                let goal = self.logic_goal(0)?;
                self.must_token(TokenKind::RParen)?;
                goal
            }
        }
        else:
            let term = must!(self.logic_term())?;
            Goal::Term(term)
        ))
    }
}

impl<I: Iterator<Item = RawToken<Token>>> Parser<I> {
    #[inline(always)]
    fn err(&self, kind: DukaParserError) -> DukaSpannedError {
        DukaSpannedError {
            kind: kind.into(),
            span: self.current_span,
        }
    }

    #[inline(always)]
    fn stmt_end(&self, val: StmtKind, start: Span) -> Stmt {
        Stmt(val, start + self.current_span)
    }
    #[inline(always)]
    fn expr_end(&self, val: ExprKind, start: Span) -> Expr {
        Expr(val, start + self.current_span)
    }
    #[inline(always)]
    fn span_end<V>(&self, val: V, start: Span) -> Spanned<V> {
        (val, start + self.current_span)
    }
}

impl<I: Iterator<Item = RawToken<Token>>> Parser<I> {
    #[inline(always)]
    fn span_start(&mut self) -> Result<RefToken<'_>, DukaSpannedError> {
        let (tk, sp) = self.peek_token(0)?;
        Ok((tk, *sp))
    }

    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError> {
        self.then_keyword(kw)?
            .then_some(())
            .ok_or(self.expected(kw))
    }
    fn then_keyword(&mut self, kw: &str) -> Result<bool, DukaSpannedError> {
        let TokenKind::Ident(ref id) = self.peek_token(0)?.0 else {
            return Ok(false);
        };
        if id == kw {
            self.next_token()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[inline(always)]
    fn then(&mut self, token: TokenKind) -> Result<bool, DukaSpannedError> {
        Ok(self.expect_token(token)?.is_some())
    }

    #[inline(always)]
    fn lookahead_token(&mut self, token: TokenKind, pos: usize) -> Result<bool, DukaSpannedError> {
        Ok(matches!(self.peek_token(pos)?, (tk, _) if *tk == token))
    }

    #[inline(always)]
    fn expect_ident(&mut self) -> TryDo<Spanned<String>, DukaSpannedError> {
        self.expect(|t| matches!(t, TokenKind::Ident(..))).map(|t| {
            if let Some((TokenKind::Ident(ident), span)) = t {
                Some((ident, span))
            } else {
                None
            }
        })
    }

    #[inline(always)]
    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaSpannedError> {
        self.expect(|tk| *tk == token)
    }

    #[inline(always)]
    fn expected(&mut self, msg: &str) -> DukaSpannedError {
        DukaSpannedError {
            kind: DukaParserError::UnexpectedToken(msg.to_string()).into(),
            // same, im sure this wont be a panic when i call it
            span: self.peek_token(0).expect("im sure this wont happen").1,
        }
    }

    #[inline]
    fn expect<T: FnOnce(&TokenKind) -> bool>(
        &mut self,
        predicate: T,
    ) -> TryDo<Token, DukaSpannedError> {
        Ok(match self.peek_token(0)? {
            (tk, _) if predicate(tk) => Some(self.next_token()?),
            _ => None,
        })
    }

    #[inline(always)]
    fn must_ident(&mut self) -> Result<Spanned<String>, DukaSpannedError> {
        match self.peek_token(0)? {
            (TokenKind::Ident(..), _) => {
                if let (TokenKind::Ident(ident), span) = self.next_token()? {
                    Ok((ident, span))
                } else {
                    unreachable!()
                }
            }
            (tk, span) => Err(DukaSpannedError {
                kind: DukaParserError::UnexpectedToken(if tk.is_keyword() {
                    format!("{}, found keyword {}", clex::ID, tk.name())
                } else {
                    clex::ID.to_owned()
                })
                .into(),
                span: *span,
            }),
        }
    }

    #[inline(always)]
    fn must_token(&mut self, token: TokenKind) -> Result<Token, DukaSpannedError> {
        self.must(|t| *t == token, token.name())
    }

    #[inline]
    fn must<T: FnOnce(&TokenKind) -> bool>(
        &mut self,
        predicate: T,
        msg: &str,
    ) -> Result<Token, DukaSpannedError> {
        self.expect(predicate)?.ok_or(self.expected(msg))
    }

    #[inline]
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError> {
        const MAX_DEPTH: usize = 3;
        assert!(n <= MAX_DEPTH);
        // if n > MAX_DEPTH {
        //     // NOTICE, This won't happen if using appropriately
        //     panic!("Do not use too many peek")
        // }

        // ENSURED, unwrap here不会有问题的 对吧
        self.tokens
            .peek_nth(n)
            .map(|r| r.as_ref())
            .transpose()
            .map(|o| o.unwrap_or(&EMPTY_TOKEN))
            .map_err(|e| e.clone())

        // while self.lookahead.len() <= n {
        //     match self.tokens.next_token() {
        //         Err(e) => return Err(e),
        //         Ok(t) if t.0.is_terminator() => break,
        //         item => self.lookahead.push_back(item),
        //     }
        // }
        // self.lookahead
        //     .get(n)
        //     .map(|r| r.as_ref())
        //     .transpose()
        //     .map(|o| o.unwrap_or(&EMPTY_TOKEN))
        //     .map_err(|e| e.clone())
    }

    #[inline]
    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        self.tokens.next().unwrap_or(Ok(EMPTY_TOKEN.clone()))
    }
}

impl<I: Iterator<Item = RawToken<Token>>> DukaParser<I> for Parser<I> {
    type ChunkType = DukaChunk;

    fn parse(stream: I) -> Result<Self::ChunkType, DukaSpannedError> {
        let mut parser = Self::new(stream);
        let start_span = parser.current_span;
        let chunk = parser.parse_chunk()?;
        Ok(DukaChunk {
            chunk,
            span: start_span + parser.current_span,
            logic: parser.logic,
        })
    }
}
