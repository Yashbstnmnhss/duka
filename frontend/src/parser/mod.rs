use std::collections::HashMap;
use std::sync::Arc;

use ast::{
    AttrName, Attrs, BangCollected, Block, DukaChunk, Expr, Field, FuncBody, If, IfClause, Linq,
    LinqClause, Match, MatchClause, Name, Node, NodeKind, ObjectDef, ObjectProperty, Param, Path,
    PathSuffix, PatternArrayTerm, PatternFieldTerm, PatternTerm, ReturnAnnotation, Stmt, TypeDesc,
    TypeFnValue, get_binop_info, get_logicop_info, get_patop_info, has_attr,
};
use duka_shared::{
    config::DukaParserConfig,
    constants::{catt, clex, cpar, ctype},
    dtype::Type,
    errors::{DukaLexerError, DukaParserError, DukaSpannedError, Span},
    types::{
        DukaParser, Fact, Goal, LogicDatabase, LogicOp, Query, QueryCount, Rule, SourceInfo,
        Spanned, SysCall, Term, TokenStream, UnOp,
    },
    utils::{MultiPeekable, MultiPeekableExtension, OrError, TryDo},
    value::{AttrValue, ConstValue},
};

use crate::{
    lexer::token::{
        EMPTY_TOKEN, Token,
        TokenKind::{self},
    },
    parser::{
        ast::{
            Attr, BangCollectedSource, BangDoNode, DestructingTableTerm, DestructingTerm,
            Destructuring, ExprOrStmt, ParamShape, TypeField, TypeOp, TypeParam, WhereClause,
            get_typeop_info, type_access, type_application,
        },
        bang::{BangExprHandler, BangHandler, BangHandlers, BangStmtHandler, ParserAPI},
    },
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
        $e?.ok_or($self.expected($expected))
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
    ($self: ident : try[$default: expr] opt($inside: expr)[$inside_default: expr] in $left: ident, $right: ident) => {
        if $self.then(TokenKind::$left)? {
            if $self.then(TokenKind::$right)? {
                $inside_default
            } else {
                let i = $inside?;
                $self.must_token(TokenKind::$right)?;
                i
            }
        } else {
            $default
        }
    };
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
    token_index: usize,
    source_info: SourceInfo,
    current_span: Span,
    /// What a `---` documents, keyed by the span of what it documents.
    ///
    /// The document is read here rather than kept on the statement because the
    /// statement is what a reader asks about and this is only how it is answered:
    /// one run of lines, joined, reaching the declaration under it.
    docs: HashMap<Span, String>,
    /// What a `---` documented, keyed by the span of the token it stands above.
    ///
    /// The statement it belongs to is not known when the run is read, and a nested
    /// block may read a run of its own before the outer statement ends.
    doc_at_token: HashMap<Span, String>,
    /// The `---` lines of the run being read, each with the line it is on.
    pending_doc: Vec<(String, u32)>,
    handlers: BangHandlers,
    config: DukaParserConfig,
    logic: LogicDatabase,
    typing_context: bool,
    /// Set while the contents of a `<...>` list are being read.
    ///
    /// There `>` closes the list rather than comparing two things, and the
    /// expression grammar has only one meaning for `>`, so an expression that
    /// runs to the right has to be stopped at it. This is set by the reader that
    /// owns the bracket, not guessed at by trying a reading and undoing it.
    in_type_args: bool,
}

#[derive(Debug, Clone)]
struct ParserCheckpoint {
    token_index: usize,
    current_span: Span,
    logic_lengths: (usize, usize, usize),
    typing_context: bool,
    in_type_args: bool,
}

#[derive(Debug, Clone)]
pub struct IncrementalParseCache {
    nodes: Vec<Stmt>,
    checkpoints: Vec<ParserCheckpoint>,
    docs: HashMap<Span, String>,
    logic: LogicDatabase,
    config: DukaParserConfig,
    safe: bool,
}

#[derive(Debug)]
enum VarDesc {
    Call(Box<NodeKind>),
    Var(Path),
}

/// main duka
impl Parser<Token> {
    pub fn new(stream: TokenStream<Token>, config: DukaParserConfig) -> Self {
        Self::from_tokens(stream, config)
    }

    /// The same parser, built so a caller that has to register handlers first
    /// can still finish the parse itself.
    pub fn from_tokens(stream: TokenStream<Token>, config: DukaParserConfig) -> Self {
        Self {
            tokens: stream.tokens.into_iter().multi_peekable(),
            token_index: 0,
            current_span: Span::default(),
            handlers: BangHandlers::default(),
            logic: LogicDatabase::default(),
            source_info: stream.source_info,
            config,
            typing_context: false,
            in_type_args: false,
            docs: HashMap::new(),
            doc_at_token: HashMap::new(),
            pending_doc: vec![],
        }
    }

    #[must_use]
    pub fn register_bang_expr(
        mut self,
        keyword: impl Into<String>,
        handler: BangHandler<Arc<dyn BangExprHandler>>,
    ) -> Self {
        if self.config.use_bang_expr {
            self.handlers.register_expr(keyword, handler);
        }
        self
    }
    #[must_use]
    pub fn register_bang_stmt(
        mut self,
        keyword: impl Into<String>,
        handler: BangHandler<Arc<dyn BangStmtHandler>>,
    ) -> Self {
        if self.config.use_bang_stmt {
            self.handlers.register_stmt(keyword, handler);
        }
        self
    }

    /// Collects `keyword! { .. }` whole, for an expander that reads it back as
    /// tokens.
    ///
    /// The parser collects a `!` body before the expander stage runs, so a name
    /// the expander knows and the parser was never told about is a parse error
    /// rather than something the expander gets a chance to look at.
    #[must_use]
    pub fn register_bang_collected_expr(mut self, keyword: impl Into<String>) -> Self {
        if self.config.use_bang_expr {
            self.handlers.register_expr(keyword, BangHandler::Tokens);
        }
        self
    }

    #[must_use]
    pub fn register_bang_collected_stmt(mut self, keyword: impl Into<String>) -> Self {
        if self.config.use_bang_stmt {
            self.handlers.register_stmt(keyword, BangHandler::Tokens);
        }
        self
    }

    #[must_use]
    #[inline]
    pub fn with_typing_context(mut self) -> Self {
        self.typing_context = true;
        self
    }

    /// Try parse the input as expression at first, if failed, then try parse it as statement.
    pub fn parse_expr_or_stmt(&mut self) -> Result<ExprOrStmt, DukaSpannedError> {
        let first = &self.peek_token(0)?.0.clone();
        if first.is_terminator() {
            return Ok(ExprOrStmt::Expr(Node(NodeKind::Empty, Span::EMPTY)));
        }

        // A leading identifier may be the lvalue of an assignment statement
        if matches!(first, TokenKind::Ident(..)) && self.looks_like_assignment() {
            let stmt = must!(self.stmt())?;
            self.no_more()?;
            return Ok(ExprOrStmt::Stmt(stmt));
        }

        if let Some(expr) = self.expr_inner(false)? {
            self.no_more()?;
            Ok(ExprOrStmt::Expr(expr))
        } else {
            let stmt = must!(self.stmt())?;
            self.no_more()?;
            Ok(ExprOrStmt::Stmt(stmt))
        }
    }

    /// Peek the token stream to decide whether the line ahead is a lvalue
    fn looks_like_assignment(&mut self) -> bool {
        let mut depth: isize = 0;
        let mut n = 1usize;
        loop {
            let Some((tk, _)) = self.tokens.peek_nth(n) else {
                return false;
            };
            match tk {
                TokenKind::Assign if depth == 0 => return true,
                tk if tk.is_left() => depth += 1,
                tk if tk.is_right() => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                tk if tk.is_terminator() => return false,
                _ => {}
            }
            n += 1;
        }
    }

    /// Parse all the input as a single chunk, return the block containing all of statments
    pub fn parse_chunk(&mut self) -> Result<Block, DukaSpannedError> {
        self.chunk()
    }

    #[inline(always)]
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

    /// Whether the token ahead can begin a statement, which is what decides
    /// where a `return` written in the middle of a block stops.
    ///
    /// A value list has no delimiter of its own, so `return` hands everything
    /// the expression reader takes to the end of its statement. What is left
    /// over is either the block's own terminator or the start of the next
    /// statement, and nothing else is a legal reading -- an identifier that
    /// goes on into `(`/`:`/`.`/`[` is the tail of the returned value, not a
    /// statement of its own.
    fn next_starts_statement(
        &mut self,
        consumed: &[TokenKind],
        retains: &[TokenKind],
    ) -> Result<bool, DukaSpannedError> {
        let (tk, _) = self.peek_token(0)?;
        if consumed.contains(tk) || retains.contains(tk) {
            return Ok(true);
        }
        if !matches!(tk, TokenKind::Ident(_)) {
            return Ok(matches!(
                tk,
                TokenKind::SemiColon
                    | TokenKind::LParen
                    | TokenKind::At
                    | TokenKind::DoubleColon
                    | TokenKind::Break
                    | TokenKind::Continue
                    | TokenKind::Return
                    | TokenKind::Goto
                    | TokenKind::Local
                    | TokenKind::Global
                    | TokenKind::Function
                    | TokenKind::Object
                    | TokenKind::If
                    | TokenKind::Match
                    | TokenKind::For
                    | TokenKind::While
                    | TokenKind::Do
                    | TokenKind::Type
                    | TokenKind::Export
            ));
        }
        if self.looks_like_assignment() {
            return Ok(true);
        }
        Ok(!matches!(
            self.peek_token(1)?.0,
            TokenKind::LParen | TokenKind::Colon | TokenKind::Dot | TokenKind::LBracket
        ))
    }

    fn unexpected_statement_token(&mut self) -> DukaSpannedError {
        let got = self
            .peek_token(0)
            .map(|t| t.0.name().to_owned())
            .unwrap_or_else(|_| TokenKind::EOF.name().to_owned());
        DukaSpannedError::new(
            DukaParserError::UnexpectedToken {
                got: got.into(),
                expected: "statement".into(),
            }
            .into(),
            self.current_span,
            self.source_info.clone(),
        )
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
                                        break Block(stmts.into())
                                    }

                                    if self.then(TokenKind::Return)? {
                                        let ret = self.ret_stmt()?;
                                        if !self.next_starts_statement(&consumed, &retains)? {
                                            return Err(self.unexpected_statement_token());
                                        }
                                        stmts.push(ret);
                                        continue
                                    }

