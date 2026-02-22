use std::sync::Arc;

use ast::{
    AttrName, Attrs, Block, DukaChunk, Expr, ExprKind, Field, FieldPattern, FuncBody, If, IfClause,
    Linq, LinqClause, Match, MatchClause, Name, ObjectDef, ObjectProperty, Param, Path, PathSuffix,
    PatternArrayTerm, PatternTerm, Stmt, StmtKind, get_binop_info, get_logicop_info,
    get_patop_info,
};
use duka_shared::{
    constants::{clex, cpar, ctype},
    errors::{DukaLexerError, DukaParserError, DukaSpannedError, Span},
    types::{
        DukaParser, Fact, Goal, LogicDatabase, LogicOp, Query, QueryCount, Rule, SourceInfo,
        Spanned, SysCall, Term, TokenStream, UnOp,
    },
    utils::{MultiPeekable, MultiPeekableExtension, OrError, TryDo},
    value::{ArrayMap, ConstValue, DukaInt},
};

use crate::{
    lexer::token::{EMPTY_TOKEN, Token, TokenKind},
    parser::bang::{BangExprHandler, BangHandlers, BangStmtHandler, ParserAPI},
};

pub mod ast;
pub mod bang;

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
    (err match $target: expr; $self: ident($v:ident -> $e: expr) => $($input:tt)*) => {
        {let $v = $target;
        match $v {
            $($input)*,
            _ => {
                return Err(
                    $self.err($e)
                )
            }
        }
    }};
    ($($input:tt)*) => {
        {$($input)*}
    };
}
/// ## Marker ()
/// must be exactly
macro_rules! must {
    ($e: expr, $self: ident, $expected: expr) => {
        $e?.ok_or($self.expected(cpar::SRY, $expected))
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

        res.into()
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
pub struct Parser<T> {
    tokens: MultiPeekable<std::vec::IntoIter<T>>,
    source_info: SourceInfo,
    current_span: Span,
    handlers: BangHandlers,
    logic: LogicDatabase,
}

#[derive(Debug)]
enum VarDesc {
    Call(Box<StmtKind>),
    Var(Path),
}

/// main duka
impl Parser<Token> {
    pub fn new(stream: TokenStream<Token>) -> Self {
        Self {
            tokens: stream.tokens.into_iter().multi_peekable(),
            current_span: Span::default(),
            handlers: BangHandlers::default(),
            logic: LogicDatabase::default(),
            source_info: stream.source_info,
        }
    }

    #[must_use]
    pub fn register_bang_expr(
        mut self,
        keyword: impl Into<String>,
        handler: Arc<dyn BangExprHandler>,
    ) -> Self {
        self.handlers.register_expr(keyword, handler);
        self
    }
    #[must_use]
    pub fn register_bang_stmt(
        mut self,
        keyword: impl Into<String>,
        handler: Arc<dyn BangStmtHandler>,
    ) -> Self {
        self.handlers.register_stmt(keyword, handler);
        self
    }

    /// Try parse the input as expression at first, if failed, then try parse it as statement.
    /// The boolean in result's tuple indicates whether it is an expression or not
    pub fn parse_expr_or_stmt(&mut self) -> Result<(Block, bool), DukaSpannedError> {
        if self.peek_token(0)?.0.is_terminator() {
            return Ok((Block::empty(), false));
        }
        if let Some(expr) = self.expr_inner(false)? {
            let span = expr.1;
            Ok((
                Block(
                    [].into(),
                    Some(Box::new(Stmt(StmtKind::Return([expr].into()), span))),
                ),
                true,
            ))
        } else {
            let stmt = must!(self.stmt())?;
            Ok((Block([stmt].into(), None), false))
        }
    }

    /// Parse all the input as a single chunk, return the block containing all of statments
    pub fn parse_chunk(&mut self) -> Result<Block, DukaSpannedError> {
        self.chunk()
    }

    fn chunk(&mut self) -> Result<Block, DukaSpannedError> {
        self.block([TokenKind::terminator()])
    }

    #[inline(always)]
    fn block<const C: usize>(
        &mut self,
        end_withs: [TokenKind; C],
    ) -> Result<Block, DukaSpannedError> {
        self.block_inner(end_withs, [])
    }

    fn block_inner<const C: usize, const R: usize>(
        &mut self,
        consumed: [TokenKind; C],
        retains: [TokenKind; R],
    ) -> Result<Block, DukaSpannedError> {
        let mut stmts = vec![];

        Ok(many! {
            loop:
            if self.expect(|t| consumed.contains(t))?.is_some()
                || retains.contains(&self.peek_token(0)?.0) {
                break Block(stmts.into(), None)
            }

            if self.then(TokenKind::Return)? {
                let ret = self.ret_stmt()?;

                if !retains.contains(&self.peek_token(0)?.0) {
                    self.must(|t| consumed.contains(t), consumed[0].name())?;
                }

                break Block(stmts.into(), Some(Box::new(ret)))
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
                let expr = must!(self.expr())?;
                self.must_token(TokenKind::RParen)?;
                StmtKind::Expr(Box::new(expr))
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

                let cond = must!(self.expr())?;
                self.must_token(TokenKind::Do)?;
                let body = self.block([TokenKind::End])?;

                StmtKind::While(Box::new(cond), Box::new(body))
            }
            TokenKind::Do => {
                self.next_token()?;
                StmtKind::Do(Box::new(self.block([TokenKind::End])?))
            }
            TokenKind::Object => {
                self.next_token()?;
                StmtKind::Object(Box::new(self.object()?))
            }
        );
        Ok(Some(self.stmt_end(kind, start_span)))
    }

    fn match_block(&mut self, must_else: bool) -> Result<Match, DukaSpannedError> {
        let target = must!(self.expr())?;

        self.must_token(TokenKind::Then)?;

        let mut clauses = vec![];
        many! {
            loop:

            if self.then(TokenKind::End)? {
                must_else.then_error(||
                    self.err(
                        DukaParserError::UnexpectedToken {
                            got: TokenKind::End.name().into(),
                            expected: TokenKind::Else.name().into()
                        }
                    )
                )?;
                return Ok(Match(Box::new(target), clauses.into(), None))
            }
            if self.then(TokenKind::Else)? {
                break
            }

            let clause = self.match_clause()?;
            clauses.push(clause);
        }

        let else_clause = self.block([TokenKind::End])?;

        Ok(Match(
            Box::new(target),
            clauses.into(),
            Some(Box::new(else_clause)),
        ))
    }

    fn match_clause(&mut self) -> Result<MatchClause, DukaSpannedError> {
        let pattern = self.match_pattern(0)?;

        let guard = opt![
            self then If: {
                let expr = must!(self.expr())?;
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
                let Expr(expr, span) = must!(self.expr())?;
                self.must_token(TokenKind::SemiColon)?;
                let stmt = StmtKind::Return(Box::new([Expr(expr, span)]));
                Block(Box::new([]), Some(Box::new(Stmt(stmt, span))))
        );

        Ok(MatchClause((pattern, guard.map(Box::new)), Box::new(block)))
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
                    DukaSpannedError::new( DukaParserError::UnknownOperator(tk.name().into()).into(),  self.current_span,  self.source_info.clone())
                )
            }
        })
    }

    fn match_atom_pattern(&mut self) -> Result<PatternTerm, DukaSpannedError> {
        Ok(oneof!(
            try match self.peek_token(0)?.0 => {
                TokenKind::Pipeline => {
                    self.next_token()?;
                    let func = must!(self.atom_exp(true))?;
                    PatternTerm::Call(Box::new(func))
                },
                TokenKind::LParen => between!(self:
                    must nonempty(self.match_pattern(0))
                    in LParen, RParen
                ),
                TokenKind::LBrace => between!(self:
                    must opt(self.match_atom_table_pattern())[PatternTerm::Table(Box::new([]))]
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
                        return Err(DukaSpannedError::new( DukaParserError::UnknownOperator(tk.0.name().into()).into(),  self.current_span,  self.source_info.clone()))
                    };
                    let right = must!(self.atom_exp(true))?;
                    PatternTerm::Compare(op, Box::new(right))
                }
            } else:
                let expr = must!(self.atom_exp(true))?;
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

        Ok(PatternTerm::Table(fields.into()))
    }

    fn match_field_pattern(&mut self) -> Result<FieldPattern, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::LBracket)? {
            let key = must!(self.expr())?;

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
                .expect(|t| matches!(t, TokenKind::Ident(id) if id == cpar::DISCARD))?
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

    fn object(&mut self) -> Result<ObjectDef, DukaSpannedError> {
        let name = self.must_ident()?;

        let base = opt![
            self then Colon:
            { Some(self.must_ident()?) }
            else: None
        ];

        let mut static_methods = vec![];
        let mut methods = vec![];
        let mut properties = vec![];

        many! {
            loop:
            oneof! {if:
                case self.then(TokenKind::Function)? => {
                    let is_static = !self.then(TokenKind::Colon)?;
                    let attrs = self.attrs()?;
                    let name = self.must_ident()?;
                    let body = self.func_body()?;
                    let func = (name, attrs, body);

                    if is_static {
                        static_methods.push(func);
                    } else {
                        methods.push(func);
                    }
                },
                case self.then(TokenKind::LBracket)? => {
                    let key = must!(self.expr())?;
                    let val = opt![
                        self then Equal: {
                            Some(must!(self.expr())?)
                        }
                        else: None
                    ];
                    properties.push(ObjectProperty::KeyValue(Box::new(key), val.map(Box::new)))
                },
                case self.expect_ident()?.is_some() => {
                    let key = self.must_ident()?;
                    let val = opt![
                        self then Equal: {
                            Some(must!(self.expr())?)
                        }
                        else: None
                    ];
                    properties.push(ObjectProperty::NameValue(key, val.map(Box::new)));
                }
                else: break
            }
        }

        self.must_token(TokenKind::End)?;

        Ok(ObjectDef {
            name,
            base,
            properties: properties.into(),
            static_methods: static_methods.into(),
            methods: methods.into(),
        })
    }

    fn function(&mut self, global: bool) -> Result<StmtKind, DukaSpannedError> {
        let attrs = self.attrs()?;
        let name = must!(self.func_name())?;
        let body = self.func_body()?;
        Ok(StmtKind::Function(name, attrs, Box::new(body), global))
    }

    fn attr_var(&mut self, global: bool) -> Result<StmtKind, DukaSpannedError> {
        let vars: Vec<AttrName> = self.attr_name_list()?;

        Ok(StmtKind::Define(
            vars.into(),
            opt![
                self then Assign: {
                    let mut vals = vec![must!(self.expr())?];

                    many! {
                        self then Comma:
                        vals.push(must!(self.expr())?)
                    }

                    vals.into()
                }
                else: Box::new([])
            ],
            global,
        ))
    }

    fn ret_stmt(&mut self) -> Result<Stmt, DukaSpannedError> {
        let start_span = self.current_span;

        let exps = if self.then(TokenKind::SemiColon)? {
            vec![]
        } else {
            let result = opt![self.expr_list()]?.unwrap_or_default();
            opt![self then SemiColon];
            result
        };

        Ok(self.stmt_end(StmtKind::Return(exps.into()), start_span))
    }

    /// along with stmt(), expr()
    fn if_block(&mut self, must_else: bool) -> Result<If, DukaSpannedError> {
        let cond = must!(self.expr())?;
        self.must_token(TokenKind::Then)?;

        let body = self.block_inner([], [TokenKind::End, TokenKind::Else, TokenKind::Elseif])?;

        let mut else_if_arms = vec![];
        many! {
            self then Elseif:
            let cond = must!(self.expr())?;
            self.must_token(TokenKind::Then)?;
            let body = self.block_inner([], [TokenKind::End, TokenKind::Else, TokenKind::Elseif])?;

            else_if_arms.push(IfClause(Box::new(body), Box::new(cond)));
        }

        Ok(If(
            IfClause(Box::new(body), Box::new(cond)),
            else_if_arms.into(),
            opt![self then Else: {
                let else_body = self.block([TokenKind::End])?;
                Some(Box::new(else_body))
            } else:
                if must_else {
                    let tk = self.next_token()?;
                    let got = tk.0.stringify();
                    return Err(self.err(
                        DukaParserError::UnexpectedToken {
                            got: got.into(),
                            expected: TokenKind::Else.name().into()
                    }));
                };
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
            let init = must!(self.expr())?;

            self.must_token(TokenKind::Comma)?;
            let cond = must!(self.expr())?;

            let step = opt![self then Comma: {
                Some(must!(self.expr())?)
            }
            else: None];

            self.must_token(TokenKind::Do)?;
            let body = self.block([TokenKind::End])?;

            StmtKind::ForNumberic(var, Box::new(init), Box::new(cond), step.map(Box::new), Box::new(body))
        },
        else:
            let vars = self
                .name_list()?
                .into_iter()
                .map(Path::Base)
                .collect();

            self.must_token(TokenKind::In)?;

            let exps = must!(self.expr_list())?;

            self.must_token(TokenKind::Do)?;
            let body = self.block([TokenKind::End])?;

            StmtKind::ForGeneric(vars, exps.into(), Box::new(body))
        ))
    }

    fn bang_stmt(&mut self, name: Name) -> Result<StmtKind, DukaSpannedError> {
        self.must_token(TokenKind::LBrace)?;
        let res = match name.0.as_str() {
            "logic" => {
                self.logic_block()?;
                StmtKind::Extern
            }
            name => {
                let handler = self
                    .handlers
                    .get_stmt(name)
                    .ok_or_else(|| self.err(DukaParserError::UnknownBang(name.into())))?;
                let mut wrapper = ParserWrapper { inner: self };
                handler.handle(&mut wrapper)?
            }
        };
        self.must_token(TokenKind::RBrace)?;
        Ok(res)
    }
    fn bang_expr(&mut self, name: Name) -> Result<ExprKind, DukaSpannedError> {
        self.must_token(TokenKind::LParen)?;
        let res = match name.0.as_str() {
            "logic" => ExprKind::SysCall(self.logic_query()?),
            "linq" => ExprKind::Linq(self.linq_expr()?),
            name => {
                let handler = self
                    .handlers
                    .get_expr(name)
                    .ok_or_else(|| self.err(DukaParserError::UnknownBang(name.into())))?;
                let mut wrapper = ParserWrapper { inner: self };
                handler.handle(&mut wrapper)?
            }
        };
        self.must_token(TokenKind::RParen)?;
        Ok(res)
    }

    fn linq_expr(&mut self) -> Result<Linq, DukaSpannedError> {
        self.must_keyword("from")?;
        let name = must!(self.simple_name())?;
        self.then(TokenKind::In)?;
        let expr = must!(self.expr())?;

        let from = LinqClause::From(name, Box::new(expr));
        let mut clauses = vec![from];

        many! {
            loop:
            let Some(clause) = self.linq_clause()? else { break };
            clauses.push(clause)
        }

        self.must_keyword("select")?;
        let select = must!(self.expr())?;

        Ok(Linq(clauses.into(), Box::new(select)))
    }
    fn linq_clause(&mut self) -> TryDo<LinqClause, DukaSpannedError> {
        Ok(Some(oneof!(if:
            case self.then_keyword("from")? => {
                let name = must!(self.simple_name())?;
                self.then(TokenKind::In)?;
                let expr = must!(self.expr())?;
                LinqClause::From(name, Box::new(expr))
            },
            case self.then_keyword("where")? => {
                let expr = must!(self.expr())?;
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
                VarDesc::Call(call) => *call,
                VarDesc::Var(name) => {
                    if let Some(op) = self.expect(TokenKind::is_binop)?
                        && let Some((binop, _)) = get_binop_info(&op.0)
                        && !binop.is_compare()
                    {
                        self.must_token(TokenKind::Assign)?;
                        let Expr(right, expr_span) = must!(self.expr())?;

                        StmtKind::Assign(
                            [name.clone()].into(),
                            [Expr(
                                ExprKind::Binary(
                                    Box::new(Expr(ExprKind::Access(name.into()), span)),
                                    Box::new(Expr(right, expr_span)),
                                    binop,
                                ),
                                span + expr_span,
                            )]
                            .into(),
                        )
                    } else {
                        let mut vars = vec![name];
                        many! {
                            self then Comma:
                            vars.push(match self.var()?.0 {
                                VarDesc::Var(var) => var,
                                VarDesc::Call(..) => return Err(
                                    self.expected(cpar::CAL, cpar::VAR)
                                ),
                            });
                        }

                        self.must_token(TokenKind::Assign)?;

                        let exps = must!(self.expr_list())?;

                        StmtKind::Assign(vars.into(), exps.into())
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
            try[Box::new([])] nonempty(self.attr_list())
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
                let expr = must!(self.expr())?;
                self.must_token(TokenKind::RParen)?;
                expr
            }
            TokenKind::Ident(..) => {
                let name = self.must_ident()?;

                if self.then(TokenKind::Bang)? {
                    let bang = self.bang_expr(name)?;
                    return Ok(Some(self.expr_end(bang, start_span)))
                }

                self.expr_end(ExprKind::Access(Box::new(Path::Base(name))), start_span)
            }
        );

        fn chain(former: Expr, new: PathSuffix, end: Span) -> Expr {
            let Expr(kind, start) = former;
            Expr(
                ExprKind::Access(Box::new(if let ExprKind::Access(base) = kind {
                    *base + new
                } else {
                    Path::Expr(Box::new(Expr(kind, start))) + new
                })),
                start + end,
            )
        }

        many! {
            loop:
            res = oneof! {
                if self.then(TokenKind::LBracket)? {
                    let expr = must!(self.expr())?;
                    self.must_token(TokenKind::RBracket)?;

                    chain(res, PathSuffix::Index(Box::new(expr)), self.current_span)
                } else if self.then(TokenKind::Dot)? {
                    let name = self.must_ident()?;

                    chain(res, PathSuffix::Dot(name), self.current_span)
                } else if self.then(TokenKind::Colon)? {
                    let name = self.must_ident()?;
                    let args = must!(self.args())?;
                    let func = chain(res, PathSuffix::Colon(name), self.current_span);

                    self.expr_end(ExprKind::Call(Box::new(func), args.into()), start_span)
                } else if let Some(args) = self.args()? {
                    self.expr_end(ExprKind::Call(Box::new(res), args.into()), start_span)
                } else {
                    break
                }
            }
        }

        Ok(Some(res))
    }

    fn var(&mut self) -> Result<Spanned<VarDesc>, DukaSpannedError> {
        let start_span = self.current_span;
        let mut base = oneof!(if:
        case self.then(TokenKind::LParen)? => {
            let expr = must!(self.expr())?;
            self.must_token(TokenKind::RParen)?;

            let suffix = must!(self.var_suffix(), "'.', '[]' etc")?;
            let base = Path::Expr(Box::new(expr));
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
                        t @ VarDesc::Call(_) => return Ok(
                            self.span_end(t, start_span)
                        ),
                        VarDesc::Var(p) => base = p,
                    }
                } else {
                    base = base + suffix
                }
            }
            else if let Some(args) = self.args()? {
                match self.var_func_suffix(base, args)? {
                    t @ VarDesc::Call(_) => return Ok(
                        self.span_end(t, start_span)
                    ),
                    VarDesc::Var(p) => base = p,
                }
            } else {
                break
            }
        }

        let end_span = self.current_span;
        Ok((
            if let Some(args) = self.args()? {
                let callee = Expr(ExprKind::Access(Box::new(base)), start_span + end_span);
                VarDesc::Call(Box::new(StmtKind::Call(Box::new(callee), args.into())))
            } else {
                VarDesc::Var(base)
            },
            start_span + end_span,
        ))
    }
    fn var_suffix(&mut self) -> TryDo<PathSuffix, DukaSpannedError> {
        Ok(Some(oneof! { if:
            case self.then(TokenKind::LBracket)? => {
                let expr = must!(self.expr())?;
                self.must_token(TokenKind::RBracket)?;

                PathSuffix::Index(Box::new(expr))
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
    fn var_func_suffix(
        &mut self,
        base: Path,
        args: Vec<Expr>,
    ) -> Result<VarDesc, DukaSpannedError> {
        let span = self.current_span;
        Ok(if let Some(suffix) = self.var_suffix()? {
            let call = ExprKind::Call(
                Box::new(self.expr_end(ExprKind::Access(Box::new(base)), span)),
                args.into(),
            );

            VarDesc::Var(Path::Expr(Box::new(self.expr_end(call, span))) + suffix)
        } else {
            VarDesc::Call(Box::new(StmtKind::Call(
                Box::new(self.expr_end(ExprKind::Access(Box::new(base)), span)),
                args.into(),
            )))
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
                let Expr(expr, span) = must!(self.expr())?;
                Block(Box::new([]), Some(Box::new(
                    Stmt(StmtKind::Return([Expr(expr, span)].into()), span)
                )))
            },
            else: self.block([TokenKind::End])?
        );

        Ok(FuncBody(params.into(), Box::new(body)))
    }

    #[inline]
    fn expr(&mut self) -> TryDo<Expr, DukaSpannedError> {
        self.expr_inner(true)
    }

    #[inline]
    fn expr_inner(&mut self, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        self.expr_limit(0, use_expr_stmt)
    }

    #[inline]
    fn expr_limit(&mut self, limit: u8, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        let Expr(mut expr, start_span) = match self.atom_exp(use_expr_stmt)? {
            Some(e) => e,
            None => return Ok(None),
        };

        Ok(Some(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_binop() {
                break self.expr_end(expr, start_span)
            }

            let Some((op, (l, r))) = get_binop_info(tk) else {
                return Err(
                    DukaSpannedError::new( DukaParserError::UnknownOperator(tk.name().into()).into(),  self.current_span,  self.source_info.clone())
                )
            };

            if op.is_single() && l == limit {
                return Err(
                    DukaSpannedError::new( DukaParserError::InvalidOperator(tk.name().into()).into(),  self.current_span,  self.source_info.clone())
                )
            }

            if l <= limit {
                break Expr(expr, start_span)
            }

            // consume op
            self.next_token()?;
            let Some(right) = self.expr_limit(r, use_expr_stmt)? else {
                return Err(self.expected(cpar::SRY, cpar::EXP));
            };
            expr = ExprKind::Binary(Box::new(self.expr_end(expr, start_span)), Box::new(right), op)

        }))
    }

    fn atom_exp(&mut self, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        oneof!(if let Some(res) = self.prefix_exp()? {
            Ok(Some(res))
        } else {
            let (tk, start_span) = self.span_start()?;
            let kind = oneof!(
                try match tk =>
                TokenKind::If if use_expr_stmt => {
                    self.next_token()?;
                    ExprKind::If(self.if_block(true)?.into())
                }
                TokenKind::Match if use_expr_stmt => {
                    self.next_token()?;
                    ExprKind::Match(self.match_block(true)?)
                }
                TokenKind::Do if use_expr_stmt => {
                    self.next_token()?;
                    let block = self.block([TokenKind::End])?;
                    ExprKind::Do(block.into())
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

                    ExprKind::Literal(
                        ConstValue::String(v.into())
                    )
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
            Box::new(must!(self.expr_limit(u8::MAX, true))?),
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
                    must opt(self.expr_list())[Some(vec![])]
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
                let str = ExprKind::Literal(ConstValue::String(val.into()));
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
                    if !f.is_const() && is_const {
                        is_const = false
                    }
                    fields.push(f)
                }
            }

            self.must_token(TokenKind::RBrace)?;
        }

        let table = if is_const {
            let mut table = ArrayMap::new();
            let mut counter = 0;
            for field in fields {
                match field {
                    Field::KeyValue(
                        Expr(ExprKind::Literal(k), _),
                        Expr(ExprKind::Literal(v), _),
                    ) => {
                        table.inner.insert(k, v);
                    }
                    Field::NameValue((k, _), Expr(ExprKind::Literal(v), _)) => {
                        table
                            .inner
                            .insert(ConstValue::String(k.as_bytes().into()), v);
                    }
                    Field::Value(Expr(ExprKind::Literal(v), _)) => {
                        table.inner.insert(ConstValue::Int(counter as DukaInt), v);
                        counter += 1;
                    }
                    _ => unreachable!(),
                }
            }

            ExprKind::Literal(ConstValue::ConstTable(Box::new(table)))
        } else {
            ExprKind::Table(fields.into())
        };
        Ok(Some(table))
    }

    fn field(&mut self) -> TryDo<Field, DukaSpannedError> {
        Ok(oneof! {if:
            case self.then(TokenKind::LBracket)? => {
                let key = must!(self.expr())?;

                self.must_token(TokenKind::RBracket)?;
                self.must_token(TokenKind::Assign)?;

                let val = must!(self.expr())?;

                Some(Field::KeyValue(key, val))
            },
            case self.lookahead_token(TokenKind::Assign, 1)? => {
                let (key, start_span) = self.must_ident()?;
                self.must_token(TokenKind::Assign)?;

                let val = must!(self.expr())?;

                Some(Field::NameValue((key, start_span), val))
            }
            else: {
                self.expr()?.map(Field::Value)
            }
        })
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

    fn expr_list(&mut self) -> TryDo<Vec<Expr>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.expr())
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
impl Parser<Token> {
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
        let ident = self.must_ident()?;
        oneof!(
            err match ident.0.as_str();
                self(got -> DukaParserError::UnexpectedToken { got: got.into(), expected: "fact, rule".into()})
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
                        String::from_utf8(vec.to_vec())
                            .map_err(|_| DukaSpannedError::new(
                                 DukaLexerError::InvalidUtf8.into(),
                                span,
                                 self.source_info.clone()
                            ))?
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

            let (op, (l, r)) = get_logicop_info(tk).ok_or(
                DukaSpannedError::new( DukaParserError::UnknownOperator(tk.name().into()).into(),  self.current_span,  self.source_info.clone())
            )?;
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
            TokenKind::If => {
                self.next_token()?;
                let cond = self.logic_goal(0)?;
                self.must_token(TokenKind::Then)?;
                let then_goal = self.logic_goal(0)?;
                let else_goal = opt![
                    self then Else: {
                        Some(self.logic_goal(0)?)
                    }
                    else: None
                ];
                Goal::If(Box::new(cond), Box::new(then_goal), else_goal.map(Box::new))
            }
            TokenKind::Ident(_) => {
                let ident = self.must_ident()?;
                oneof! {
                    if self.then(TokenKind::LParen)? {
                        // Meta predicate call like is_list(X)
                        let args = self.logic_terms()?;
                        self.must_token(TokenKind::RParen)?;
                        Goal::Meta(ident.0, args)
                    } else if self.then(TokenKind::Assign)? {
                        // Unify: X = Y
                        let right = must!(self.logic_term())?;
                        Goal::Unify(Term::Atom(ident.0), right)
                    } else {
                        // Regular term
                        Goal::Term(Term::Atom(ident.0))
                    }
                }
            }
        }
        else:
            let term = must!(self.logic_term())?;
            // Check for comparison after getting the term
            if let Some(op) = self.expect(TokenKind::is_compare)? {
                let (binop, _) = get_binop_info(&op.0)
                    .filter(|o| o.0.is_compare())
                    .ok_or(
                        DukaSpannedError::new( DukaParserError::UnknownOperator(op.0.name().into()).into(),  self.current_span,  self.source_info.clone())
                )?;


                let right = must!(self.logic_term())?;
                Goal::Compare(term, right, binop.name().into())
            } else {
                Goal::Term(term)
            }
        ))
    }
    fn logic_binding(&mut self) -> TryDo<Spanned<String>, DukaSpannedError> {
        Ok(opt![
            self then At: {
                Some(self.must_ident()?)
            }
            else: None
        ])
    }
    fn logic_query(&mut self) -> Result<SysCall, DukaSpannedError> {
        let count = opt![
            self then For: {
                self.must_token(TokenKind::LBrace)?;
                let res = oneof! {
                    if let Some(bind) = self.logic_binding()? {
                        QueryCount::Binding(bind.0)
                    }
                    else if self.then_keyword(
                        "all"
                    )? {
                       QueryCount::All
                    } else {
                        let (TokenKind::Int(n), _) = self.must(|p| matches!(p, TokenKind::Int(..)), ctype::INT)? else { unreachable!() };
                        QueryCount::Exact(n as usize)
                    }
                };
                self.must_token(TokenKind::RBrace)?;
                res
            }
            else: QueryCount::Exact(1)
        ];
        let query = Query(self.logic_goal(0)?);
        let idx = self.logic.queries.push(query);
        Ok(SysCall::Query(idx, count))
    }
}

impl<T> Parser<T> {
    #[inline(always)]
    fn err(&self, kind: DukaParserError) -> DukaSpannedError {
        DukaSpannedError::new(kind.into(), self.current_span, self.source_info.clone())
    }

    #[inline(always)]
    fn span_end<V>(&self, val: V, start: Span) -> Spanned<V> {
        (val, start + self.current_span)
    }
}

impl Parser<Token> {
    #[inline(always)]
    fn stmt_end(&self, val: StmtKind, start: Span) -> Stmt {
        Stmt(val, start + self.current_span)
    }
    #[inline(always)]
    fn expr_end(&self, val: ExprKind, start: Span) -> Expr {
        Expr(val, start + self.current_span)
    }
    #[inline(always)]
    fn span_start(&mut self) -> Result<RefToken<'_>, DukaSpannedError> {
        let (tk, sp) = self.peek_token(0)?;
        Ok((tk, *sp))
    }

    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError> {
        self.then_keyword(kw)?
            .then_some(())
            .ok_or(self.expected(cpar::SRY, kw))
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
    fn expected(&mut self, got: &str, expected: &str) -> DukaSpannedError {
        DukaSpannedError::new(
            DukaParserError::UnexpectedToken {
                got: got.into(),
                expected: expected.into(),
            }
            .into(),
            // same, im sure this wont be a panic when i call it
            match self.peek_token(0).unwrap() {
                (tk, _) if tk.is_terminator() => self.current_span,
                (_, span) => *span,
            },
            self.source_info.clone(),
        )
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
            (tk, span) => Err(DukaSpannedError::new(
                DukaParserError::UnexpectedToken {
                    got: tk.stringify().into(),
                    expected: clex::ID.into(),
                }
                .into(),
                *span,
                self.source_info.clone(),
            )),
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
        self.expect(predicate)?.ok_or(self.expected(cpar::SRY, msg))
    }

    #[inline]
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError> {
        Ok(self.tokens.peek_nth(n).unwrap_or(&EMPTY_TOKEN))
    }

    #[inline]
    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        Ok(self
            .tokens
            .next()
            .inspect(|t| {
                self.current_span = t.1;
            })
            .unwrap_or((TokenKind::EOF, self.current_span)))
    }
}

impl DukaParser<Token> for Parser<Token> {
    type ChunkType = DukaChunk;

    fn parse(stream: TokenStream<Token>) -> Result<Self::ChunkType, DukaSpannedError> {
        let mut parser = Self::new(stream);
        let start_span = parser.current_span;
        let chunk = parser.parse_chunk()?;
        Ok(DukaChunk {
            chunk,
            span: start_span + parser.current_span,
            logic: Box::new(parser.logic),
            source_info: parser.source_info,
        })
    }
}

/// WRAPPER FOR PARSER API
struct ParserWrapper<'a> {
    inner: &'a mut Parser<Token>,
}

impl<'a> ParserAPI for ParserWrapper<'a> {
    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError> {
        self.inner.must_keyword(kw)
    }

    fn then_keyword(&mut self, kw: &str) -> Result<bool, DukaSpannedError> {
        self.inner.then_keyword(kw)
    }

    fn then(&mut self, token: TokenKind) -> Result<bool, DukaSpannedError> {
        self.inner.then(token)
    }

    fn lookahead_token(&mut self, token: TokenKind, pos: usize) -> Result<bool, DukaSpannedError> {
        self.inner.lookahead_token(token, pos)
    }

    fn expect_ident(&mut self) -> TryDo<Spanned<String>, DukaSpannedError> {
        self.inner.expect_ident()
    }

    fn expect_token(&mut self, token: TokenKind) -> TryDo<Token, DukaSpannedError> {
        self.inner.expect_token(token)
    }

    fn expected(&mut self, got: &str, expected: &str) -> DukaSpannedError {
        self.inner.expected(got, expected)
    }

    fn must_ident(&mut self) -> Result<Spanned<String>, DukaSpannedError> {
        self.inner.must_ident()
    }

    fn must_token(&mut self, token: TokenKind) -> Result<Token, DukaSpannedError> {
        self.inner.must_token(token)
    }

    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError> {
        self.inner.peek_token(n)
    }

    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        self.inner.next_token()
    }
}