                        // A `---` run directly above a statement documents that statement. The run is
                        // recorded against the token it stands above, because at this point the
                        // statement has not been read and a nested block may reach a run of its
                        // own on the way there. The document belongs to the statement, so it is
                        // moved onto the statement's span.
        let starts_at = self.peek_token(0)?.1;
                    let stmt = must!(self.stmt())?;
                    if let Some(doc) = self.doc_at_token.get(&starts_at) {
                        for span in declared_names(&stmt) {
                            self.docs.insert(span, doc.clone());
                        }
                    }
                    stmts.push(stmt)
                                })
    }

    fn skip_to_sync(&mut self, retains: &[TokenKind]) -> Result<bool, DukaSpannedError> {
        loop {
            let tk = &self.peek_token(0)?.0;
            if tk.is_terminator() {
                return Ok(false);
            }
            if tk.is_sync() || retains.contains(tk) {
                return Ok(true);
            }
            self.next_token()?;
        }
    }

    fn checkpoint(&self) -> ParserCheckpoint {
        ParserCheckpoint {
            token_index: self.token_index,
            current_span: self.current_span,
            logic_lengths: (
                self.logic.facts.len(),
                self.logic.rules.len(),
                self.logic.queries.len(),
            ),
            typing_context: self.typing_context,
            in_type_args: self.in_type_args,
        }
    }

    fn block_inner_recovery(
        &mut self,
        consumed: &[TokenKind],
        retains: &[TokenKind],
        errors: &mut Vec<DukaSpannedError>,
        max_errors: usize,
        mut checkpoints: Option<&mut Vec<ParserCheckpoint>>,
    ) -> Option<Block> {
        let mut stmts = vec![];
        loop {
            if errors.len() >= max_errors {
                errors.push(DukaSpannedError::new(
                    DukaParserError::TooManyErrors(max_errors).into(),
                    self.current_span,
                    self.source_info.clone(),
                ));
                return None;
            }
            let next_kind = self
                .peek_token(0)
                .map(|t| t.0.clone())
                .unwrap_or(TokenKind::EOF);
            if consumed.contains(&next_kind) || retains.contains(&next_kind) {
                if let Some(checkpoints) = checkpoints.as_deref_mut() {
                    checkpoints.push(self.checkpoint());
                }
                return Some(Block(stmts.into()));
            }
            if let Some(checkpoints) = checkpoints.as_deref_mut() {
                checkpoints.push(self.checkpoint());
            }

            // a `return` ends here only if what follows can begin the next
            // statement; otherwise the value list ran into something that is
            // neither, and the block is reported on and resynced from
            if next_kind == TokenKind::Return {
                let Ok(_) = self.next_token() else {
                    return Some(Block(stmts.into()));
                };
                match self.ret_stmt() {
                    Ok(ret) => match self.next_starts_statement(consumed, retains) {
                        Ok(true) => {
                            stmts.push(ret);
                            continue;
                        }
                        Ok(false) => errors.push(self.unexpected_statement_token()),
                        Err(e) => errors.push(e),
                    },
                    Err(e) => errors.push(e),
                }
                match self.skip_to_sync(retains) {
                    Ok(true) => continue,
                    _ => return Some(Block(stmts.into())),
                }
            }

            // The token this statement starts on is the one a `---` run above it
            // was recorded against; the document belongs to the statement, so it is
            // moved onto the statement's span.
            let starts_at = self.peek_token(0).ok().map(|t| t.1);
            match self.stmt() {
                Ok(Some(stmt)) => {
                    if let Some(doc) = starts_at.and_then(|at| self.doc_at_token.get(&at)) {
                        // keyed by every name the statement declares, because a
                        // symbol is found by its own name and not by the statement
                        // it was written in
                        for span in declared_names(&stmt) {
                            self.docs.insert(span, doc.clone());
                        }
                    }
                    stmts.push(stmt)
                }
                Ok(None) => {
                    // `stmt` declined without consuming anything, so the loop
                    // must not spin on the same token: report it and resync
                    let Ok((kind, _)) = self.peek_token(0) else {
                        return Some(Block(stmts.into()));
                    };
                    errors.push(DukaSpannedError::new(
                        DukaParserError::UnexpectedToken {
                            got: kind.name().into(),
                            expected: "statement".into(),
                        }
                        .into(),
                        self.current_span,
                        self.source_info.clone(),
                    ));
                    match self.skip_to_sync(retains) {
                        Ok(true) => continue,
                        _ => return Some(Block(stmts.into())),
                    }
                }
                Err(e) => {
                    errors.push(e);
                    match self.skip_to_sync(retains) {
                        Ok(true) => continue,
                        _ => return Some(Block(stmts.into())),
                    }
                }
            }
        }
    }

    #[inline]
    fn label(&mut self) -> TryDo<Name, DukaSpannedError> {
        Ok(opt![
            self then DoubleColon: {
                let label = self.must_ident()?;
                self.must_token(TokenKind::DoubleColon)?;
                Some(label)
            }
            else: None
        ])
    }

    fn stmt(&mut self) -> TryDo<Stmt, DukaSpannedError> {
        let (tk, start_span) = self.span_start()?;
        let kind = oneof!(
            try match tk =>
            TokenKind::LParen => {
                self.next_token()?;
                let expr = must!(self.expr())?;
                self.must_token(TokenKind::RParen)?;
                NodeKind::Expr(Box::new(expr))
            }
            TokenKind::SemiColon => {
                self.next_token()?;
                NodeKind::Empty
            }
            TokenKind::At => {
                let attrs = self.attrs()?;
                let start = self.current_span;
                let exported = self.then(TokenKind::Export)?;
                let def = self.def(attrs)?;
                if exported {
                    NodeKind::Export(Box::new(
                        self.stmt_end(def, start)
                    ))
                } else {
                    def
                }
            }
            // functionCall or varlist
            TokenKind::Ident(_) => {
                self.when_ident()?
            }
            TokenKind::DoubleColon => {
                self.next_token()?; // consume "::"
                let label = self.must_ident()?;
                self.must_token(TokenKind::DoubleColon)?;
                NodeKind::Label(label)
            }
            TokenKind::Break => {
                self.next_token()?;

                let label = self.label()?;
                let values = opt![self.expr_list()]?.unwrap_or_default();
                NodeKind::Break(label, values.into())
            }
            TokenKind::Continue => {
                self.next_token()?;

                let label = self.label()?;
                NodeKind::Continue(label)
            }
            TokenKind::Goto => {
                self.next_token()?;
                let label = must!(self.label())?;
                NodeKind::Goto(label)
            }
            TokenKind::Local | TokenKind::Global => {
                self.def([].into())?
            }
            TokenKind::Function | TokenKind::Object => {
                self.def([].into())?
            }
            TokenKind::If => {
                self.next_token()?;
                NodeKind::If(self.if_block(false)?)
            }
            TokenKind::Match => {
                self.next_token()?;
                NodeKind::Match(self.match_block(false)?)
            }
            TokenKind::For => {
                self.next_token()?;
                let banged = self.then(TokenKind::Bang)?;
                self.for_stmt(banged)?
            }
            TokenKind::While => {
                self.next_token()?;
                let banged = self.then(TokenKind::Bang)?;

                let label = self.label()?;

                let cond = must!(self.expr())?;
                self.must_token(TokenKind::Do)?;
                let body = self.block([TokenKind::End])?;

                NodeKind::While(Box::new(cond), Box::new(body), banged, label)
            }
            TokenKind::Do => {
                self.next_token()?;
                let banged = self.then(TokenKind::Bang)?;
                let label = self.label()?;
                NodeKind::Do(Box::new(self.block([TokenKind::End])?), banged, label)
            }
            TokenKind::Type => {
                self.next_token()?;
                self.ty_def()?
            }
            TokenKind::Export => {
                self.next_token()?;
                let start = self.current_span;
                let exported = if self.then(TokenKind::Type)? {
                    self.ty_def()?
                } else {
                    self.def([].into())?
                };
                NodeKind::Export(Box::new(
                    self.stmt_end(exported, start)
                ))
            }
        );
        Ok(Some(self.stmt_end(kind, start_span)))
    }

    // Must!, `type function <name>` is consumed already
    fn type_function(&mut self) -> Result<FuncBody, DukaSpannedError> {
        let saved = self.typing_context;
        self.typing_context = true;
        let body = self.function_body(false);
        self.typing_context = saved;
        body
    }

    /// Reads the right-hand side of a `type` definition -- `type A = ...` and the
    /// one-line `type f = ...` form alike.
    ///
    /// It is read the way the body of a `type function` is read, which is the
    /// only other place in the language a type is written as something that
    /// computes rather than as a shape. Both put `typing_context` on, and that
    /// flag is what lets an expression be read here: a type-level `C + 1` is an
    /// `NodeKind::Binary`, and the only way a type position can hold one is
    /// `TypeDesc::Expr`, which `try_simple_type_value` produces.
    ///
    /// Without it, the type grammar was all there was, and that grammar has two
    /// operators -- `|` and `&` -- so `type A = C + 1` stopped after `C` and left
    /// `+ 1` in the stream to be reported as a stray token. Nothing about `+` was
    /// missing from the language; only the reading of this position was, and the
    /// evaluator was already waiting on the other end of `TypeDesc::Expr`.
    ///
    /// Nilability is off for the same reason it is off inside a type function:
    /// this is what a type is, not a place a value is kept. It used to be set
    /// separately at each of the two call sites, which meant a third one could
    /// forget.
    fn ty_def(&mut self) -> Result<NodeKind, DukaSpannedError> {
        Ok(if self.then(TokenKind::Function)? {
            let name = self.must_ident()?;
            let mut inline = false;
            let mut i = 0usize;
            if matches!(self.peek_token(0)?.0, TokenKind::Less) {
                let mut depth = 0usize;
                loop {
                    let tk = self.peek_token(i)?.0.clone();
                    if tk.is_terminator() {
                        break;
                    }
                    match tk {
                        TokenKind::Less => depth += 1,
                        TokenKind::Greater => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            if matches!(self.peek_token(i)?.0, TokenKind::LParen) {
                let mut depth = 0usize;
                loop {
                    let tk = self.peek_token(i)?.0.clone();
                    if tk.is_terminator() {
                        break;
                    }
                    match tk {
                        TokenKind::LParen => depth += 1,
                        TokenKind::RParen => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                inline = matches!(self.peek_token(i)?.0, TokenKind::Assign);
            }
            if inline {
                if matches!(self.peek_token(0)?.0, TokenKind::Less) {
                    let _ = self.opt_ty_par_def_list()?;
                }
                let params = between!(self:
                    must opt(self.type_fn_params().map(|b| b.into_vec()))[vec![]]
                    in LParen, RParen
                );
                self.must_token(TokenKind::Assign)?;
                let saved = self.config.default_nonnilable;
                self.config.default_nonnilable = true;
                let ty = self.parse_type_annotation()?;
                self.config.default_nonnilable = saved;
                NodeKind::InlineTypeFunction(name, params.into_boxed_slice(), Box::new(ty))
            } else {
                let body = self.type_function()?;
                NodeKind::TypeFunction(name, Box::new(body))
            }
        } else {
            let (name, name_span) = self.must_ident()?;
            self.must_token(TokenKind::Assign)?;
            let saved = self.config.default_nonnilable;
            self.config.default_nonnilable = true;
            let ty = self.parse_type_annotation()?;
            self.config.default_nonnilable = saved;
            NodeKind::TypeAlias((name, name_span), Box::new(ty))
        })
    }

    /// Object, function and variables definitions
    fn def(&mut self, attrs: Attrs) -> Result<NodeKind, DukaSpannedError> {
        let global = oneof![if self.then(TokenKind::Local)? {
            false
        } else if self.then(TokenKind::Global)? {
            true
        } else {
            // an object declaration is always global, whatever prefix is written in front of it
            matches!(self.peek_token(0)?.0, TokenKind::Object) || !self.config.var_default_local
        }];
        self.pure_def(global, attrs)
    }

    /// Object, function and variables definitions, without `local global export`
    fn pure_def(&mut self, global: bool, attrs: Attrs) -> Result<NodeKind, DukaSpannedError> {
        Ok(oneof! {if self.then(TokenKind::Object)? {
            NodeKind::Object(Box::new(self.object(global, attrs)?))
        } else if self.then(TokenKind::Function)? {
            self.function(global, attrs)?
        } else if matches!(self.peek_token(0)?.0, TokenKind::LBrace | TokenKind::LBracket) {
            self.destruct_var(global)?
        } else {
            let banged = self.then(TokenKind::Bang)?;
            self.attr_var(global, banged, attrs)?
        }})
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
                let Node(expr, span) = must!(self.expr())?;
                self.must_token(TokenKind::SemiColon)?;
                let stmt = NodeKind::Return(Box::new([Node(expr, span)]), false);
                Block(Box::new([Node(stmt, span)]))
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
        if self.typing_context
            && let TokenKind::Ident(_) = self.peek_token(0)?.0
            && let TokenKind::LParen = self.peek_token(1)?.0
        {
            let name = if self.then(TokenKind::Function)? | self.then(TokenKind::Fn)? {
                (ctype::FUN.to_owned(), self.current_span)
            } else if self.then(TokenKind::Object)? {
                (ctype::OBJ.to_owned(), self.current_span)
            } else {
                self.must_ident()?
            };
            self.next_token()?;
            let mut args = vec![];
            if !self.then(TokenKind::RParen)? {
                loop {
                    args.push(self.match_pattern(0)?);
                    if self.then(TokenKind::RParen)? {
                        break;
                    }
                    self.must_token(TokenKind::Comma)?;
                }
            }
            return Ok(PatternTerm::Type(name, args.into()));
        }
        Ok(oneof!(
            try match self.peek_token(0)?.0 => {
                TokenKind::Pipeline(ref pl) => {
                    let pl = pl.clone();
                    self.next_token()?;
                    let func = must!(self.atom_exp(true))?;
                    let sub = if self.then(TokenKind::Then)? {
                        Some(Box::new(self.match_pattern(0)?))
                    } else {
                        None
                    };
                    PatternTerm::Call(pl, Box::new(func), sub)
                },
                TokenKind::LParen => between!(self:
                    must nonempty(self.match_pattern(0))
                    in LParen, RParen
                ),
                TokenKind::LBracket => between!(self:
                    must opt(self.match_atom_arrtab_pattern(true))[PatternTerm::Table(Box::new([]))]
                    in LBracket, RBracket
                ),
                TokenKind::LBrace => between!(self:
                    must opt(self.match_atom_arrtab_pattern(false))[PatternTerm::Array(Box::new([]))]
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
                    let ty = opt![self then
                        Colon: {
                            Some(self.parse_type_annotation()?)
                        } else: None
                    ];
                    PatternTerm::Bind(name, ty)
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

    /// Table or Array
    fn match_atom_arrtab_pattern(&mut self, array: bool) -> Result<PatternTerm, DukaSpannedError> {
        if array {
            let mut items = vec![];

            items.push(self.match_array_pattern()?);

            many! {
                self then Comma or SemiColon:
                items.push(self.match_array_pattern()?);
            }
            Ok(PatternTerm::Array(items.into()))
        } else {
            let mut fields = vec![];

            fields.push(self.match_field_pattern()?);

            many! {
                self then Comma or SemiColon:
                fields.push(self.match_field_pattern()?);
            }

            Ok(PatternTerm::Table(fields.into()))
        }
    }

    /// must!
    fn match_array_pattern(&mut self) -> Result<PatternArrayTerm, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::Dots)? {
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
                }
                else: 1
            ])
        } else {
            let term = self.match_pattern(0)?;
            PatternArrayTerm::Term(term)
        }))
    }

    fn match_field_pattern(&mut self) -> Result<PatternFieldTerm, DukaSpannedError> {
        Ok(oneof!(if self.then(TokenKind::LBracket)? {
            let key = must!(self.expr())?;

            self.must_token(TokenKind::RBracket)?;
            self.must_token(TokenKind::Assign)?;

            let pattern = self.match_pattern(0)?;

            PatternFieldTerm::Expr(key, pattern)
        } else if self.lookahead_token(TokenKind::Assign, 1)? {
            let key = self.must_ident()?;
            self.must_token(TokenKind::Assign)?;

            let pattern = self.match_pattern(0)?;

            PatternFieldTerm::Named(key, pattern)
        } else {
            PatternFieldTerm::Array(self.match_array_pattern()?)
        }))
    }

    fn object(&mut self, global: bool, attrs: Attrs) -> Result<ObjectDef, DukaSpannedError> {
        let name = self.must_ident()?;

        let generics = self.opt_ty_par_def_list()?;

        let base = opt![
            self then Colon:
            { Some(self.must_ident()?) }
            else: None
        ];
        let base = opt![
            self then Extends:
            { Some(self.must_ident()?) }
            else: base
        ];

        let mut static_methods = vec![];
        let mut methods = vec![];
        let mut properties = vec![];

        many! {
            loop:
            oneof! {if:
                case self.then(TokenKind::At)? => {
                    let attrs = self.attrs()?;
                    self.must_token(TokenKind::Function)?;
                    let is_static = !self.then(TokenKind::Colon)?;
                    let name = self.must_ident()?;
                    let body = self.function_body(has_attr(&attrs, catt::DECLARE))?;
                    let func = (name, attrs, body);

                    if is_static {
                        static_methods.push(func);
                    } else {
                        methods.push(func);
                    }
                },
                case self.then(TokenKind::Function)? => {
                    let is_static = !self.then(TokenKind::Colon)?;
                    let name = self.must_ident()?;
                    let body = self.function_body(false)?;
                    let func = (name, [].into(), body);

                    if is_static {
                        static_methods.push(func);
                    } else {
                        methods.push(func);
                    }
                },
                case self.then(TokenKind::LBracket)? => {
                    let key = must!(self.expr())?;
                    self.must_token(TokenKind::RBracket)?;
                    let ty = opt![
                        self then Colon: {
                            Some(self.parse_type_annotation()?)
                        }
                        else: None
                    ];
                    let val = opt![
                        self then Assign: {
                            Some(must!(self.expr())?)
                        }
                        else: None
                    ];
                    properties.push(ObjectProperty::KeyValue(Box::new(key), val.map(Box::new), ty))
                },
                case matches!(self.peek_token(0)?.0, TokenKind::Ident(..)) => {
                    let key = self.must_ident()?;
                    let ty = opt![
                        self then Colon: {
                            Some(self.parse_type_annotation()?)
                        }
                        else: None
                    ];
                    let val = opt![
                        self then Assign: {
                            Some(must!(self.expr())?)
                        }
                        else: None
                    ];
                    properties.push(ObjectProperty::NameValue(key, val.map(Box::new), ty));
                }
                else: break
            }
        }

        self.must_token(TokenKind::End)?;

        Ok(ObjectDef {
            global,
            name,
            attrs,
            base,
            type_params: generics.into(),
            properties: properties.into(),
            static_methods: static_methods.into(),
            methods: methods.into(),
        })
    }

    fn function(&mut self, global: bool, attrs: Attrs) -> Result<NodeKind, DukaSpannedError> {
        let name = must!(self.func_name())?;
        let body = self.function_body(has_attr(&attrs, catt::DECLARE))?;
        Ok(NodeKind::FunctionDecl(name, attrs, Box::new(body), global))
    }

    #[inline]
    fn destruct_term(&mut self) -> Result<DestructingTerm, DukaSpannedError> {
        Ok(oneof! {
            if self.then(TokenKind::LBrace)? {
                let mut items = vec![];
                many! {loop:
                    if self.then(TokenKind::RBrace)? {
                        break
                    }

                    let name = must!(self.simple_name())?;
                    items.push(if self.then(TokenKind::Assign)? {
                        let term = self.destruct_term()?;
                        DestructingTableTerm(name, term)
                    }
                    else if name.0 == cpar::DISCARD {
                        // `{ a, _ }` reads the field `a` and drops the next one
                        let span = name.1;
                        DestructingTableTerm(name, DestructingTerm::Ignored(span))
                    }
                    else {
                        DestructingTableTerm(name.clone(), DestructingTerm::Bind(name))
                    });


                    if self.then(TokenKind::RBrace)? {
                        break
                    }
                    self.must_token(TokenKind::Comma)?;
                }
                DestructingTerm::Term(Destructuring::Table(
                    items.into_boxed_slice()
                ))
            }
            else if self.then(TokenKind::LBracket)? {
                let items: Vec<DestructingTerm> = list!(self:
                    by Comma separate (self.destruct_term())
                    nonempty
                );
                self.must_token(TokenKind::RBracket)?;
                DestructingTerm::Term(Destructuring::Array(
                    items.into_boxed_slice()
                ))
            }
            else {
                // `_` reads a place and drops it, and `...` takes what is left.
                // Neither is a name: a name would be in scope after the statement.
                let span = self.current_span;
                if self.then(TokenKind::Dots)? {
                    let name = if matches!(self.peek_token(0)?.0, TokenKind::Ident(_)) {
                        let (name, _) = must!(self.simple_name())?;
                        Some((name, span))
                    } else {
                        None
                    };
                    return Ok(DestructingTerm::Rest(name, span));
                }
                let name = must!(self.simple_name())?;
                if name.0 == cpar::DISCARD {
                    return Ok(DestructingTerm::Ignored(name.1));
                }
                DestructingTerm::Bind(name)
            }
        })
    }

    /// # `{` or `[` is NOT comsumed
    fn destruct_var(&mut self, global: bool) -> Result<NodeKind, DukaSpannedError> {
        let term = self.destruct_term()?;
        self.must_token(TokenKind::Assign)?;
        let expr = must!(self.expr())?;

        Ok(match term {
            DestructingTerm::Bind((n, s)) => NodeKind::Define(
                [(((n, s), [].into(), None), s)].into(),
                [expr].into(),
                global,
                false,
            ),
            DestructingTerm::Term(d) => NodeKind::Destructing(d, Box::new(expr), global),
            // neither form reaches here: `_` and `...` are not `local` on their
            // own, they are places inside a pattern
            DestructingTerm::Ignored(_) | DestructingTerm::Rest(..) => {
                return Err(self.err(DukaParserError::UnexpectedToken {
                    got: "_".into(),
                    expected: "a name, or a pattern of { } and [ ]".into(),
                }));
            }
        })
    }

    fn attr_var(
        &mut self,
        global: bool,
        banged: bool,
        all_attrs: Attrs,
    ) -> Result<NodeKind, DukaSpannedError> {
        let vars: Vec<AttrName> = self.attr_name_list()?;
        let vars: Vec<AttrName> = if all_attrs.is_empty() {
            vars
        } else {
            vars.into_iter()
                .map(|((name, attrs, ty), span)| {
                    let mut merged: Vec<Attr> = all_attrs.to_vec();
                    merged.extend(attrs);
                    ((name, merged.into(), ty), span)
                })
                .collect()
        };

        Ok(NodeKind::Define(
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
            banged,
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

        Ok(self.stmt_end(NodeKind::Return(exps.into(), false), start_span))
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

    fn for_stmt(&mut self, banged: bool) -> Result<NodeKind, DukaSpannedError> {
        let label = self.label()?;
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

            NodeKind::ForNumeric(var, Box::new(init), Box::new(cond), step.map(Box::new), Box::new(body), label)
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

            NodeKind::ForGeneric(vars, exps.into(), Box::new(body), banged, label)
        ))
    }

    fn bang_stmt(&mut self, name: Name) -> Result<NodeKind, DukaSpannedError> {
        if self.then(TokenKind::Do)? {
            self.next_token()?;
            let body = self.block([TokenKind::End])?;

            return Ok(NodeKind::BangDo(BangDoNode {
                context: Box::new(name.1 * NodeKind::Access(Box::new(Path::Base(name)))),
                body: Box::new(body),
            }));
        }
        self.must_token(TokenKind::LBrace)?;
        let res = match name.0.as_str() {
            "logic" => {
                self.logic_block()?;
                NodeKind::Empty
            }
            n => {
                let handler = self
                    .handlers
                    .get_stmt(n)
                    .ok_or_else(|| self.err(DukaParserError::UnknownBang(n.into())))?;
                match handler {
                    BangHandler::Handler(handler) => {
                        let mut wrapper = ParserWrapper { inner: self };
                        handler.handle(&mut wrapper)?
                    }
                    BangHandler::Raw => {
                        let from = self.current_span.start.at_char as usize;
                        self.collect_tokens_until(TokenKind::RBrace)?;
                        let end = self.current_span.end.at_char as usize;
                        NodeKind::BangCollected(BangCollected {
                            name: name.0,
                            source: BangCollectedSource::Raw(
                                str::from_utf8(&self.source_info.source[from..end])
                                    .expect("Checked in lexer")
                                    .to_owned(),
                            ),
                        })
                    }
                    BangHandler::Tokens => {
                        let start = self.current_span;
                        let tokens = self.collect_tokens_until(TokenKind::RBrace)?;
                        NodeKind::BangCollected(BangCollected {
                            name: name.0,
                            source: BangCollectedSource::Tokens(tokens, start + self.current_span),
                        })
                    }
                }
            }
        };
        self.must_token(TokenKind::RBrace)?;
        Ok(res)
    }
    fn bang_expr(&mut self, name: Name) -> Result<NodeKind, DukaSpannedError> {
        let next = self.peek_token(0)?;
        oneof! {
            if next.0 == TokenKind::Do {
                self.next_token()?;
                let body = self.block([TokenKind::End])?;

                Ok(
                    NodeKind::BangDo(BangDoNode {
                        context: Box::new(name.1 * NodeKind::Access(
                            Box::new(Path::Base(name))
                        )),
                        body: Box::new(body),
                    })
                )
            }
            else if let TokenKind::LParen | TokenKind::LBrace = next.0 {
                let right_p = matches!(next.0, TokenKind::LParen);
                self.next_token()?;
                let res = match name.0.as_str() {
                    "logic" => NodeKind::SysCall(self.logic_query()?),
                    "linq" => NodeKind::Linq(self.linq_expr()?),
                    n => {
                        let handler = self
                            .handlers
                            .get_expr(n)
                            .ok_or_else(|| self.err(DukaParserError::UnknownBang(n.into())))?;
                        match handler {
                            BangHandler::Handler(handler) => {
                                let mut wrapper = ParserWrapper { inner: self };
                                handler.handle(&mut wrapper)?
                            },
                            BangHandler::Raw => {
                                let from = self.current_span.start.at_char as usize;
                                self.collect_tokens_until(if right_p { TokenKind::RParen } else { TokenKind::RBrace })?;
                                let end = self.current_span.end.at_char as usize;
                                NodeKind::BangCollected(BangCollected {
                                    name: name.0,
                                    source: BangCollectedSource::Raw(str::from_utf8(&self.source_info.source[from..end]).expect("Checked in lexer").to_owned()),
                                })
                            },
                            BangHandler::Tokens => {
                                let start = self.current_span;
                                let tokens = self.collect_tokens_until(if right_p { TokenKind::RParen } else { TokenKind::RBrace })?;
                                NodeKind::BangCollected(BangCollected {
                                    name: name.0,
                                    source: BangCollectedSource::Tokens(tokens, start + self.current_span),
                                })
                            }
                        }
                    }
                };
                self.must_token(if right_p { TokenKind::RParen } else { TokenKind::RBrace })?;
                Ok(res)
            }
            else {
                Err(self.err(DukaParserError::UnknownBang(name.0.into())))
            }
        }
    }

    fn collect_tokens_until(&mut self, close: TokenKind) -> Result<Vec<Token>, DukaSpannedError> {
        let open = match &close {
            TokenKind::RBrace => TokenKind::LBrace,
            TokenKind::RParen => TokenKind::LParen,
            TokenKind::RBracket => TokenKind::LBracket,
            _ => return Err(self.expected("closing delimiter")),
        };
        let mut depth = 1u32;
        let mut tokens = vec![];
        loop {
            // the closing token is left for the caller, which is what consumes
            // it: this only reads the body
            let (kind, _span) = self.peek_token(0)?.clone();
            if kind == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(tokens);
                }
            } else if kind == open {
                depth += 1;
            } else if kind == TokenKind::EOF {
                return Err(self.expected("closing delimiter"));
            }
            tokens.push(self.next_token()?);
        }
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
            case self.then(TokenKind::Where)? => {
                let expr = must!(self.expr())?;
                LinqClause::Where(Box::new(expr))
            }
            else: return Ok(None)
        )))
    }

    #[inline]
    fn when_ident(&mut self) -> Result<NodeKind, DukaSpannedError> {
        oneof!(
            if self.config.use_bang_stmt && self.lookahead_token(TokenKind::Bang, 1)? {
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
                            let Node(right, expr_span) = must!(self.expr())?;

                            NodeKind::Assign(
                                [name.clone()].into(),
                                [Node(
                                    NodeKind::Binary(
                                        Box::new(Node(NodeKind::Access(name.into()), span)),
                                        Box::new(Node(right, expr_span)),
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
                                        self.expected(cpar::VAR)
                                    ),
                                });
                            }

                            self.must_token(TokenKind::Assign)?;

                            let exps = must!(self.expr_list())?;

                            NodeKind::Assign(vars.into(), exps.into())
                        }
                    }
                }))
            }
        )
    }

    #[inline]
    fn attr_property(&mut self) -> Result<(Name, AttrValue), DukaSpannedError> {
        let key = must!(self.simple_name())?;
        let lit = if self.then(TokenKind::Equal)? {
            let val = must!(self.atom_exp(false))?;
            let NodeKind::Literal(lit) = val.0 else {
                return Err(self.err(DukaParserError::OnlyAcceptLiteral));
            };
            AttrValue::Single(lit)
        } else if self.then(TokenKind::LParen)? {
            let nodes = must!(self.expr_list())?;
            self.must_token(TokenKind::RParen)?;
            let vals = nodes
                .into_iter()
                .map(|val| {
                    let NodeKind::Literal(lit) = val.0 else {
                        return Err(self.err(DukaParserError::OnlyAcceptLiteral));
                    };
                    Ok(lit)
                })
                .collect::<Result<Vec<_>, _>>()?;
            match vals.as_slice() {
                [] => AttrValue::Single(ConstValue::Bool(true)),
                [_] => AttrValue::Single(vals.into_iter().next().unwrap()),
                _ => AttrValue::Many(vals),
            }
        } else {
            AttrValue::Single(ConstValue::Bool(true))
        };
        Ok((key, lit))
    }
    #[inline]
    fn attr_properties(&mut self) -> Result<Box<[(Name, AttrValue)]>, DukaSpannedError> {
        let v: Vec<_> = list!(self:
            by Comma separate (self.attr_property())
            nonempty
        );
        Ok(v.into_boxed_slice())
    }
    #[inline]
    fn attr(&mut self) -> Result<Attr, DukaSpannedError> {
        let name = must!(self.simple_name())?;
        let properties = between!(self:
            try[Box::new([])] opt(
                self.attr_properties()
            )[Box::new([])]
            in LParen, RParen
        );
        Ok((name, properties))
    }
    #[inline]
    fn attrs(&mut self) -> Result<Attrs, DukaSpannedError> {
        let mut attrs = vec![];
        many! {
            self then At:
            attrs.push(self.attr()?);
        }
        Ok(attrs.into_boxed_slice())
    }

    #[inline(always)]
    fn simple_name(&mut self) -> TryDo<Name, DukaSpannedError> {
        self.expect_ident()
    }

    #[inline]
    /// Not only attribute, but also type
    fn var_attr_name(&mut self) -> Result<AttrName, DukaSpannedError> {
        // `@declare x: int` and `x: int @declare` are the same statement, and the
        // one written in front is the one a declaration usually takes.
        let prefix = self.attrs()?;
        let (name, span) = must!(self.simple_name(), clex::ID)?;
        let ty = if self.config.type_annotations && self.then(TokenKind::Colon)? {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };
        let suffix = self.suffix_attrs()?;
        let merged: Attrs = prefix.into_iter().chain(suffix).collect();
        Ok((((name, span), merged, ty), span))
    }

    /// The attributes written after a variable's name, and only those.
    ///
    /// A suffix attribute sits where the declaration goes on after it, and the
    /// only two things that go on are another name and the initialiser -- so
    /// `,` and `=` are what prove an `@` belongs to the variable in front of it.
    /// That has to be read *before* the run, because an attribute run cannot be
    /// given back once it has been read.
    ///
    /// Without that, the `@` that starts the next statement was read as this
    /// variable's own: `local a: int` followed by `@const b = 3` moved `@const`
    /// onto `a` and left `b` plain, and neither line was an error.
    fn suffix_attrs(&mut self) -> Result<Attrs, DukaSpannedError> {
        if !self.attr_run_continues_declaration()? {
            return Ok([].into());
        }
        self.attrs()
    }

    /// Look ahead over an `@name(...)` run and answer whether the variable
    /// declaration it was written in goes on after it.
    #[inline]
    fn attr_run_continues_declaration(&mut self) -> Result<bool, DukaSpannedError> {
        let mut i = 0usize;
        loop {
            if !matches!(self.peek_token(i)?.0, TokenKind::At) {
                return Ok(false);
            }
            i += 1;
            if !matches!(self.peek_token(i)?.0, TokenKind::Ident(_)) {
                return Ok(false);
            }
            i += 1;
            if matches!(self.peek_token(i)?.0, TokenKind::LParen) {
                let mut depth = 0usize;
                loop {
                    match &self.peek_token(i)?.0 {
                        TokenKind::LParen => depth += 1,
                        TokenKind::RParen => depth -= 1,
                        t if t.is_terminator() => return Ok(false),
                        _ => {}
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            if !matches!(self.peek_token(i)?.0, TokenKind::At) {
                break;
            }
        }
        Ok(matches!(
            self.peek_token(i)?.0,
            TokenKind::Comma | TokenKind::Assign
        ))
    }

    // 涉及左递归
    fn prefix_exp(&mut self) -> TryDo<Expr, DukaSpannedError> {
        let (tk, start_span) = self.span_start()?;
        let mut res = oneof!(
            try match tk =>
            TokenKind::Function => {
                self.next_token()?;
                let func = self.function_body(false)?;
                self.expr_end(NodeKind::Function(func), start_span)
            }
            TokenKind::LBracket => {
                let array = must!(self.array_constructor())?;
                self.expr_end(array, start_span)
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

                if self.config.use_bang_expr && self.then(TokenKind::Bang)? {
                    let bang = self.bang_expr(name)?;
                    return Ok(Some(self.expr_end(bang, start_span)))
                }

                self.expr_end(NodeKind::Access(Box::new(Path::Base(name))), start_span)
            }
        );

        fn chain(former: Expr, new: PathSuffix, end: Span) -> Expr {
            let Node(kind, start) = former;
            Node(
                NodeKind::Access(Box::new(if let NodeKind::Access(base) = kind {
                    *base + new
                } else {
                    Path::Expr(Box::new(Node(kind, start))) + new
                })),
                start + end,
            )
        }

        many! {
            loop:

            if self.peek_token(0)?.0 == TokenKind::Question
                && [TokenKind::LBracket, TokenKind::Dot, TokenKind::Colon, TokenKind::LParen].contains(&self.peek_token(1)?.0) {
                self.next_token()?;
                let span = self.current_span;
                res = chain(res, PathSuffix::NilCheck(span), span);
            }

            res = oneof! {
                if self.then(TokenKind::LBracket)? {
                    let expr = must!(self.expr())?;
                    self.must_token(TokenKind::RBracket)?;

                    chain(res, PathSuffix::Index(Box::new(expr)), self.current_span)
                } else if self.then(TokenKind::Dot)? {
                    if self.config.type_annotations && matches!(&self.peek_token(0)?.0, TokenKind::Less) {
                        self.next_token()?;
                        let mut targs = vec![];
                        let saved = self.config.default_nonnilable;
                        self.config.default_nonnilable = true;
                        let result = {
                            loop {
                                targs.push(self.parse_type_annotation()?);
                                if self.close_angle()? {
                                    break;
                                }
                                self.must_token(TokenKind::Comma)?;
                            }
                            Ok(())
                        };
                        self.config.default_nonnilable = saved;
                        result?;
                        chain(
                            res,
                            PathSuffix::TypeArgs(targs.into(), self.current_span),
                            self.current_span,
                        )
                    } else {
                        let name = self.must_attr_name()?;

                        chain(res, PathSuffix::Dot(name), self.current_span)
                    }
                } else if self.then(TokenKind::Colon)? {
                    let name = self.must_attr_name()?;
                    let args = must!(self.args())?;
                    let func = chain(res, PathSuffix::Colon(name), self.current_span);
                    self.expr_end(NodeKind::Call(Box::new(func), args.into()), start_span)
                } else if let Some(args) = self.args()? {
                    self.expr_end(NodeKind::Call(Box::new(res), args.into()), start_span)
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
                        VarDesc::Var(p) => base = p,
                        VarDesc::Call(_) => unreachable!()
                    }
                } else {
                    base = base + suffix
                }
            }
            else if let Some(args) = self.args()? {
                match self.var_func_suffix(base, args)? {
                    VarDesc::Var(p) => base = p,
                    VarDesc::Call(_) => unreachable!()
                }
            } else {
                break
            }
        }

        let end_span = self.current_span;
        Ok((
            if let Some(args) = self.args()? {
                let callee = Node(NodeKind::Access(Box::new(base)), start_span + end_span);
                VarDesc::Call(Box::new(NodeKind::Call(Box::new(callee), args.into())))
            } else {
                match base {
                    Path::Expr(expr) => match *expr {
                        Node(NodeKind::Call(callee, args), _) => {
                            VarDesc::Call(Box::new(NodeKind::Call(callee, args)))
                        }
                        other => VarDesc::Var(Path::Expr(Box::new(other))),
                    },
                    other => VarDesc::Var(other),
                }
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
                if self.config.type_annotations && matches!(&self.peek_token(0)?.0, TokenKind::Less) {
                    self.next_token()?;
                    let mut targs = vec![];
                    let saved = self.config.default_nonnilable;
                    self.config.default_nonnilable = true;
                    let result = {
                        loop {
                            targs.push(self.parse_type_annotation()?);
                            if self.close_angle()? {
                                break;
                            }
                            self.must_token(TokenKind::Comma)?;
                        }
                        Ok(())
                    };
                    self.config.default_nonnilable = saved;
                    result?;
                    PathSuffix::TypeArgs(targs.into(), self.current_span)
                } else {
                    let name = self.must_attr_name()?;

                    PathSuffix::Dot(name)
                }
            },
            case self.then(TokenKind::Colon)? => {
                let name = self.must_attr_name()?;

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
        let call = NodeKind::Call(
            Box::new(self.expr_end(NodeKind::Access(Box::new(base)), span)),
            args.into(),
        );
        let path = Path::Expr(Box::new(self.expr_end(call, span)));

        Ok(VarDesc::Var(if let Some(suffix) = self.var_suffix()? {
            path + suffix
        } else {
            path
        }))
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

    #[inline]
    fn type_fn(&mut self) -> Result<FuncBody, DukaSpannedError> {
        let saved = self.typing_context;
        self.typing_context = true;
        let body = self.fn_body();
        self.typing_context = saved;
        body
    }

    /// The precedence at which a `type fn` body stops reading operators.
    ///
    /// `type fn(t) t?` can sit inside a type argument list, and that list is
    /// closed by `>`. The expression grammar has one meaning for `>` and it is
    /// "compare", so an unbounded reading swallows the closing bracket and then
    /// demands a right-hand side that is not there -- which is how
    /// `array<type fn(t) t>` failed to parse. This is the same boundary a
    /// bracketed expression gets for free from its closer, written down because
    /// the body of an inline `type fn` is not bracketed.
    ///
    /// It is the precedence of the comparison group, plus one: at exactly that
    /// precedence the operator is reported as misplaced rather than treated as
    /// the end of the expression.
    ///
    /// `and` and `or` are ordinary binary operators, looser than the
    /// comparisons, so a body that wants them is written as a `type function`
    /// with a block -- which is the value-context spelling too.
    fn type_fn_body_limit() -> u8 {
        get_binop_info(&TokenKind::Equal)
            .map(|(_, (l, _))| l.saturating_add(1))
            .unwrap_or_default()
    }

    /// without fn keyword
    fn fn_body(&mut self) -> Result<FuncBody, DukaSpannedError> {
        let generics = self.opt_ty_par_def_list()?;
        let params = between!(self:
            must opt(self.par_list())[vec![]]
            in LParen, RParen
        );

        let ret = if self.config.type_annotations && self.then(TokenKind::Colon)? {
            let start = self.current_span;
            let (tys, var_arg) = self.parse_fn_returns()?;
            Some(ReturnAnnotation {
                tys,
                var_arg,
                span: start + self.current_span,
            })
        } else {
            None
        };

        // Inside a `<...>` list the `>` that follows is the end of the list, so
        // the body stops at it. Everywhere else this is an ordinary expression
        // and runs as far as it likes, which is what the standard library's own
        // `type fn` bodies rely on.
        let body = if self.in_type_args {
            must!(self.expr_limit(Self::type_fn_body_limit(), false))?
        } else {
            must!(self.expr())?
        };
        let span = body.1;

        Ok(FuncBody(
            params.into_boxed_slice(),
            generics.into(),
            ret,
            [].into(),
            Box::new(Block(Box::new([Node(
                NodeKind::Return(Box::new([body]), false),
                span,
            )]))),
        ))
    }

    /// Consumes one closing `>` of a type argument list. `array<array<int>>`
    /// lexes both closers as a single `>>` shift token, so it is split in place
    /// and the leftover `>` stays for the enclosing list. One closer is consumed
    /// per call, which makes `array<array<array<int>>>` work out as well.
    fn close_angle(&mut self) -> Result<bool, DukaSpannedError> {
        if self.then(TokenKind::Greater)? {
            return Ok(true);
        }
        let (kind, span) = self.peek_token(0)?.clone();
        if !matches!(kind, TokenKind::ShiftR) {
            return Ok(false);
        }
        self.tokens.replace_nth(0, (TokenKind::Greater, span));
        self.tokens.rollback((TokenKind::Greater, span));
        self.next_token()?;
        self.token_index -= 1;
        Ok(true)
    }

    fn close_angle_must(&mut self) -> Result<(), DukaSpannedError> {
        if self.close_angle()? {
            return Ok(());
        }
        self.must_token(TokenKind::Greater).map(|_| ())
    }

    /// `<T, U: int = string>`, the whole list is optional
    fn opt_ty_par_def_list(&mut self) -> Result<Vec<TypeParam>, DukaSpannedError> {
        if !self.then(TokenKind::Less)? || self.close_angle()? {
            return Ok(vec![]);
        }
        let params = self.ty_par_def_list()?;
        self.close_angle_must()?;
        Ok(params)
    }

    /// `<T, U: int, V = string>`, bound and default are both optional and may
    /// be combined
    fn ty_par_def_list(&mut self) -> Result<Vec<TypeParam>, DukaSpannedError> {
        let mut res: Vec<TypeParam> = vec![];
        loop {
            // `...Ts`, a parameter that takes the rest of the type arguments as a
            // list. The `...` goes in front of the name, the same way the `...`
            // that makes a value parameter a rest parameter goes in front of the
            // name it belongs to.
            let shape = if self.then(TokenKind::Dots)? {
                ParamShape::Pack
            } else {
                ParamShape::Fixed
            };
            let name = self.must_ident()?;
            // A bound is a type position of its own: the `:` says one follows, so the
            // question of whether one was written is answered by looking for the
            // `:` rather than by consulting `type_annotations`, which describes
            // the annotation this parameter list happens to sit in.
            //
            // A bound and a default are both read with nilability off, and for
            // the same reason a return annotation is: they say what a type has to
            // be, not what a place holding a value starts out as. Reading them
            // with it on is what made `<T: [V]>` come out as `(V | nil)?` --
            // the tuple was right and the `nil` was not asked for.
            let read_type = |p: &mut Self| {
                let saved = p.config.default_nonnilable;
                p.config.default_nonnilable = true;
                let result = p.parse_type_annotation();
                p.config.default_nonnilable = saved;
                result
            };
            let bound = if self.then(TokenKind::Colon)? {
                Some(read_type(self)?)
            } else {
                None
            };
            let default = if self.then(TokenKind::Assign)? {
                Some(read_type(self)?)
            } else {
                None
            };
            res.push(TypeParam(name, bound, default, shape));
            if !self.then(TokenKind::Comma)? {
                break;
            }
        }
        Ok(res)
    }

    /// without function keyword
    ///
    /// `declared` is the `@declare` written on the function. A declaration says
    /// what exists rather than what to build, so the signature is the whole of
    /// it: there is no body to read, because the function it describes belongs
    /// to whoever provides it at runtime.
    ///
    /// The `= expr` shorthand is still read, and still reaches the checker with
    /// a body under it. Swallowing it here would turn a declaration that was
    /// written with something in it into a stray `=`, which says nothing about
    /// what the writer got wrong.
    fn function_body(&mut self, declared: bool) -> Result<FuncBody, DukaSpannedError> {
        let generics = self.opt_ty_par_def_list()?;

        let params = between!(self:
            must opt(self.par_list())[vec![]]
            in LParen, RParen
        );

        let ret = if self.config.type_annotations && self.then(TokenKind::Colon)? {
            let start = self.current_span;
            let (tys, var_arg) = self.parse_fn_returns()?;
            Some(ReturnAnnotation {
                tys,
                var_arg,
                span: start + self.current_span,
            })
        } else {
            None
        };

        let constraints = self.opt_where_clause()?;

        let body = if self.then(TokenKind::Assign)? {
            let expr = must!(self.expr())?;
            let span = expr.1;
            Block(Box::new([Node(
                NodeKind::Return(vec![expr].into_boxed_slice(), false),
                span,
            )]))
        } else if declared {
            Block::empty()
        } else {
            self.block([TokenKind::End])?
        };

        let (generics, constraints) = Self::fold_where_bounds(generics, constraints);
        Ok(FuncBody(
            params.into(),
            generics.into(),
            ret,
            constraints.into(),
            Box::new(body),
        ))
    }

    /// Folds `where T: B` into the parameter list, and drops it from the `where`.
    ///
    /// The two spellings are one statement, so the list carries every bound and
    /// the `where` keeps only what a list cannot say. A parameter that already
    /// wrote a bound in the list keeps it.
    fn fold_where_bounds(
        mut generics: Vec<TypeParam>,
        constraints: Vec<WhereClause>,
    ) -> (Vec<TypeParam>, Vec<WhereClause>) {
        let mut kept = Vec::with_capacity(constraints.len());
        for clause in constraints {
            let WhereClause::Bound((name, _), bound, _) = &clause else {
                kept.push(clause);
                continue;
            };
            let Some(param) = generics
                .iter_mut()
                .find(|TypeParam((declared, _), ..)| declared.as_str() == name.as_str())
            else {
                // a bound on a name that is not a parameter says nothing about
                // the list, and the checker already reports it
                kept.push(clause);
                continue;
            };
            if param.1.is_none() {
                param.1 = Some((**bound).clone());
            }
        }
        (generics, kept)
    }

    /// `where U: Point, Sized(T), type V = T`, the whole list is optional.
    ///
    /// `then(TokenKind::Where)` and not `then_keyword("where")`: `must_keyword`
    /// only ever matches an `Ident`, and `where` is a keyword of its own. That is
    /// the same thing that made the linq `where` clause stop matching when the
    /// token became a keyword.
    fn opt_where_clause(&mut self) -> Result<Vec<WhereClause>, DukaSpannedError> {
        if !self.then(TokenKind::Where)? {
            return Ok(vec![]);
        }
        let mut clauses = vec![];
        loop {
            clauses.push(self.where_clause()?);
            if !self.then(TokenKind::Comma)? {
                break;
            }
        }
        Ok(clauses)
    }

    fn where_clause(&mut self) -> Result<WhereClause, DukaSpannedError> {
        // `type V = T` and `type {A, B} = T`
        //
        // A bare name keeps its own shape: it takes the whole of `T`. Reading it
        // as a one-element pattern would bind it to the first part of `T` instead,
        // which is a different statement.
        if self.then(TokenKind::Type)? {
            let start = self.current_span;
            let names = match self.destruct_term()? {
                DestructingTerm::Bind(name) => Destructuring::Name(name),
                DestructingTerm::Term(d) => d,
                // a clause that binds nothing would silently bind nothing at all,
                // so it is refused rather than read as an empty pattern
                term @ (DestructingTerm::Ignored(_) | DestructingTerm::Rest(..)) => {
                    let _ = term;
                    return Err(self.err(DukaParserError::UnexpectedToken {
                        got: "_".into(),
                        expected: "a type name, or a pattern of [ ] and { }".into(),
                    }));
                }
            };
            self.must_token(TokenKind::Assign)?;
            let value = self.where_type()?;
            let end = self.current_span;
            return Ok(WhereClause::Bind(names, Box::new(value), start + end));
        }
        let start = self.current_span;
        // `U: Point` is the only clause whose left-hand side is a bare name; the
        // `:` is what tells the two apart, and a name followed by anything else is
        // a concept.
        if matches!(&self.peek_token(0)?.0, TokenKind::Ident(_))
            && matches!(&self.peek_token(1)?.0, TokenKind::Colon)
        {
            let name = must!(self.simple_name())?;
            self.must_token(TokenKind::Colon)?;
            let bound = self.where_type()?;
            let end = self.current_span;
            return Ok(WhereClause::Bound(name, Box::new(bound), start + end));
        }
        let concept = self.concept_expr()?;
        let end = self.current_span;
        Ok(WhereClause::Concept(Box::new(concept), start + end))
    }

    /// The right-hand side of a bound or a binding.
    ///
    /// Nilability off, like a type alias and a type parameter's bound: `T: Point`
    /// means `T` is a `Point`. The type grammar and nothing else -- a clause that
    /// computes says so with brackets, because `(...)` is where a type position
    /// reads an expression.
    fn where_type(&mut self) -> Result<TypeDesc, DukaSpannedError> {
        let saved = self.config.default_nonnilable;
        self.config.default_nonnilable = true;
        let result = self.parse_type_annotation();
        self.config.default_nonnilable = saved;
        result
    }

    /// A concept, which is an expression rather than a type: `T == int` computes
    /// an answer, it does not name one.
    ///
    /// Read with the typing context on, so `Sized(T)` is a type call and
    /// `array<T>` is a generic rather than a comparison. The list separator `,`
    /// is not an operator, so the reading stops at the next clause on its own --
    /// there is nothing here to rewind, which is the property that made the old
    /// two-reading parse of a type position untenable.
    fn concept_expr(&mut self) -> Result<Expr, DukaSpannedError> {
        let saved = self.typing_context;
        self.typing_context = true;
        let parsed = self.expr();
        self.typing_context = saved;
        match parsed? {
            Some(expr) => Ok(expr),
            None => Err(self.expected("a concept")),
        }
    }

    #[inline]
    fn expr(&mut self) -> TryDo<Expr, DukaSpannedError> {
        self.expr_inner(self.config.use_stmt_expr)
    }

    #[inline]
    fn expr_inner(&mut self, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        self.expr_limit(0, use_expr_stmt)
    }

    #[inline]
    fn expr_limit(&mut self, limit: u8, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        let Node(mut expr, start_span) = match self.atom_exp(use_expr_stmt)? {
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
                break Node(expr, start_span)
            }

            // consume op
            self.next_token()?;
            let Some(right) = self.expr_limit(r, use_expr_stmt)? else {
                return Err(self.expected(cpar::EXPR));
            };
            expr = NodeKind::Binary(Box::new(self.expr_end(expr, start_span)), Box::new(right), op)

        }))
    }

    fn atom_exp(&mut self, use_expr_stmt: bool) -> TryDo<Expr, DukaSpannedError> {
        let start = self.current_span;
        if self.typing_context {
            oneof! {
                if self.then(TokenKind::Type)? {
                    // `type(expr)` answers a value-context expression with the
                    // type it has -- `decltype` for DUKA. A type position reads it
                    // in `try_type_atom`; a type function body reads its answer as
                    // an expression, so the same shape has to be known here too, or
                    // `type(t)` in a body is a syntax error while `type(t)` in an
                    // annotation is not.
                    if self.then(TokenKind::LParen)? {
                        let start_span = self.current_span;
                        let expr = must!(self.expr())?;
                        let end = self.current_span;
                        self.must_token(TokenKind::RParen)?;
                        return Ok(Some(self.expr_end(
                            NodeKind::TypeLit(TypeDesc::TypeOf(
                                Box::new(expr),
                                start_span + end,
                            )),
                            start,
                        )));
                    }
                    let body = if self.then(TokenKind::Fn)? {
                        self.type_fn()?
                    } else {
                        self.must_token(TokenKind::Function)?;
                        self.type_function()?
                    };
                    return Ok(Some(self.expr_end(
                        NodeKind::TypeLit(TypeDesc::FnLit(Box::new(body))),
                        start,
                    )));
                }
                if let Some(ty) = self.try_simple_type_value()? {
                    return Ok(Some(self.expr_end(NodeKind::TypeLit(ty), start)));
                }
            }
        }

        let expr = if let Some(res) = self.prefix_exp()? {
            Some(res)
        } else {
            let (tk, start_span) = self.span_start()?;
            let kind = oneof!(
                try match tk =>
                TokenKind::If if use_expr_stmt => {
                    self.next_token()?;
                    NodeKind::If(self.if_block(true)?)
                }
                TokenKind::Match if use_expr_stmt => {
                    self.next_token()?;
                    NodeKind::Match(self.match_block(true)?)
                }
                TokenKind::Do if use_expr_stmt => {
                    self.next_token()?;
                    let block = self.block([TokenKind::End])?;
                    NodeKind::Do(block.into(), false, None)
                }
                TokenKind::Nil => {
                    self.next_token()?;
                    NodeKind::Literal(ConstValue::Nil)
                }
                TokenKind::True => {
                    self.next_token()?;
                    NodeKind::Literal(ConstValue::Bool(true))
                }
                TokenKind::False => {
                    self.next_token()?;
                    NodeKind::Literal(ConstValue::Bool(false))
                }
                TokenKind::Float(f) => {
                    let k = NodeKind::Literal(ConstValue::Float(*f));
                    self.next_token()?;
                    k
                }
                TokenKind::Int(i) => {
                    let k = NodeKind::Literal(ConstValue::Int(*i));
                    self.next_token()?;
                    k
                }
                TokenKind::String(..) => {
                    let TokenKind::String(v) = self.next_token()?.0 else {
                        unreachable!()
                    };

                    NodeKind::Literal(
                        ConstValue::String(v)
                    )
                }
                TokenKind::Dots => {
                    self.next_token()?;
                    NodeKind::VarArg
                }
                TokenKind::LBrace => must!(self.table_constructor())?,
                TokenKind::LBracket => must!(self.array_constructor())?,
                TokenKind::Function => {
                    self.next_token()?;
                    NodeKind::Function(self.function_body(false)?)
                }
                TokenKind::Fn => {
                    self.next_token()?;
                    NodeKind::Function(self.fn_body()?)
                }
                t if t.is_unop() => self.unop_exp()?
            );
            Some(self.expr_end(kind, start_span))
        };
        match expr {
            Some(expr)
                if self.peek_token(0)?.0 == TokenKind::Bang
                    && self.peek_token(1)?.0 == TokenKind::Do =>
            {
                self.next_token()?;
                self.next_token()?;

                let body = self.block([TokenKind::End])?;

                Ok(Some(self.expr_end(
                    NodeKind::BangDo(BangDoNode {
                        context: Box::new(expr),
                        body: Box::new(body),
                    }),
                    start,
                )))
            }
            _ => Ok(expr),
        }
    }

    fn unop_exp(&mut self) -> Result<NodeKind, DukaSpannedError> {
        let tk = self.next_token()?.0;
        Ok(NodeKind::Unary(
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
            TokenKind::LBracket => {
                let array = must!(self.array_constructor())?;
                vec![self.expr_end(array, start_span)]
            }
            TokenKind::LBrace => {
                let table = must!(self.table_constructor())?;
                vec![self.expr_end(table, start_span)]
            }
            TokenKind::String(..) => {
                let TokenKind::String(val) = self.next_token()?.0 else {
                    unreachable!()
                };
                let str = NodeKind::Literal(ConstValue::String(val));
                vec![self.expr_end(str, start_span)]
            }
        )))
    }

    /// already checked
    fn array_constructor(&mut self) -> TryDo<NodeKind, DukaSpannedError> {
        self.next_token()?; //already checked
        let mut items = vec![];
        if !self.then(TokenKind::RBracket)? {
            opt![if let Some(e) = self.expr()? {
                items.push(e);
            }];
            many! {
                self then Comma or SemiColon:
                if self.lookahead_token(TokenKind::RBracket, 0)? {
                    break;
                } else {
                    let e = must!(self.expr())?;
                    items.push(e);
                }
            }
            self.must_token(TokenKind::RBracket)?;
        }
        Ok(Some(NodeKind::Array(items.into())))
    }

    /// already checked
    fn table_constructor(&mut self) -> TryDo<NodeKind, DukaSpannedError> {
        self.next_token()?; // already checked
        let mut fields = vec![];

        if !self.then(TokenKind::RBrace)? {
            opt![if let Some(f) = self.field()? {
                fields.push(f);
            }];

            many! {
                self then Comma or SemiColon:
                // {...,}
                if self.lookahead_token(TokenKind::RBrace, 0)? {
                    break
                } else {
                    let f = must!(self.field())?;
                    fields.push(f)
                }
            }

            self.must_token(TokenKind::RBrace)?;
        }

        Ok(Some(NodeKind::Table(fields.into())))
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
            by Comma separate (self.var_attr_name())
            nonempty
        ))
    }

    fn expr_list(&mut self) -> TryDo<Vec<Expr>, DukaSpannedError> {
        Ok(list!(self:
            by Comma separate (self.expr())
            empty[None]
        ))
    }

    /// A type position.
    ///
    /// This reads the type shapes and nothing else, and it reads them once. The
    /// shapes are `array<int>`, `a | b`, `{k: T}`, `Opt(int)`, `type fn(t) t`
    /// and a bare name, which is everything an annotation can be written in.
    ///
    /// It deliberately does not also try an expression. Every position that
    /// calls this one is closed by something the expression grammar reads as an
    /// operator rather than as an end -- `>` closing a type argument list, `:`
    /// closing a parameter's name, `,` and `)` closing a list -- so an
    /// expression reading here swallows the delimiter and then asks for an
    /// operand that is not there. The earlier version papered over that by
    /// reading the type grammar, failing, and rewinding to read an expression
    /// instead, which needed the token stream to be rewound; where it could not
    /// be, the error reported was the second reading's and said nothing about
    /// the real cause, and `array<type fn(t) t>` looped forever.
    ///
    /// A type position that really wants an expression -- a `where` predicate --
    /// asks for one, and holds the result as a `TypeDesc::Expr`.
    #[inline(always)]
    fn parse_type_annotation(&mut self) -> Result<TypeDesc, DukaSpannedError> {
        self.parse_annotated(0)
    }

    /// Simple type value, (no type function)
    fn try_simple_type_value(&mut self) -> Result<Option<TypeDesc>, DukaSpannedError> {
        let starts_type = match &self.peek_token(0)?.0 {
            TokenKind::Ident(_)
            | TokenKind::String(_)
            | TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Nil
            | TokenKind::LParen
            // `[a, b]` is a tuple type and `{k: v}` a record type here; left out of this
            // list they were read as array and table literals instead.
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::Function
            | TokenKind::Fn => true,
            _ => return Ok(None),
        };
        if !starts_type {
            return Ok(None);
        }
        // In the body of a `type function`, `f(x)` is a call and not a type, and
        // reading it as a type would turn every call into a type function
        // application. An annotation is read by `parse_annotated` directly and
        // never comes through here, so this is only ever about a body.
        if self.typing_context
            && matches!(&self.peek_token(0)?.0, TokenKind::Ident(name) if name != "func")
            && matches!(&self.peek_token(1)?.0, TokenKind::LParen)
        {
            return Ok(None);
        }
        // The same question for a braced shape, and the same answer: `{k = v}` is a table
        // and not a record, so it is a value and not a type. The `=` is what tells
        // them apart. A table written where a type is meant needs `()` around it.
        if self.typing_context
            && matches!(&self.peek_token(0)?.0, TokenKind::LBrace)
            && matches!(&self.peek_token(2)?.0, TokenKind::Assign)
        {
            return Ok(None);
        }
        let saved = self.config.default_nonnilable;
        self.config.default_nonnilable = true;
        let result = self.parse_annotated(0);
        self.config.default_nonnilable = saved;
        result.map(Some)
    }

    #[inline(always)]
    /// Parse a type annotation.
    /// Each atom is nullable by default (depend on config); `!` strips its `| nil`, `?` adds it.
    fn parse_annotated(&mut self, limit: u8) -> Result<TypeDesc, DukaSpannedError> {
        let mut atom = must!(self.try_type_atom())?;
        Ok(many! {
            loop:
            let (tk, _) = self.peek_token(0)?;

            if !tk.is_typeop() {
                break atom
            }

            if let Some((op, (l, r))) = get_typeop_info(tk)
            {
                if l <= limit {
                    break atom
                }

                // consume op
                self.next_token()?;
                let right = self.parse_annotated(r)?;
                match op {
                    TypeOp::Intersect => atom = atom.intersect(right),
                    TypeOp::Union => atom = atom.union(right)
                }
            } else {
                return Err(
                    DukaSpannedError::new(DukaParserError::UnknownOperator(tk.name().into()).into(),  self.current_span,  self.source_info.clone())
                )
            }
        })
    }

    /// without '<', nonempty, '>' is consumed
    fn ty_par_list(&mut self) -> Result<Vec<TypeDesc>, DukaSpannedError> {
        let mut args = vec![];
        let saved_args = self.in_type_args;
        let saved_nil = self.config.default_nonnilable;
        self.in_type_args = true;
        self.config.default_nonnilable = true;
        let result = {
            loop {
                // The shape parser, not `parse_type_annotation`, which is the
                // same reader; the difference is the flag above.
                args.push(self.parse_annotated(0)?);
                if self.close_angle()? {
                    break;
                }
                self.must_token(TokenKind::Comma)?;
            }
            Ok(args)
        };
        self.in_type_args = saved_args;
        self.config.default_nonnilable = saved_nil;
        result
    }

    fn try_type_atom(&mut self) -> Result<Option<TypeDesc>, DukaSpannedError> {
        if self.then(TokenKind::Type)? {
            if self.then(TokenKind::LParen)? {
                let start_span = self.current_span;
                let expr = must!(self.expr())?;
                let end = self.current_span;
                self.must_token(TokenKind::RParen)?;
                return self.finish_member(TypeDesc::TypeOf(Box::new(expr), start_span + end));
            }
            let body = if self.then(TokenKind::Fn)? {
                self.type_fn()?
            } else if self.then(TokenKind::Function)? {
                self.type_function()?
            } else {
                let got = self.peek_token(0)?.0.name().into();
                return Err(self.err(DukaParserError::UnexpectedToken {
                    got,
                    expected: "fn, function, or ( after type".into(),
                }));
            };
            return self.finish_member(TypeDesc::FnLit(Box::new(body)));
        }

        // oneof
        let keywordish = match &self.peek_token(0)?.0 {
            TokenKind::Ident(name) => Some(name.clone()),
            _ => None,
        };

        // array list table, !!NOITCE `<` !!
        //
        // In the body of a `type function`, `f < x` is a comparison and not a
        // generic application, so only the shapes that cannot be anything else
        // are read as one. An annotation is read with `typing_context` off, and
        // there `Box<int>` is a type and there is nothing else it could be.
        //
        // A bare `<>` stays the way a generic type is applied -- `where type V = T`
        // is written `pick<int>`, and forcing a dot on that would read worse than
        // what it saves. `.name<...>` is the form for a member of something else,
        // where `m.pick<int>` could otherwise be `pick` reached from `m` or a
        // comparison about it.
        if self.config.type_annotations
            && keywordish.is_some()
            && matches!(&self.peek_token(1)?.0, TokenKind::Less)
            && (!self.typing_context
                || matches!(keywordish.as_deref(), Some("array" | "list" | "table")))
        {
            let (name, name_span) = match self.peek_token(0)? {
                (TokenKind::Ident(name), span) => (name.clone(), *span),
                _ => unreachable!(),
            };

            self.next_token()?; // ident
            self.next_token()?; // less <
            let mut args = self.ty_par_list()?;

            let ty = match name.as_str() {
                "array" | "list" if args.len() == 1 => TypeDesc::array_of(args.pop()),
                "table" if args.len() == 2 => {
                    let v = args.pop().unwrap();
                    let k = args.pop().unwrap();
                    TypeDesc::table_of(Some(k), Some(v))
                }
                _ => TypeDesc::Generic(name.into_boxed_str(), args.into(), name_span),
            };
            return self.finish_member(ty);
        }

        // function
        let is_func = matches!(
            &self.peek_token(0)?.0,
            TokenKind::Ident(name) if name.as_str() == "func"
        ) || matches!(&self.peek_token(0)?.0, TokenKind::Function | TokenKind::Fn);
        if is_func {
            self.next_token()?;
            if !self.then(TokenKind::LParen)? {
                return self.finish_member(TypeDesc::Pure(Type::Function(None)));
            }
            let (params, var_arg) = self.parse_fn_params()?;
            let (returns, return_var_arg) = if self.then(TokenKind::Arrow)? {
                self.parse_fn_returns()?
            } else {
                ([].into(), false)
            };
            return self.finish_member(TypeDesc::function_of(Some(TypeFnValue {
                params,
                var_arg,
                returns,
                return_var_arg,
            })));
        }

        // keywordish ident to type
        if let Some(ty) = self.try_parse_basic_type()? {
            return self.finish_member(TypeDesc::Pure(ty));
        }

        // literal type
        if self.config.type_annotations {
            oneof! {
                let tk = &self.peek_token(0)?.0;

                if matches!(tk, TokenKind::LBracket) {
                    let items = between!(self:
                        must opt({
                            Ok(list!(self:
                                by Comma separate (self.parse_type_annotation())
                                nonempty
                            ))
                        })[vec![]]
                        in LBracket, RBracket
                    );
                    return self.finish_member(TypeDesc::tuple_of(items.into_boxed_slice()))
                }
                else if matches!(tk, TokenKind::LBrace) {
                    // `{k = v}` is a table and `{k: v}` is a record, and the `=` is
                    // what tells them apart. A table is a value, so it is read as
                    // an expression and held as one -- no brackets needed, because
                    // the shape already says which it is.
                    if matches!(&self.peek_token(2)?.0, TokenKind::Assign) {
                        let start_span = self.current_span;
                        let Node(kind, end) = must!(self.expr())?;
                        return self.finish_member(TypeDesc::Expr(Box::new(Node(
                            kind,
                            start_span + end,
                        ))));
                    }
                    let pairs = between!(self:
                        must opt({
                            Ok(list!(self:
                                by Comma separate ({
                                    let (name, span) = self.must_ident()?;
                                    self.must_token(TokenKind::Colon)?;
                                    let ty = self.parse_type_annotation()?;
                                    Ok(TypeField::new(name.into_boxed_str(), span, ty))
                                })
                                nonempty
                            ))
                        })[vec![]]
                        in LBrace, RBrace
                    );
                    return self.finish_member(TypeDesc::typetable_of(pairs.into_boxed_slice()))
                }
                else if matches!(tk, TokenKind::Minus) {
                    if let TokenKind::Int(i) = &self.peek_token(1)?.0 {
                        let i = -*i;
                        self.next_token()?;
                        self.next_token()?;
                        return self.finish_member(TypeDesc::Pure(Type::Literal(
                            ConstValue::Int(i)
                        )));
                    }
                }
                else if let Some(lit) = match tk {
                    TokenKind::String(s) => Some(ConstValue::String(s.clone())),
                    TokenKind::Int(i) => Some(ConstValue::Int(*i)),
                    TokenKind::Float(f) => Some(ConstValue::Float(*f)),
                    TokenKind::True => Some(ConstValue::Bool(true)),
                    TokenKind::False => Some(ConstValue::Bool(false)),
                    TokenKind::Nil => Some(ConstValue::Nil),
                    _ => None,
                } {
                    self.next_token()?;
                    return self.finish_member(TypeDesc::Pure(Type::Literal(lit)));
                }
            }
        }

        // `(` `)`
        //
        // The brackets are the escape. Inside them a type position is read as an
        // expression: `(T)` is the type `T`, `(T + 1)` is a computation. Outside
        // them nothing changes, so `C + 1` without brackets is still rejected by
        // the type grammar -- one reading for a type, one way to ask for the other.
        if self.then(TokenKind::LParen)? {
            let saved_typing = self.typing_context;
            let saved_nil = self.config.default_nonnilable;
            self.typing_context = true;
            self.config.default_nonnilable = true;
            let Node(kind, inner) = must!(self.expr())?;
            self.typing_context = saved_typing;
            self.config.default_nonnilable = saved_nil;
            let group = match kind {
                NodeKind::TypeLit(td) => td,
                other => TypeDesc::Expr(Box::new(Node(other, inner))),
            };
            self.must_token(TokenKind::RParen)?;
            return self.finish_member(group);
        }

        // try for TypeCall
        let (name, name_span) = match self.peek_token(0)? {
            (TokenKind::Ident(name), span) => (name.clone(), *span),
            _ => return Ok(None),
        };
        self.next_token()?;

        if self.config.type_annotations && self.peek_token(0)?.0 == TokenKind::LParen {
            let saved = self.config.default_nonnilable;
            self.config.default_nonnilable = true;
            let result = {
                let args = between!(self:
                    must opt({
                        Ok(list!(self:
                            by Comma separate (self.parse_type_annotation())
                            nonempty
                        ))
                    })[vec![]]
                    in LParen, RParen
                );
                Ok(Some(type_application(
                    name.clone().into_boxed_str(),
                    args.into(),
                    name_span,
                )))
            }?;
            self.config.default_nonnilable = saved;
            if let Some(ty) = result {
                return self.finish_member(ty);
            }
        }
        self.finish_member(TypeDesc::Named(name.into_boxed_str(), name_span))
    }

    /// Apply the `!` / `?` postfix (at the atom level), or the default
    /// nullability from the config when neither suffix is present.
    /// One `.name`, read as far past the name as it goes.
    ///
    /// `m.f(a)` applies something and `m.Pair<int, string>` says what a class was
    /// applied to; the arguments of the second belong to the name in front of them,
    /// so they stay on the path rather than becoming a call.
    fn step_member(
        &mut self,
        base: &TypeDesc,
        member: TypeDesc,
        span: Span,
    ) -> Result<TypeDesc, DukaSpannedError> {
        let named = matches!(
            &member,
            TypeDesc::Pure(Type::Literal(ConstValue::String(_)))
        );
        if named && self.config.type_annotations && self.then(TokenKind::Less)? {
            let saved = self.config.default_nonnilable;
            self.config.default_nonnilable = true;
            // `ty_par_list` closes the list itself, which is why nothing is read
            // after it here: closing a second time finds whatever follows the
            // `>` -- the `=` of a binding -- and asks for another `>`.
            let args = self.ty_par_list()?;
            self.config.default_nonnilable = saved;
            let TypeDesc::Pure(Type::Literal(ConstValue::String(name))) = &member else {
                unreachable!("`named` said so")
            };
            let name = String::from_utf8_lossy(name).into_owned();
            let base = crate::parser::ast::path_of(base, span);
            let path = Path::Chain(
                Box::new(Path::Chain(Box::new(base), PathSuffix::Dot((name, span)))),
                PathSuffix::TypeArgs(args.into(), span),
            );
            return Ok(TypeDesc::Expr(Box::new(Node(
                NodeKind::Access(Box::new(path)),
                span,
            ))));
        }
        let args = if self.config.type_annotations && self.peek_token(0)?.0 == TokenKind::LParen {
            let saved = self.config.default_nonnilable;
            self.config.default_nonnilable = true;
            let args = between!(self:
                must opt({
                    Ok(list!(self:
                        by Comma separate (self.parse_type_annotation())
                        nonempty
                    ))
                })[vec![]]
                in LParen, RParen
            );
            self.config.default_nonnilable = saved;
            Some(args)
        } else {
            None
        };
        Ok(type_access(
            base.clone(),
            member,
            args.map(|a| a.into()),
            span,
        ))
    }

    fn finish_member(&mut self, ty: TypeDesc) -> Result<Option<TypeDesc>, DukaSpannedError> {
        let mut ty = ty;
        loop {
            oneof! {
                let (member, span) = if self.then(TokenKind::Dot)? {
                    let (member, member_span) = match self.peek_token(0)? {
                        (TokenKind::Ident(name), span) => (name.clone(), *span),
                        _ => {
                            let (tok, _) = self.peek_token(0)?;
                            let got: Box<str> = tok.name().into();
                            return Err(self.err(DukaParserError::UnexpectedToken {
                                got,
                                expected: "identifier".into(),
                            }));
                        }
                    };
                    self.next_token()?;
                    (TypeDesc::Pure(Type::Literal(ConstValue::String(member.into_bytes().into_boxed_slice()))), member_span)
                }
                else if self.then(TokenKind::LBracket)? {
                    let start = self.current_span;
                    let idx = must!(self.try_simple_type_value())?;
                    let end = self.current_span;
                    self.must_token(TokenKind::RBracket)?;
                    (idx, start + end)
                }
                else {
                    break
                };
                ty = self.step_member(&ty, member, span)?;
            }
        }
        Ok(Some(oneof! {
            if self.then(TokenKind::Bang)? {
                ty.nonnilable()
            } else if self.then(TokenKind::Question)? {
                ty.nilable()
            }             else if matches!(
                ty,
                TypeDesc::Pure(Type::Literal(_))
                    | TypeDesc::Expr(_)
                    | TypeDesc::TypeOf(..)
            ) || self.config.default_nonnilable
            {
                ty
            } else {
                ty.nilable()
            }
        }))
    }

    /// `->` is consumed
    /// returns `(returns, return_var_arg)`
    fn parse_fn_returns(&mut self) -> Result<(Box<[TypeDesc]>, bool), DukaSpannedError> {
        let saved = self.config.default_nonnilable;
        self.config.default_nonnilable = true;
        let result = self.parse_fn_returns_inner();
        self.config.default_nonnilable = saved;
        result
    }

    fn parse_fn_returns_inner(&mut self) -> Result<(Box<[TypeDesc]>, bool), DukaSpannedError> {
        if self.then(TokenKind::Dots)? {
            return Ok(([].into(), true));
        }
        if !self.then(TokenKind::LParen)? {
            return Ok(([self.parse_type_annotation()?].into(), false));
        }
        let mut returns = vec![];
        if self.then(TokenKind::RParen)? {
            return Ok(([].into(), false));
        }
        if self.then(TokenKind::Dots)? {
            self.must_token(TokenKind::RParen)?;
            return Ok(([].into(), true));
        }
        loop {
            returns.push(self.parse_type_annotation()?);
            if !self.then(TokenKind::Comma)? {
                break;
            }
            if self.then(TokenKind::Dots)? {
                self.must_token(TokenKind::RParen)?;
                return Ok((returns.into(), true));
            }
        }
        self.must_token(TokenKind::RParen)?;
        Ok((returns.into(), false))
    }

    /// `(` is consumed
    /// returns `(params, var_arg)`
    fn parse_fn_params(&mut self) -> Result<(Box<[TypeDesc]>, bool), DukaSpannedError> {
        let mut params = vec![];
        if self.peek_token(0)?.0 == TokenKind::RParen {
            self.next_token()?;
            return Ok(([].into(), false));
        }
        if self.then(TokenKind::Dots)? {
            self.must_token(TokenKind::RParen)?;
            return Ok(([].into(), true));
        }
        loop {
            params.push(self.parse_type_annotation()?);
            if !self.then(TokenKind::Comma)? {
                break;
            }
            if self.then(TokenKind::Dots)? {
                self.must_token(TokenKind::RParen)?;
                return Ok((params.into(), true));
            }
        }
        self.must_token(TokenKind::RParen)?;
        Ok((params.into(), false))
    }

    fn try_parse_basic_type(&mut self) -> Result<Option<Type>, DukaSpannedError> {
        match &self.peek_token(0)?.0 {
            TokenKind::Ident(name) => {
                let Some(ty) = Type::from_keyword(name.as_str()) else {
                    return Ok(None);
                };
                self.next_token()?;
                Ok(Some(ty))
            }
            TokenKind::Nil => {
                self.next_token()?;
                Ok(Some(Type::Nil))
            }
            _ => Ok(None),
        }
    }

    // without `<` `>`
    fn par_list(&mut self) -> Result<Vec<Param>, DukaSpannedError> {
        let parse_param = |me: &mut Self| {
            let name = me.must_ident()?;
            if me.config.type_annotations && me.then(TokenKind::Colon)? {
                Ok(Param::Typed(name, me.parse_type_annotation()?))
            } else {
                Ok(Param::Name(name))
            }
        };
        Ok(if self.then(TokenKind::Dots)? {
            // `(...)` 单独作为唯一参数
            vec![Param::Var(self.current_span)]
        } else {
            let mut res: Vec<Param> = vec![];
            res.push(parse_param(self)?);
            while self.then(TokenKind::Comma)? {
                if self.then(TokenKind::Dots)? {
                    res.push(Param::Var(self.current_span));
                    break;
                }
                res.push(parse_param(self)?);
            }
            res
        })
    }
    fn type_fn_params(&mut self) -> Result<Box<[Param]>, DukaSpannedError> {
        let parse_param = |p: &mut Self| {
            let name = p.must_ident()?;
            if p.config.type_annotations && p.then(TokenKind::Colon)? {
                Ok(Param::Typed(name, p.parse_type_annotation()?))
            } else {
                Ok(Param::Name(name))
            }
        };
        let mut res: Vec<Param> = vec![];
        res.push(parse_param(self)?);
        while self.then(TokenKind::Comma)? {
            res.push(parse_param(self)?);
        }
        Ok(res.into_boxed_slice())
    }

    // external
}

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
    pub fn span_end<V>(&self, val: V, start: Span) -> Spanned<V> {
        (val, start + self.current_span)
    }
}

impl Parser<Token> {
    #[inline(always)]
    fn check_end(&mut self) -> Result<bool, DukaSpannedError> {
        self.peek_token(0).map(|t| t.0.is_terminator())
    }

    #[inline(always)]
    fn stmt_end(&self, val: NodeKind, start: Span) -> Stmt {
        Node(val, start + self.current_span)
    }
    #[inline(always)]
    fn expr_end(&self, val: NodeKind, start: Span) -> Expr {
        Node(val, start + self.current_span)
    }
    #[inline(always)]
    fn span_start(&mut self) -> Result<RefToken<'_>, DukaSpannedError> {
        let (tk, sp) = self.peek_token(0)?;
        Ok((tk, *sp))
    }

    fn must_keyword(&mut self, kw: &str) -> Result<(), DukaSpannedError> {
        let (tk, sp) = self.next_token()?;
        let TokenKind::Ident(id) = tk else {
            return Err(if tk.is_terminator() {
                DukaSpannedError::new(
                    DukaParserError::UnexpectedEnd(kw.into()).into(),
                    sp,
                    self.source_info.clone(),
                )
            } else {
                self.expected(kw)
            });
        };
        (id == kw).then_some(()).ok_or_else(|| self.expected(kw))
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

    #[inline]
    fn no_more(&mut self) -> Result<(), DukaSpannedError> {
        (!self.check_end()?).then_error(|| {
            DukaSpannedError::new(
                DukaParserError::ShouldBeEnd.into(),
                self.next_token().unwrap().1,
                self.source_info.clone(),
            )
        })?;
        Ok(())
    }
    #[inline]
    fn expected(&mut self, expected: &str) -> DukaSpannedError {
        let cur = self.current_span;
        let (error_at, span) = self.peek_token(0).unwrap();
        DukaSpannedError::new(
            if !error_at.is_terminator() {
                DukaParserError::UnexpectedToken {
                    got: error_at.stringify().to_string().into_boxed_str(),
                    expected: expected.into(),
                }
            } else {
                DukaParserError::UnexpectedEnd(expected.into())
            }
            .into(),
            // same, im sure this won't be a panic when I call it
            if error_at.is_terminator() { cur } else { *span },
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

    /// NO CONTEXTUAL KEYWORD
    #[inline(always)]
    fn must_ident(&mut self) -> Result<Spanned<String>, DukaSpannedError> {
        match self.peek_token(0)? {
            (TokenKind::Ident(..), _) => {
                let (TokenKind::Ident(ident), span) = self.next_token()? else {
                    unreachable!()
                };
                Ok((ident, span))
            }
            (tk, span) => {
                let s = *span;
                Err(if tk.is_terminator() {
                    DukaSpannedError::new(
                        DukaParserError::UnexpectedEnd(clex::ID.into()).into(),
                        s,
                        self.source_info.clone(),
                    )
                } else {
                    self.expected(clex::ID)
                })
            }
        }
    }

    #[inline(always)]
    fn must_attr_name(&mut self) -> Result<Spanned<String>, DukaSpannedError> {
        match self.peek_token(0)? {
            (token, span) if token.is_keyword() && token.is_contextual() => {
                let (name, s) = (token.name().to_owned(), *span);
                let _ = self.next_token()?;
                Ok((name, s))
            }
            _ => self.must_ident(),
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

    /// The `n`th token that is not a comment.
    ///
    /// Counted inside the look-ahead rather than reached by consuming: a `next` taken
    /// while a comment sat in the window takes the *current* token instead, and the
    /// parse then fails somewhere later than the mistake.
    #[inline]
    fn peek_token(&mut self, n: usize) -> Result<&Token, DukaSpannedError> {
        // counted over the buffer rather than reached by consuming: a `next`
        // taken while a comment sat in the window would take the *current*
        // token instead of the comment, shifting everything after it by one --
        // and the parse would then fail somewhere later than the mistake.
        let mut index = 0;
        let mut seen = 0;
        while seen < n {
            match self.tokens.peek_nth(index) {
                None => break,
                Some(token) if token.0.is_comment() => {}
                Some(_) => seen += 1,
            }
            index += 1;
        }
        while self
            .tokens
            .peek_nth(index)
            .is_some_and(|token| token.0.is_comment())
        {
            index += 1;
        }
        Ok(self.tokens.peek_nth(index).unwrap_or(&EMPTY_TOKEN))
    }

    #[inline]
    fn next_token(&mut self) -> Result<Token, DukaSpannedError> {
        loop {
            let token = self
                .tokens
                .next()
                .inspect(|t| {
                    self.token_index += 1;
                    self.current_span = t.1;
                })
                .unwrap_or((TokenKind::EOF, self.current_span));
            if let TokenKind::DocComment(text) = &token.0 {
                // kept while the run goes on, so that several `---` lines are one
                // document rather than three that each overwrite the last
                // the line it starts on, not the one it ends on: a line comment's span runs
                // to the next line because it takes the newline with it
                self.pending_doc.push((text.clone(), token.1.start.line));
                continue;
            }
            if !token.0.is_comment() {
                // Whatever the run was above, it is above *this* token. Recorded
                // against the token rather than the statement, because the parser
                // has not reached the statement yet and a nested block may read a
                // run of its own on the way there.
                let run = std::mem::take(&mut self.pending_doc);
                if let Some(doc) = doc_text(run, token.1.start.line) {
                    self.doc_at_token.insert(token.1, doc);
                }
                break Ok(token);
            }
        }
    }
}

/// The span of the name a path was written with, which is the span a symbol for
/// it is found by.
fn name_span(path: &Path) -> Option<Span> {
    match path {
        Path::Base((_, span)) => Some(*span),
        _ => None,
    }
}

/// The spans of the names a statement introduces, which is what a document is
/// keyed by.
///
/// A reader asks about a name, and a symbol is found by the span of its own name
/// rather than by the statement it was written in -- so a document reaches a
/// symbol by being recorded under every name the statement declared.
fn declared_names(stmt: &Stmt) -> Vec<Span> {
    match &stmt.0 {
        NodeKind::FunctionDecl(path, _, _, _) => name_span(path).into_iter().collect(),
        NodeKind::Define(names, _, _, _) => {
            names.iter().map(|(((_, span), _, _), _)| *span).collect()
        }
        NodeKind::TypeAlias((_, span), _) | NodeKind::TypeFunction((_, span), _) => vec![*span],
        NodeKind::Destructing(names, _, _) => crate::analyzer::destructuring::names(names)
            .into_iter()
            .map(|(_, span)| span)
            .collect(),
        _ => vec![stmt.1],
    }
}

/// The prose of a `---` run, when it documents the statement that starts on
/// `at_line`.
///
/// A blank line ends a document. There is no token for one, so the lines the run
/// covers are what says: a run whose last line is the line above the declaration
/// is about it, and a run further away is a remark that happened to be near.
fn doc_text(run: Vec<(String, u32)>, at_line: u32) -> Option<String> {
    let last = run.last()?;
    if last.1 + 1 != at_line {
        return None;
    }
    let text = run
        .into_iter()
        .map(|(line, _)| line)
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

impl DukaParser<Token> for Parser<Token> {
    type ChunkType = DukaChunk;

    fn parse(
        stream: TokenStream<Token>,
        config: DukaParserConfig,
    ) -> Result<Self::ChunkType, DukaSpannedError> {
        Self::new(stream, config).parse_into_chunk()
    }
}

impl Parser<Token> {
    /// Reads a chunk out of a parser that is already set up, so a caller that
    /// had to register handlers first can still finish the job.
    pub fn parse_into_chunk(mut self) -> Result<DukaChunk, DukaSpannedError> {
        let start_span = self.current_span;
        let chunk = self.parse_chunk()?;
        let span = start_span + self.current_span;
        Ok(DukaChunk {
            block: chunk,
            span,
            logic: Box::new(self.logic),
            source_info: self.source_info,
            docs: self.docs,
        })
    }
}

const MAX_PARSE_ERRORS: usize = 50;

impl Parser<Token> {
    pub fn parse_lenient(
        stream: TokenStream<Token>,
        config: DukaParserConfig,
    ) -> (DukaChunk, Vec<DukaSpannedError>) {
        let mut parser = Self::new(stream, config);
        let start_span = parser.current_span;
        let mut errors = vec![];
        let block = parser.block_inner_recovery(
            &[TokenKind::terminator()],
            &[],
            &mut errors,
            MAX_PARSE_ERRORS,
            None,
        );
        match block {
            Some(block) => {
                let chunk = DukaChunk {
                    block,
                    span: start_span + parser.current_span,
                    logic: Box::new(parser.logic),
                    source_info: parser.source_info.clone(),
                    docs: parser.docs,
                };
                (chunk, errors)
            }
            None => {
                let chunk = DukaChunk {
                    block: Block::empty(),
                    span: start_span,
                    logic: Box::new(parser.logic),
                    source_info: parser.source_info.clone(),
                    docs: parser.docs,
                };
                (chunk, errors)
            }
        }
    }

    pub fn parse_lenient_incremental(
        stream: TokenStream<Token>,
        config: DukaParserConfig,
        previous: Option<IncrementalParseCache>,
        edit_at_char: Option<u32>,
    ) -> (DukaChunk, Vec<DukaSpannedError>, IncrementalParseCache) {
        let config_snapshot = config.clone();
        let start_index = previous
            .as_ref()
            .filter(|cache| {
                cache.safe
                    && cache.checkpoints.len() == cache.nodes.len() + 1
                    && cache.config == config
            })
            .map(|cache| {
                edit_at_char.map_or(cache.nodes.len(), |at_char| {
                    cache
                        .nodes
                        .iter()
                        .position(|node| {
                            node.1.start.at_char <= at_char && at_char <= node.1.end.at_char
                        })
                        .or_else(|| {
                            cache
                                .nodes
                                .iter()
                                .position(|node| node.1.start.at_char >= at_char)
                        })
                        .unwrap_or_else(|| cache.nodes.len().saturating_sub(1))
                })
            });
        let mut checkpoints = start_index
            .and_then(|index| {
                previous
                    .as_ref()
                    .map(|cache| cache.checkpoints[..index].to_vec())
            })
            .unwrap_or_default();
        let mut prefix = vec![];
        let (mut parser, resumed) = if let (Some(index), Some(previous)) = (start_index, previous) {
            let checkpoint = previous.checkpoints[index].clone();
            let tokens = stream.from_index(checkpoint.token_index);
            let mut parser = Self::from_tokens(tokens, config);
            parser.token_index = checkpoint.token_index;
            parser.current_span = checkpoint.current_span;
            let (facts, rules, queries) = checkpoint.logic_lengths;
            parser.docs = previous.docs;
            parser
                .docs
                .retain(|span, _| span.start.at_char <= checkpoint.current_span.end.at_char);
            parser.logic = LogicDatabase {
                facts: previous.logic.facts[..facts].to_vec(),
                rules: previous.logic.rules[..rules].to_vec(),
                queries: previous.logic.queries.to_slice()[..queries].to_vec().into(),
            };
            parser.typing_context = checkpoint.typing_context;
            parser.in_type_args = checkpoint.in_type_args;
            prefix.extend(previous.nodes.into_iter().take(index));
            (parser, true)
        } else {
            (Self::new(stream, config), false)
        };
        let start_span = parser.current_span;
        let mut errors = vec![];
        let block = parser.block_inner_recovery(
            &[TokenKind::terminator()],
            &[],
            &mut errors,
            MAX_PARSE_ERRORS,
            Some(&mut checkpoints),
        );
        let (block, span) = if let Some(block) = block {
            prefix.extend(block.0);
            (
                Block(prefix.clone().into()),
                if resumed {
                    Span::EMPTY + parser.current_span
                } else {
                    start_span + parser.current_span
                },
            )
        } else {
            prefix.clear();
            (
                Block::empty(),
                if resumed { Span::EMPTY } else { start_span },
            )
        };
        let chunk = DukaChunk {
            block,
            span,
            logic: Box::new(parser.logic),
            source_info: parser.source_info.clone(),
            docs: parser.docs,
        };
        let cache = IncrementalParseCache {
            nodes: prefix,
            safe: errors.is_empty() && checkpoints.len() == chunk.block.0.len() + 1,
            checkpoints,
            docs: chunk.docs.clone(),
            logic: (*chunk.logic).clone(),
            config: config_snapshot,
        };
        (chunk, errors, cache)
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

    fn expected(&mut self, expected: &str) -> DukaSpannedError {
        self.inner.expected(expected)
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
