//! The Duka language server backend.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use duka_frontend::lexer::token::TokenKind;
use duka_shared::{dtype::Type, errors::Span, utils::SymbolType};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::{
    compile, convert, docs,
    workspace::{SharedWorkspace, Workspace, lock},
};

/// Upper bound on the items a single completion request returns. Hitting it
/// flips the response to `isIncomplete`, so the client asks again as the user
/// narrows the prefix instead of us shipping every symbol in scope.
const MAX_COMPLETION_ITEMS: usize = 1000;

/// How long diagnostics wait for the typing to pause. A change invalidates the
/// snapshot immediately, so a hover still sees the text as it is, but the
/// pipeline only runs once the burst is over: analysing on every keystroke is
/// the difference between a server that keeps up and one that does not.
const DIAGNOSTIC_DEBOUNCE: Duration = Duration::from_millis(200);

pub struct Backend {
    client: Client,
    workspace: Arc<SharedWorkspace>,
    /// files that currently carry published diagnostics, so stale ones can be
    /// cleared once a change moves the errors elsewhere
    published: Arc<Mutex<HashSet<Url>>>,
    /// files waiting for the debounce to elapse, mapped to the edit that
    /// scheduled them. An edit that arrives later replaces the one waiting.
    pending: Arc<Mutex<HashMap<Url, u64>>>,
    edits: Arc<AtomicU64>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: Arc::new(Mutex::new(Workspace::default())),
            published: Arc::new(Mutex::new(HashSet::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            edits: Arc::new(AtomicU64::new(0)),
        }
    }

    fn text(&self, uri: &Url) -> Option<String> {
        lock(&self.workspace).text(uri)
    }

    async fn publish(&self, uri: &Url) {
        publish_diagnostics(&self.client, &self.workspace, &self.published, uri).await
    }

    /// Schedules a publish once the typing pauses. A newer edit for the same
    /// file takes over the schedule, so a burst of changes costs one analysis.
    fn schedule_publish(&self, uri: &Url) {
        let edit = self.edits.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uri.clone(), edit);
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        let published = self.published.clone();
        let pending = self.pending.clone();
        let uri = uri.clone();
        tokio::spawn(async move {
            tokio::time::sleep(DIAGNOSTIC_DEBOUNCE).await;
            let superseded = pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&uri)
                .is_some_and(|waiting| *waiting != edit);
            if superseded {
                return;
            }
            pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&uri);
            publish_diagnostics(&client, &workspace, &published, &uri).await;
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_owned(), ":".to_owned()]),
                    ..Default::default()
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::FUNCTION,
                                    SemanticTokenType::VARIABLE,
                                    SemanticTokenType::KEYWORD,
                                    SemanticTokenType::MACRO,
                                    SemanticTokenType::TYPE,
                                    SemanticTokenType::MODIFIER,
                                    SemanticTokenType::PROPERTY,
                                    SemanticTokenType::EVENT,
                                    SemanticTokenType::OPERATOR,
                                ],
                                token_modifiers: vec![],
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        lock(&self.workspace).open(doc.uri.clone(), doc.text, doc.version);
        self.publish(&doc.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let Some(change) = params.content_changes.last() else {
            return;
        };
        lock(&self.workspace).change(&uri, change.text.clone(), params.text_document.version);
        self.schedule_publish(&uri);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        lock(&self.workspace).close(&uri);
        if let Ok(mut published) = self.published.lock() {
            published.remove(&uri);
        }
        let _ = self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.publish(&params.text_document.uri).await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let Some(token) = convert::token_at(&text, pos, &analysis.tokens.tokens) else {
            return Ok(None);
        };
        if !matches!(token.0, TokenKind::Ident(_)) {
            return Ok(None);
        }
        let line_count = convert::LineIndex::new(&text).line_count() as u32;
        // a symbol declared in another file (a required module, the builtin
        // prelude) has a span that means nothing in this document, so it must
        // not be reported against this uri
        let here = |span: &duka_shared::errors::Span| span.start.line < line_count;
        for link in &analysis.scope.links {
            if link.name_span == token.1 && here(&link.decl_span) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range: convert::lsp_range(&text, link.decl_span),
                })));
            }
        }
        let table = &analysis.scope.symbols;
        let sym = table.symbol_at_span(token.1).or_else(|| {
            analysis
                .scope
                .uses
                .get(&token.1)
                .and_then(|id| table.symbol_by_id(*id))
        });
        if let Some(sym) = sym
            && here(&sym.span)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, sym.span),
            })));
        }
        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let text = match self.text(uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let table = &analysis.scope.symbols;

        let line_index = convert::LineIndex::new(&text);
        let Some(idx) = analysis.tokens.tokens.iter().position(|t| {
            let range = line_index.range(&text, t.1);
            pos >= range.start && pos < range.end
        }) else {
            return Ok(None);
        };
        let token = &analysis.tokens.tokens[idx];

        let (kind, span) = token;
        if let Some(view) = kind
            .is_keyword()
            .then_some(())
            .and_then(|_| docs::keyword_view(kind.name()))
            .or_else(|| {
                let TokenKind::Ident(name) = kind else {
                    return None;
                };
                duka_shared::dtype::Type::from_keyword(name)
                    .and_then(|ty| docs::type_view(&ty))
                    .or_else(|| {
                        idx.checked_sub(1)
                            .and_then(|i| analysis.tokens.tokens.get(i))
                            .filter(|(k, _)| matches!(k, TokenKind::At | TokenKind::Less))
                            .map(|_| docs::attr_view(name))
                    })
            })
        {
            return Ok(Some(convert::to_markup_hover(&text, token, &view.render())));
        }

        if !matches!(kind, TokenKind::Ident(_)) {
            return Ok(convert::to_vararg_hover(&text, token)
                .or_else(|| convert::to_literal_hover(&line_index, &text, token)));
        }

        if let Some((container, detail)) =
            object_member(&analysis, *span, ident_name(kind).unwrap_or_default())
        {
            return Ok(Some(convert::to_member_hover(
                &text, token, &container, &detail,
            )));
        }

        if table_key_at(&analysis.tokens.tokens, idx) {
            return Ok(convert::to_table_key_hover(&line_index, &text, token));
        }

        if let Some(base) =
            member_path_before(&line_index, &text, span.start.line, span.start.column)
        {
            // a builtin has no declaration to read, its documentation is the
            // only description there is
            if let Some(view) = docs::value_builtin_view(&format!(
                "{base}.{}",
                ident_name(kind).unwrap_or_default()
            )) {
                return Ok(Some(convert::to_markup_hover(&text, token, &view.render())));
            }
            if let Some(container) = path_type(&analysis, &base) {
                if let Some((_, detail)) = convert::record_members(&container)
                    .into_iter()
                    .find(|(name, _)| Some(name.as_str()) == ident_name(kind))
                {
                    return Ok(Some(convert::to_member_hover(&text, token, &base, &detail)));
                }
            }
            // an object value carries its type as the object name, so the
            // members come from the object table rather than record text
            if let Some((owner, detail)) = object_member_of_name(
                &analysis,
                &container_name(&analysis, &base),
                ident_name(kind),
            ) {
                return Ok(Some(convert::to_member_hover(
                    &text,
                    token,
                    &format!("{base}:{owner}"),
                    &detail,
                )));
            }
        }

        if let Some(link) = analysis.scope.links.iter().find(|l| l.name_span == *span) {
            let object = analysis.scope.objects.get(link.owner);
            let method = object.and_then(|o| o.methods.iter().find(|m| m.span == link.decl_span));
            if let (Some(object), Some(method)) = (object, method) {
                return Ok(Some(convert::to_method_hover(&text, token, object, method)));
            }
            return Ok(None);
        }
        let ty = table
            .symbol_at_span(*span)
            .or_else(|| {
                analysis
                    .scope
                    .uses
                    .get(span)
                    .and_then(|id| table.symbol_by_id(*id))
            })
            .or_else(|| {
                // A name in a type position never went through the expression
                // resolver, so it has no entry in `uses`. Resolving by position
                // rather than by name is what makes a redeclared or shadowed
                // name pick the declaration that is actually in effect.
                ident_name(kind)
                    .and_then(|name| table.resolve_at((span.start.line, span.start.column), name))
            });
        // what the type parameters of a generic call solved to, plus a `where`
        // line for the ones declared with a bound. This is an addition to the
        // usual hover, never a replacement: the resolved signature matters more.
        let generic_note = generic_binding_note(&analysis, *span);
        let name = ident_name(kind).unwrap_or_default();
        if let Some(symbol) = ty
            && let Some(hover) = symbol_hover(
                &analysis,
                &text,
                token,
                symbol,
                name,
                generic_note.as_deref(),
            )
        {
            return Ok(Some(hover));
        }
        // Neither the standard library nor the type context builtins are
        // symbols, so a name that resolves to nothing is one of them if either
        // table has it. A local of the same name would have resolved above and
        // shadows them, which is the order the language gives.
        if let Some(view) = ident_name(kind).and_then(|name| {
            docs::value_builtin_view(name).or_else(|| docs::type_builtin_view(name))
        }) {
            return Ok(Some(convert::to_markup_hover(&text, token, &view.render())));
        }
        let mut hover = convert::to_hover(&text, token, ty);
        if let Some(note) = &generic_note {
            convert::append_markup(&mut hover, note);
        }
        Ok(Some(hover))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let data = convert::semantic_tokens(
            &text,
            &analysis.tokens.tokens,
            &analysis.scope.symbols,
            &analysis.roles,
        );
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let index = convert::LineIndex::new(&text);
        let mut out: Vec<DocumentSymbol> = vec![];

        // One pass over the token stream looking for declarations: an
        // identifier that is the first thing on its line, optionally behind
        // `export` / `local` / `function` / `type` / `object`. Anything else on
        // that line, a parameter list in particular, is not a declaration.
        // Line breaks are detected by comparing line numbers: a newline token
        // can sit entirely inside one line, so its own span says nothing.
        let mut at_line_start = true;
        let mut exported = false;
        let mut last_line = 0u32;
        let mut seen_any = false;
        for (kind, span) in &analysis.tokens.tokens {
            if !seen_any || span.start.line != last_line {
                at_line_start = true;
                exported = false;
                last_line = span.start.line;
                seen_any = true;
            }
            if kind.is_terminator() {
                continue;
            }
            match kind {
                TokenKind::Export => {
                    exported = true;
                }
                TokenKind::Local
                | TokenKind::Global
                | TokenKind::Type
                | TokenKind::Function
                | TokenKind::Fn
                | TokenKind::Object => {}
                _ if at_line_start && matches!(kind, TokenKind::Ident(_)) => {
                    at_line_start = false;
                    let symbol = analysis.scope.symbols.symbol_at_span(*span);
                    let sym_kind = symbol.and_then(|s| match s.symbol_type {
                        SymbolType::Function => Some(SymbolKind::FUNCTION),
                        SymbolType::ObjectClass(_) => Some(SymbolKind::STRUCT),
                        SymbolType::TypeAlias(_) => Some(SymbolKind::INTERFACE),
                        SymbolType::TypeFunction(_) | SymbolType::InlineTypeFunction(_) => {
                            Some(SymbolKind::FUNCTION)
                        }
                        // an exported value is part of the module surface, so it
                        // belongs in the outline even when it is just a binding
                        SymbolType::Constant(_) if exported => Some(SymbolKind::CONSTANT),
                        SymbolType::Variable if exported => Some(SymbolKind::VARIABLE),
                        _ => None,
                    });
                    if let Some(symbol_kind) = sym_kind {
                        let name = ident_name(kind).unwrap_or_default().to_string();
                        let range = index.range(&text, *span);
                        out.push(DocumentSymbol {
                            name,
                            detail: symbol.and_then(|s| s.ty.as_deref().map(str::to_string)),
                            kind: symbol_kind,
                            tags: None,
                            #[allow(deprecated)]
                            deprecated: None,
                            range,
                            selection_range: range,
                            children: None,
                        });
                    }
                }
                _ => at_line_start = false,
            }
        }

        out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
        Ok(Some(DocumentSymbolResponse::Nested(out)))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let p = params.text_document_position;
        let uri = &p.text_document.uri;
        let pos = p.position;
        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let index = convert::LineIndex::new(&text);
        let Some((kind, span)) =
            convert::token_at_indexed(&index, &text, pos, &analysis.tokens.tokens)
        else {
            return Ok(None);
        };
        let Some(name) = ident_name(kind) else {
            return Ok(None);
        };
        let target = analysis.scope.symbols.symbol_at_span(*span);
        let target_id = target.map(|s| s.id);

        let mut out: Vec<Location> = vec![];
        // inside `{ ... }` an `ident =` pair is a table key, not a reference
        let mut brace_depth = 0usize;
        let mut prev_significant: Option<&TokenKind> = None;
        for (i, (kind, span)) in analysis.tokens.tokens.iter().enumerate() {
            if !kind.is_terminator() {
                match kind {
                    TokenKind::LBrace => brace_depth += 1,
                    TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                    _ => {}
                }
            }
            let is_ident = ident_name(kind) == Some(name);
            if is_ident && brace_depth > 0 {
                let next_is_assign = analysis
                    .tokens
                    .tokens
                    .get(i + 1)
                    .is_some_and(|(k, _)| matches!(k, TokenKind::Assign));
                let after_open = prev_significant.is_none_or(|k| {
                    matches!(k, TokenKind::LBrace | TokenKind::Comma) || k.is_terminator()
                });
                if next_is_assign && after_open {
                    prev_significant = Some(kind);
                    continue;
                }
            }
            if !kind.is_terminator() {
                prev_significant = Some(kind);
            }
            if !is_ident {
                continue;
            }
            // `b: boolean` inside a record type, and `{ a: 1 }` style keys, are
            // field declarations that happen to share the name
            let is_field_decl = analysis
                .tokens
                .tokens
                .get(i + 1)
                .is_some_and(|(k, _)| matches!(k, TokenKind::Colon));
            if is_field_decl {
                continue;
            }
            // a member read is described by the member, not the container
            if member_path_before(&index, &text, span.start.line, span.start.column).is_some() {
                continue;
            }
            // a same-named declaration in another scope is a different symbol
            if let (Some(target_id), Some(found)) =
                (target_id, analysis.scope.symbols.symbol_at_span(*span))
                && found.id != target_id
            {
                continue;
            }
            out.push(Location {
                uri: uri.clone(),
                range: index.range(&text, *span),
            });
        }
        out.sort_by_key(|l| (l.range.start.line, l.range.start.character));
        out.dedup_by(|a, b| a.range == b.range);
        Ok(Some(out))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = &params.text_document.uri;
        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let index = convert::LineIndex::new(&text);
        let range = params.range;
        let in_range = |r: Range| r.start.line >= range.start.line && r.end.line <= range.end.line;
        let mut hints: Vec<InlayHint> = vec![];

        // inferred type on a declaration that has no annotation
        for (i, (kind, span)) in analysis.tokens.tokens.iter().enumerate() {
            if !matches!(kind, TokenKind::Ident(_)) {
                continue;
            }
            // A declaration name is what follows `local` / `global` /
            // `export` or a comma in a multi name binding. Only the token right
            // before it matters: requiring the whole line to be keywords rejects
            // the second and later name of `local a, b, c = ...`, while a use
            // site is preceded by `(`, `.`, `:` or an operator.
            let head = analysis.tokens.tokens[..i]
                .iter()
                .rev()
                .find(|(k, _)| !k.is_terminator())
                .is_none_or(|(k, _)| {
                    matches!(
                        k,
                        TokenKind::Local
                            | TokenKind::Global
                            | TokenKind::Export
                            | TokenKind::Type
                            | TokenKind::Function
                            | TokenKind::Fn
                            | TokenKind::Object
                            | TokenKind::Comma
                    )
                });
            if !head {
                continue;
            }
            // an explicit annotation already says the type
            let annotated = analysis
                .tokens
                .tokens
                .get(i + 1)
                .is_some_and(|(k, _)| matches!(k, TokenKind::Colon));
            if annotated {
                continue;
            }
            let Some(symbol) = analysis.scope.symbols.symbol_at_span(*span) else {
                continue;
            };
            // `any` still gets a hint: showing that a type is unknown is more
            // use than showing nothing
            let Some(ty) = symbol.ty.as_deref().filter(|t| !t.is_empty()) else {
                continue;
            };
            if !matches!(
                symbol.symbol_type,
                SymbolType::Variable | SymbolType::Constant(_)
            ) {
                continue;
            }
            let position = index.position(&text, span.end.line, span.end.column);
            if !in_range(index.range(&text, *span)) {
                continue;
            }
            hints.push(InlayHint {
                position,
                label: InlayHintLabel::String(format!(": {}", one_line_type(ty))),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: Some(false),
                padding_right: Some(false),
                data: None,
            });
        }

        // parameter names at a call site
        for (i, (kind, span)) in analysis.tokens.tokens.iter().enumerate() {
            if !matches!(kind, TokenKind::Ident(_)) {
                continue;
            }
            let Some((TokenKind::LParen, open)) = analysis.tokens.tokens.get(i + 1) else {
                continue;
            };
            // A standard library function has no declaration to read the
            // parameter names off, its signature is where they live.
            let params =
                docs::value_builtin_params(&builtin_path(&text, &index, *span)).or_else(|| {
                    let decl = analysis
                        .scope
                        .links
                        .iter()
                        .find(|l| l.name_span == *span)
                        .map(|l| l.decl_span)
                        .or_else(|| analysis.scope.symbols.symbol_at_span(*span).map(|s| s.span))
                        .or_else(|| {
                            // at the call site the name has no span-mapped symbol
                            ident_name(kind)
                                .and_then(|n| analysis.scope.symbols.lookup_unambiguous(n))
                                .map(|s| s.span)
                        })?;
                    // the declaration itself is not a call site
                    if decl.start == span.start || decl.start.line >= text.lines().count() as u32 {
                        return None;
                    }
                    param_names(&analysis.tokens.tokens, decl)
                });
            let Some(params) = params else { continue };
            if params.is_empty() {
                continue;
            }
            // walk the arguments, matching them to the parameter list
            let mut depth = 0usize;
            let mut arg_index = 0usize;
            for (j, (k, s)) in analysis.tokens.tokens.iter().enumerate().skip(i + 1) {
                match k {
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                    TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                if depth != 1 {
                    continue;
                }
                let starts_arg = j == i + 2
                    || matches!(
                        analysis.tokens.tokens.get(j - 1).map(|(p, _)| p),
                        Some(TokenKind::Comma)
                    );
                if !starts_arg {
                    continue;
                }
                let Some(name) = params.get(arg_index) else {
                    break;
                };
                arg_index += 1;
                let _ = open;
                let position = index.position(&text, s.start.line, s.start.column);
                if !in_range(index.range(&text, *s)) {
                    continue;
                }
                hints.push(InlayHint {
                    position,
                    label: InlayHintLabel::String(format!("{name}:")),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(false),
                    padding_right: Some(false),
                    data: None,
                });
            }
        }

        hints.sort_by_key(|h| (h.position.line, h.position.character));
        Ok(Some(hints))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(text) = self.text(uri) else {
            return Ok(None);
        };
        let mut workspace = lock(&self.workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return Ok(None);
        };
        let index = convert::LineIndex::new(&text);
        // the token the cursor is in, which is also what says whether the word
        // being completed names a type or a value
        let at = analysis
            .tokens
            .tokens
            .iter()
            .position(|(_, span)| {
                let range = index.range(&text, *span);
                pos >= range.start && pos < range.end
            })
            .unwrap_or(analysis.tokens.tokens.len());

        if let Some((kind, string_span)) =
            convert::token_at_indexed(&index, &text, pos, &analysis.tokens.tokens)
        {
            if matches!(kind, TokenKind::Comment(_)) {
                return Ok(None);
            }
            if matches!(kind, TokenKind::String(_)) {
                // inside `require "..."` the useful completion is the module
                // name, everything else stays quiet inside a string
                let at = analysis
                    .tokens
                    .tokens
                    .iter()
                    .position(|(_, s)| s == string_span);
                let after_require = at
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| analysis.tokens.tokens.get(i))
                    .is_some_and(|(k, _)| matches!(k, TokenKind::Ident(n) if n == "require"));
                if !after_require {
                    return Ok(None);
                }
                let items = compile::module_candidates(uri.to_file_path().ok().as_deref())
                    .into_iter()
                    .map(|name| CompletionItem {
                        label: name,
                        kind: Some(CompletionItemKind::MODULE),
                        ..Default::default()
                    })
                    .collect();
                return Ok(Some(CompletionResponse::List(CompletionList {
                    is_incomplete: false,
                    items,
                })));
            }
        }

        if let Some(base) = member_access_base(&index, &text, pos) {
            // a standard library module has no type to read members off
            let members = docs::value_builtin_members(&base);
            if !members.is_empty() {
                return Ok(Some(CompletionResponse::List(CompletionList {
                    is_incomplete: false,
                    items: members
                        .into_iter()
                        .take(MAX_COMPLETION_ITEMS)
                        .map(builtin_item)
                        .collect(),
                })));
            }
            let Some(ty) = path_type(&analysis, &base) else {
                return Ok(None);
            };
            let members = convert::record_members(&ty);
            if members.is_empty() {
                return Ok(None);
            }
            let items = members
                .into_iter()
                .take(MAX_COMPLETION_ITEMS)
                .map(|(name, detail)| {
                    let kind = if detail.starts_with("function") {
                        CompletionItemKind::FUNCTION
                    } else {
                        CompletionItemKind::FIELD
                    };
                    CompletionItem {
                        label: name,
                        kind: Some(kind),
                        detail: Some(detail),
                        ..Default::default()
                    }
                })
                .collect();
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items,
            })));
        }

        // What is being completed decides which namespace applies. The standard
        // library is not one flat list: `print` is a value and `IsSubType` only
        // means anything in a type, so offering both in the same place would be
        // wrong in both.
        let word = name_at(&text, pos).unwrap_or_default();
        let context = completion_context(&analysis.tokens.tokens, at);
        if context == CompletionContext::Declaration {
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items: vec![],
            })));
        }
        let (items, truncated) = completion_items(&analysis, context, &word);
        if truncated {
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: true,
                items,
            })));
        }
        Ok(Some(CompletionResponse::Array(items)))
    }
}

/// One item per name. A builtin the analyser also declared is the same thing
/// seen twice: the symbol knows the type it was given and the metadata knows the
/// prose, so the item takes the type from one and the documentation from the
/// other instead of being offered twice under the same name.
fn completion_items(
    analysis: &compile::DocAnalysis,
    context: CompletionContext,
    word: &str,
) -> (Vec<CompletionItem>, bool) {
    let mut items: Vec<CompletionItem> = vec![];
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut truncated = false;

    'scopes: for scope in analysis.scope.symbols.scopes.iter().rev() {
        for (name, syms) in &scope.symbols {
            if items.len() >= MAX_COMPLETION_ITEMS {
                truncated = true;
                break 'scopes;
            }
            let Some(sym) = syms.iter().rev().find(|s| s.is_value()) else {
                continue;
            };
            let Some(kind) = completion_kind(&sym.symbol_type, context) else {
                continue;
            };
            at.insert(name.to_string(), items.len());
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(kind),
                detail: sym.ty.as_deref().map(|t| t.to_string()),
                ..Default::default()
            });
        }
    }

    let builtins = match context {
        CompletionContext::Value => docs::value_builtin_globals(),
        _ => docs::type_builtin_views(),
    };
    for view in builtins {
        if items.len() >= MAX_COMPLETION_ITEMS {
            truncated = true;
            break;
        }
        if !view.name.starts_with(word) {
            continue;
        }
        let documentation = view.render();
        match at.get(&*view.name).copied() {
            Some(existing) => {
                let item = &mut items[existing];
                if item.detail.as_deref().is_none_or(str::is_empty) {
                    item.detail = view.signature.clone();
                }
                item.documentation = Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: documentation,
                }));
            }
            None => {
                let label = view.name.clone().into_owned();
                at.insert(label.clone(), items.len());
                items.push(builtin_item(view));
            }
        }
    }

    if context == CompletionContext::Value {
        for doc in duka_shared::docs::KEYWORD_DOCS {
            let duka_shared::docs::KeywordDoc::Keyword { keyword, doc } = doc else {
                continue;
            };
            if items.len() >= MAX_COMPLETION_ITEMS {
                truncated = true;
                break;
            }
            let label = (*keyword).to_owned();
            if at.contains_key(&label) {
                continue;
            }
            at.insert(label.clone(), items.len());
            items.push(CompletionItem {
                label,
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(doc.title.to_string()),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: docs::keyword_view(keyword)
                        .map(|v| v.render())
                        .unwrap_or_default(),
                })),
                ..Default::default()
            });
        }
    }

    (items, truncated)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionContext {
    Value,
    Type,
    /// A name is being declared. Completing here would only ever suggest a name
    /// the declaration then shadows, so there is nothing worth offering.
    Declaration,
}

/// Which namespace the word being completed belongs to, read off the token in
/// front of it. Only one token is looked at: an annotation is opened by the `:`
/// after a declared name, by the `=` of a `type` declaration, or by a `->`
/// before a return type, and a declaration is opened by its keyword. A `.` or a
/// `:` that follows a value is a member read, and that is answered before this
/// is asked.
fn completion_context(
    tokens: &[duka_frontend::lexer::token::Token],
    at: usize,
) -> CompletionContext {
    let Some((kind, _)) = tokens[..at].iter().rev().find(|(k, _)| !k.is_terminator()) else {
        return CompletionContext::Value;
    };
    match kind {
        TokenKind::Colon | TokenKind::Arrow => return CompletionContext::Type,
        TokenKind::Local
        | TokenKind::Global
        | TokenKind::Export
        | TokenKind::Function
        | TokenKind::Fn => return CompletionContext::Declaration,
        TokenKind::Assign => {}
        _ => return CompletionContext::Value,
    }
    // `type A = ` names a type, `local a = ` names a value; both open with an
    // `=`, so the statement in front of it is what tells them apart
    let before = &tokens[..at];
    let start = before
        .iter()
        .rposition(|(k, _)| k.is_terminator())
        .map(|i| i + 1)
        .unwrap_or(0);
    if before[start..]
        .iter()
        .any(|(k, _)| matches!(k, TokenKind::Type))
    {
        CompletionContext::Type
    } else {
        CompletionContext::Value
    }
}

/// The completion kind of a symbol, or `None` when it does not belong to the
/// namespace being completed. A record field and a type parameter are members,
/// so neither is ever offered here.
fn completion_kind(kind: &SymbolType, context: CompletionContext) -> Option<CompletionItemKind> {
    use SymbolType as S;
    Some(match (kind, context) {
        (S::Function, CompletionContext::Value) => CompletionItemKind::FUNCTION,
        (S::Constant(_), CompletionContext::Value) => CompletionItemKind::CONSTANT,
        (S::Variable | S::Parameter(_), CompletionContext::Value) => CompletionItemKind::VARIABLE,
        (S::TypeAlias(_), CompletionContext::Type) => CompletionItemKind::STRUCT,
        (S::ObjectClass(_), CompletionContext::Type) => CompletionItemKind::CLASS,
        (S::TypeFunction(_) | S::InlineTypeFunction(_), CompletionContext::Type) => {
            CompletionItemKind::FUNCTION
        }
        (S::TypeBuiltin, CompletionContext::Type) => CompletionItemKind::FUNCTION,
        _ => return None,
    })
}

/// A completion item for something the standard library provides, documented
/// the same way a hover documents it.
fn builtin_item(view: docs::DocView<'static>) -> CompletionItem {
    let kind = if view.is_module {
        CompletionItemKind::MODULE
    } else if view
        .signature
        .as_deref()
        .is_some_and(|s| s.starts_with("const "))
    {
        CompletionItemKind::CONSTANT
    } else {
        CompletionItemKind::FUNCTION
    };
    let label = view.name.clone().into_owned();
    let documentation = view.render();
    CompletionItem {
        label,
        kind: Some(kind),
        detail: view.signature.clone(),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: documentation,
        })),
        ..Default::default()
    }
}

/// The word the cursor sits in, which is the prefix a completion filters by.
fn name_at(text: &str, pos: Position) -> Option<String> {
    let line = text.lines().nth(pos.line as usize)?;
    let chars: Vec<char> = line.chars().collect();
    let end = (pos.character as usize).min(chars.len());
    let mut start = end;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    (start < end).then(|| chars[start..end].iter().collect())
}

/// The dotted path a callee is written with, `string.upper` for a member of a
/// module and `print` for a global one, which is how the standard library
/// addresses its own functions.
fn builtin_path(text: &str, index: &convert::LineIndex, span: Span) -> String {
    let start = index.position(text, span.start.line, span.start.column);
    let line = index.line_text(text, start.line as usize);
    let chars: Vec<char> = line.chars().collect();
    // the span is the name itself, so it reads forward from where it starts
    let from = (start.character as usize).min(chars.len());
    let mut to = from;
    while to < chars.len() && is_ident_char(chars[to]) {
        to += 1;
    }
    let mut path = match member_path_before(index, text, span.start.line, span.start.column) {
        Some(base) => format!("{base}."),
        None => String::new(),
    };
    path.extend(chars[from..to].iter());
    path
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Groups the errors of a document by the file they belong to, so an error
/// inside a required module is reported on that module. The locks are released
/// before awaiting, the guards must not cross an await point.
fn diagnostics_of(
    workspace: &SharedWorkspace,
    published: &Mutex<HashSet<Url>>,
    uri: &Url,
) -> (HashMap<Url, Vec<Diagnostic>>, HashSet<Url>) {
    let mut groups: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    {
        let mut workspace = lock(workspace);
        let Some(analysis) = workspace.analysis(uri) else {
            return (groups, HashSet::new());
        };
        // a module that is reachable twice is analyzed once but its errors can
        // still arrive twice, the client should only ever see one
        let mut seen: HashSet<(u32, u32, u32, u32, String)> = HashSet::new();
        for err in &analysis.errors {
            let text = convert::error_text(err);
            let key = (
                err.span.start.line,
                err.span.start.column,
                err.span.end.line,
                err.span.end.column,
                err.kind.to_string(),
            );
            if !seen.insert(key) {
                continue;
            }
            let target = convert::error_uri(err, uri);
            groups
                .entry(target)
                .or_default()
                .push(convert::to_diagnostic(&text, uri, err));
        }
    }
    let published = published.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (groups, published)
}

async fn publish_diagnostics(
    client: &Client,
    workspace: &SharedWorkspace,
    published: &Mutex<HashSet<Url>>,
    uri: &Url,
) {
    let (groups, previously) = diagnostics_of(workspace, published, uri);
    // a file that no longer has errors still needs an empty list, otherwise
    // the client keeps showing the previous run
    for stale in previously.iter() {
        if !groups.contains_key(stale) {
            let _ = client
                .publish_diagnostics(stale.clone(), vec![], None)
                .await;
        }
    }
    for (target, diagnostics) in &groups {
        let _ = client
            .publish_diagnostics(target.clone(), diagnostics.clone(), None)
            .await;
    }
    *published.lock().unwrap_or_else(|e| e.into_inner()) = groups.into_keys().collect();
}

/// The dotted path immediately left of the cursor, but only when the cursor
/// sits after a `.` or `:` so `local x: ` is a type position rather than a
/// member access.
fn member_access_base(index: &convert::LineIndex, text: &str, pos: Position) -> Option<String> {
    let line = index.line_text(text, pos.line as usize);
    let chars: Vec<char> = line.chars().collect();
    let cursor = (pos.character as usize).min(chars.len());
    let mut start = cursor;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    if start == 0 {
        return None;
    }
    if !matches!(chars[start - 1], '.' | ':') {
        return None;
    }
    let mut path_end = start - 1;
    while path_end > 0 {
        let c = chars[path_end - 1];
        if is_ident_char(c) || c == '.' {
            path_end -= 1;
        } else {
            break;
        }
    }
    if path_end >= start - 1 {
        return None;
    }
    Some(chars[path_end..start - 1].iter().collect())
}

fn ident_name(kind: &TokenKind) -> Option<&str> {
    match kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    }
}

/// The dotted path left of an identifier when that identifier is being read as
/// a member, so hover can describe the member rather than its container.
fn member_path_before(
    index: &convert::LineIndex,
    text: &str,
    line: u32,
    column: u32,
) -> Option<String> {
    let start = index.position(text, line, column);
    let line_text = index.line_text(text, start.line as usize);
    let chars: Vec<char> = line_text.chars().collect();
    let mut i = (start.character as usize).min(chars.len());
    if i > 0 && chars[i - 1] == '?' {
        i -= 1;
    }
    if i == 0 || !matches!(chars[i - 1], '.' | ':') {
        return None;
    }
    let dot = i - 1;
    let mut start_path = dot;
    while start_path > 0 {
        let c = chars[start_path - 1];
        if is_ident_char(c) || c == '.' {
            start_path -= 1;
        } else {
            break;
        }
    }
    if start_path >= dot {
        return None;
    }
    Some(chars[start_path..dot].iter().collect())
}

/// `{ a = 1 }` - an `ident =` right after `{` or `,` inside braces is a table
/// key. It shares a name with whatever local happens to be called `a`, so it
/// has to be recognised structurally instead of by lookup.
fn table_key_at(tokens: &[duka_frontend::lexer::token::Token], at: usize) -> bool {
    let mut depth = 0i32;
    for (kind, _) in tokens.iter().take(at) {
        if kind.is_terminator() {
            continue;
        }
        match kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth -= 1,
            _ => {}
        }
    }
    if depth <= 0 {
        return false;
    }
    let next_is_assign = tokens
        .get(at + 1)
        .is_some_and(|(k, _)| matches!(k, TokenKind::Assign));
    let prev = tokens[..at]
        .iter()
        .rev()
        .find(|(k, _)| !k.is_terminator())
        .map(|(k, _)| k);
    let after_open =
        prev.is_none_or(|k| matches!(k, TokenKind::LBrace | TokenKind::Comma) || k.is_terminator());
    next_is_assign && after_open
}

/// A one-segment base's type, which for an object value is just the object name
fn container_name(analysis: &compile::DocAnalysis, base: &str) -> Option<String> {
    if base.contains('.') {
        return None;
    }
    path_type(analysis, base)
}

/// Looks a member up on an object by the object name a value carries
fn object_member_of_name(
    analysis: &compile::DocAnalysis,
    object_name: &Option<String>,
    member: Option<&str>,
) -> Option<(String, String)> {
    let object_name = object_name.as_deref()?;
    let member = member?;
    let object = analysis
        .scope
        .objects
        .iter()
        .find(|o| &*o.name == object_name)?;
    for m in &object.members {
        if &*m.name == member {
            return Some((object.name.to_string(), m.ty.to_string()));
        }
    }
    for m in &object.methods {
        if &*m.name == member {
            return Some((
                object.name.to_string(),
                Type::Function(Some(m.sig.clone())).to_string(),
            ));
        }
    }
    None
}

/// The `T = int` / `where T: num` block for a generic call, or `None` when the
/// hovered token is not inside one
fn generic_binding_note(analysis: &compile::DocAnalysis, span: Span) -> Option<String> {
    let bindings = analysis
        .scope
        .generic_bindings
        .iter()
        .find(|(call, _)| call.start <= span.start && span.end <= call.end)
        .map(|(_, b)| b)
        .filter(|b| !b.is_empty())?;

    let mut note = String::from("\n\n---\n\n\n");
    for b in bindings {
        note.push_str(&format!("- `{} = {}`\n", b.name, b.value));
    }
    let bounds: Vec<String> = bindings
        .iter()
        .filter_map(|b| {
            b.bound
                .as_ref()
                .map(|bound| format!("{}: {}", b.name, bound))
        })
        .collect();
    if !bounds.is_empty() {
        note.push('\n');
        note.push_str(&format!("`where {}`", bounds.join(", ")));
    }
    Some(note)
}
/// The hover for a symbol that has a shape of its own: a type function, a type
/// parameter, a member or a parameter. Everything else is left to the generic
/// rendering, which reads `SymbolType` on its own.
fn symbol_hover(
    analysis: &compile::DocAnalysis,
    text: &str,
    token: &duka_frontend::lexer::token::Token,
    symbol: &duka_shared::utils::Symbol,
    name: &str,
    generic_note: Option<&str>,
) -> Option<Hover> {
    let mut hover = match &symbol.symbol_type {
        SymbolType::TypeFunction(_) | SymbolType::InlineTypeFunction(_) => {
            let signature = match symbol.ty.as_deref().filter(|t| !t.is_empty()) {
                Some(ty) => format!("type function {name}{ty}"),
                None => format!("type function {name}"),
            };
            let mut view = docs::DocView {
                name: Cow::Borrowed(name),
                signature: Some(signature),
                content: "",
                example: None,
                details: vec![],
                source: None,
                is_module: false,
            };
            // a type context builtin is not a symbol, so this is the only place
            // its documentation can come from
            if let Some(builtin) = docs::type_builtin_view(name) {
                view.content = builtin.content;
                view.example = builtin.example;
                view.source = builtin.source;
            }
            convert::to_markup_hover(text, token, &view.render())
        }
        SymbolType::TypeParam { bound, default } => {
            let mut value = format!("```duka\n(type parameter) {name}\n```");
            if let Some(bound) = bound {
                value.push_str(&format!("\n\nbound: `{bound}`"));
            }
            if let Some(default) = default {
                value.push_str(&format!("\n\ndefault: `{default}`"));
            }
            convert::to_markup_hover(text, token, &value)
        }
        SymbolType::Field(declared) => {
            let owner = symbol
                .owner
                .and_then(|id| analysis.scope.symbols.symbol_by_id(id))
                .map(|owner| symbol_name(analysis, owner))
                .unwrap_or_default();
            let rendered = symbol.ty.as_deref().unwrap_or_default();
            let detail = if rendered.is_empty() {
                declared.to_string()
            } else {
                rendered.to_string()
            };
            convert::to_member_hover(text, token, &owner, &detail)
        }
        SymbolType::Parameter(_) => convert::to_param_hover(text, token, symbol.ty.as_deref()),
        _ => return None,
    };
    if let Some(note) = generic_note {
        convert::append_markup(&mut hover, note);
    }
    Some(hover)
}

/// The name a symbol was written under, read back from the token that carries
/// its declaration span.
fn symbol_name(analysis: &compile::DocAnalysis, symbol: &duka_shared::utils::Symbol) -> String {
    analysis
        .tokens
        .tokens
        .iter()
        .find(|(_, span)| *span == symbol.span)
        .and_then(|(kind, _)| ident_name(kind))
        .unwrap_or_default()
        .to_string()
}

/// An inlay hint is a single line control, but a rendered type is multi line:
/// a nested record comes out as `{\n\ta: ...`. Feeding that in raw shows the
/// control characters and blows the line open, so it gets folded onto one line
/// and clipped.
fn one_line_type(ty: &str) -> String {
    let flat: String = ty.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 60 {
        return flat;
    }
    let cut: String = flat.chars().take(57).collect();
    format!("{cut}...")
}

/// Reads the parameter names out of a function declaration, so a call site can
/// label its arguments. The list is whatever sits between the parentheses that
/// follow the declared name.
fn param_names(tokens: &[duka_frontend::lexer::token::Token], decl: Span) -> Option<Vec<String>> {
    let name_at = tokens
        .iter()
        .position(|(_, span)| *span == decl)
        .or_else(|| tokens.iter().position(|(_, span)| span.start == decl.start))?;
    let mut i = name_at + 1;
    while i < tokens.len() && !matches!(tokens[i].0, TokenKind::LParen) {
        if tokens[i].0.is_terminator() {
            return None;
        }
        i += 1;
    }
    if i >= tokens.len() {
        return None;
    }
    let mut names = vec![];
    let mut in_annotation = false;
    i += 1;
    loop {
        match tokens.get(i) {
            None => return None,
            Some((TokenKind::RParen, _)) => return Some(names),
            Some((TokenKind::Colon, _)) => in_annotation = true,
            Some((TokenKind::Comma, _)) => in_annotation = false,
            Some((TokenKind::Ident(name), _)) if !in_annotation => {
                names.push(name.to_string());
            }
            Some((TokenKind::Dots, _)) => {
                names.push("...".to_string());
                return Some(names);
            }
            Some(_) => {}
        }
        i += 1;
    }
}

/// Object fields and methods are not record text, they live in the object
/// table, so `count: int` inside `object Counter` resolved to nothing and
/// hovered as an unresolved name.
fn object_member(
    analysis: &compile::DocAnalysis,
    span: Span,
    name: &str,
) -> Option<(String, String)> {
    for object in &analysis.scope.objects {
        if span.start.line < object.decl_span.start.line
            || span.end.line > object.decl_span.end.line
        {
            continue;
        }
        for member in &object.members {
            if &*member.name == name {
                return Some((object.name.to_string(), member.ty.to_string()));
            }
        }
        for method in &object.methods {
            if &*method.name == name {
                return Some((
                    object.name.to_string(),
                    Type::Function(Some(method.sig.clone())).to_string(),
                ));
            }
        }
    }
    None
}

/// Resolves `a.b.c` to the rendered type of `c`, reading intermediate members
/// back out of the record text the symbol table stores.
fn path_type(analysis: &compile::DocAnalysis, path: &str) -> Option<String> {
    let mut ty: Option<String> = None;
    let mut structured: Option<std::sync::Arc<Type>> = None;
    for (i, seg) in path.split('.').enumerate() {
        if i == 0 {
            let symbol = analysis.scope.symbols.lookup_unambiguous(seg)?;
            structured = symbol.ty_value.clone();
            ty = symbol.ty.as_deref().map(str::to_string);
        } else {
            // the analysed type walks directly; only symbols that never got one
            // fall back to reading the rendered text back apart
            if let Some(current) = structured.as_deref()
                && let Some(next) = convert::member_of_type(current, seg)
            {
                ty = Some(next.to_string());
                structured = Some(next);
                continue;
            }
            let members = convert::record_members(ty.as_deref()?);
            ty = members
                .into_iter()
                .find(|(name, _)| name == seg)
                .map(|(_, detail)| detail);
            structured = None;
        }
        ty.as_ref()?;
    }
    ty
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(text: &str) -> compile::DocAnalysis {
        let mut cache = duka_frontend::analyzer::modules::ModuleBuildCache::default();
        compile::analyze(text, "test.duka", None, &HashMap::new(), &mut cache)
    }

    #[test]
    fn token_at_matches_multiline() {
        let text = "local a\nlocal b\nlocal c\nprint(b)\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 1,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("line2 token");
        assert!(matches!(
            token.0,
            duka_frontend::lexer::token::TokenKind::Ident(_)
        ));
        assert_eq!(token.1.start.line, 2);

        let pos = Position {
            line: 3,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("line4 token");
        assert!(matches!(
            token.0,
            duka_frontend::lexer::token::TokenKind::Ident(_)
        ));
        assert_eq!(token.1.start.line, 4);
    }

    #[test]
    fn hover_symbol_by_span_is_distinct_per_declaration() {
        let text = "local a = 1\nlocal b = 2\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let pos = Position {
            line: 0,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("a");
        let sym_a = table.symbol_at_span(token.1).expect("a symbol");
        let pos = Position {
            line: 1,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("b");
        let sym_b = table.symbol_at_span(token.1).expect("b symbol");
        assert_ne!(sym_a.id, sym_b.id);
    }

    #[test]
    fn goto_method_call_targets_decl() {
        let text = "object A\n    function :foo(a)\n        return a\n    end\nend\nlocal a: A = A.new()\na:foo(1)\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 6,
            character: 2,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("foo token");
        let link = analysis
            .scope
            .links
            .iter()
            .find(|l| l.name_span == token.1)
            .expect("method link");
        assert_eq!(analysis.scope.objects[link.owner].name.as_ref(), "A");
        assert_eq!(
            link.decl_span,
            analysis.scope.objects[link.owner].methods[0].span
        );
    }

    #[test]
    fn hover_at_use_site_resolves_to_declaration() {
        let text = "local a = 1\nprint(a)\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 1,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("use token");
        assert_eq!(token.1.start.line, 2);
        let id = analysis
            .scope
            .uses
            .get(&token.1)
            .copied()
            .expect("recorded use");
        let sym = analysis
            .scope
            .symbols
            .symbol_by_id(id)
            .expect("symbol by id");
        assert_eq!(sym.span.start.line, 1);
        assert!(!sym.is_global);
    }

    #[test]
    fn hover_method_call_shows_owner_method() {
        let text = "object A\n    function :foo(a: int)\n        return a\n    end\nend\nlocal a: A = A.new()\na:foo(1)\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 6,
            character: 2,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("foo token");
        let link = analysis
            .scope
            .links
            .iter()
            .find(|l| l.name_span == token.1)
            .expect("method link");
        let object = analysis.scope.objects.get(link.owner).expect("object");
        let method = object
            .methods
            .iter()
            .find(|m| m.span == link.decl_span)
            .expect("method");
        let hover = convert::to_method_hover(text, token, object, method);
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("A"), "{value}");
        assert!(value.contains(":foo"), "{value}");
        assert!(value.contains("function"), "{value}");
    }

    #[test]
    fn hover_static_method_shows_dot() {
        let text = "object A\n    function foo()\n        return 1\n    end\nend\nA.foo()\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 5,
            character: 2,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("foo token");
        let link = analysis
            .scope
            .links
            .iter()
            .find(|l| l.name_span == token.1)
            .expect("method link");
        let object = analysis.scope.objects.get(link.owner).expect("object");
        let method = object
            .methods
            .iter()
            .find(|m| m.span == link.decl_span)
            .expect("method");
        let hover = convert::to_method_hover(text, token, object, method);
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("A.foo"), "{value}");
        assert!(!value.contains(":foo"), "{value}");
    }

    #[test]
    fn hover_variable_shows_local_and_type() {
        let text = "local a: int = 1\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 0,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("a");
        let symbol = analysis.scope.symbols.symbol_at_span(token.1).expect("sym");
        let hover = convert::to_hover(text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("local"), "{value}");
        assert!(value.contains("int"), "{value}");
    }

    #[test]
    fn hover_inferred_type_from_initializer() {
        let text = "local a = 1\nprint(a)\n";
        let analysis = analyze(text);
        for line in 0..2 {
            let pos = Position { line, character: 6 };
            let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("a token");
            let table = &analysis.scope.symbols;
            let symbol = table.symbol_at_span(token.1).or_else(|| {
                analysis
                    .scope
                    .uses
                    .get(&token.1)
                    .and_then(|id| table.symbol_by_id(*id))
            });
            let hover = convert::to_hover(text, token, symbol);
            let value = match hover.contents {
                HoverContents::Markup(m) => m.value,
                _ => panic!("expected markup"),
            };
            assert!(value.contains("int"), "line {line}: {value}");
        }
    }

    #[test]
    fn hover_inferred_type_from_function_call() {
        let text = "function f()\n    return 1\nend\nlocal x = f()\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 3,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("x token");
        let symbol = analysis.scope.symbols.symbol_at_span(token.1).expect("sym");
        let hover = convert::to_hover(text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("int"), "{value}");
    }

    #[test]
    fn hover_function_shows_inferred_signature() {
        let text = "function f(a: int)\n    return a + 1\nend\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 0,
            character: 9,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("f token");
        let symbol = analysis.scope.symbols.symbol_at_span(token.1).expect("sym");
        let hover = convert::to_hover(text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("function f"), "{value}");
        assert!(value.contains("int"), "{value}");
    }

    #[test]
    fn hover_global_variable_shows_global() {
        let text = "global a = 1\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 0,
            character: 7,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("a");
        let symbol = analysis.scope.symbols.symbol_at_span(token.1).expect("sym");
        let hover = convert::to_hover(text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("global"), "{value}");
    }

    fn semantics(text: &str, analysis: &compile::DocAnalysis) -> Vec<(String, u32)> {
        let data = convert::semantic_tokens(
            text,
            &analysis.tokens.tokens,
            &analysis.scope.symbols,
            &analysis.roles,
        );
        let mut out = vec![];
        let mut line = 0u32;
        let mut character = 0u32;
        for t in data {
            line += t.delta_line;
            if t.delta_line == 0 {
                character += t.delta_start;
            } else {
                character = t.delta_start;
            }
            let start = Position { line, character };
            let token = convert::token_at(text, start, &analysis.tokens.tokens)
                .expect("token at semantic position");
            let name = match &token.0 {
                duka_frontend::lexer::token::TokenKind::Ident(n) => n.as_str(),
                _ => "<kw>",
            };
            out.push((name.to_owned(), t.token_type));
        }
        out
    }

    fn types_of(semantics: &[(String, u32)], name: &str) -> Vec<u32> {
        semantics
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, t)| *t)
            .collect()
    }

    #[test]
    fn semantic_type_tokens() {
        let text = "object A\nend\nlocal a: A = A.new()\nlocal b: int = 1\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(
            types_of(&s, "A")
                .iter()
                .all(|t| *t == convert::SEMANTIC_TYPE)
        );
        assert!(
            types_of(&s, "a")
                .iter()
                .all(|t| *t == convert::SEMANTIC_VARIABLE)
        );
        assert!(
            types_of(&s, "b")
                .iter()
                .all(|t| *t == convert::SEMANTIC_VARIABLE)
        );
        assert!(
            types_of(&s, "int")
                .iter()
                .all(|t| *t == convert::SEMANTIC_TYPE)
        );
        assert!(
            types_of(&s, "new")
                .iter()
                .all(|t| *t == convert::SEMANTIC_FUNCTION)
        );
    }

    #[test]
    fn semantic_keyword_constant_tokens() {
        let text = "local c: bool = true\nif false then\n    print(nil)\nend\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(
            types_of(&s, "true")
                .iter()
                .all(|t| *t == convert::SEMANTIC_KEYWORD)
        );
        assert!(
            types_of(&s, "false")
                .iter()
                .all(|t| *t == convert::SEMANTIC_KEYWORD)
        );
        assert!(
            types_of(&s, "nil")
                .iter()
                .all(|t| *t == convert::SEMANTIC_KEYWORD)
        );
        for kw in ["local", "if", "then", "end"] {
            assert!(
                types_of(&s, kw)
                    .iter()
                    .all(|t| *t == convert::SEMANTIC_KEYWORD),
                "{kw} should be keyword"
            );
        }
        assert!(
            types_of(&s, "bool")
                .iter()
                .all(|t| *t == convert::SEMANTIC_TYPE)
        );
    }

    #[test]
    fn semantic_metamethod_tokens() {
        let text = "local mt = { __index = function(k) return k * 2 end }\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(
            types_of(&s, "__index")
                .iter()
                .all(|t| *t == convert::SEMANTIC_METAMETHOD)
        );
        assert!(
            types_of(&s, "function")
                .iter()
                .all(|t| *t == convert::SEMANTIC_KEYWORD)
        );
    }

    #[test]
    fn semantic_property_tokens() {
        let text = "print(a.b)\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(
            types_of(&s, "b")
                .iter()
                .all(|t| *t == convert::SEMANTIC_PROPERTY)
        );
    }

    #[test]
    fn semantic_method_chain_tokens() {
        let text = "a.b():c()\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(
            types_of(&s, "b")
                .iter()
                .all(|t| *t == convert::SEMANTIC_FUNCTION)
        );
        assert!(
            types_of(&s, "c")
                .iter()
                .all(|t| *t == convert::SEMANTIC_FUNCTION)
        );
    }

    #[test]
    fn a_standard_library_call_is_labelled_from_its_signature() {
        let text = "print(\"a\", 1)\n";
        let index = convert::LineIndex::new(text);
        let analysis = analyze(text);
        let span = token_of(text, &analysis, "print").1;
        assert_eq!(builtin_path(text, &index, span), "print");
        assert_eq!(
            docs::value_builtin_params(&builtin_path(text, &index, span)).as_deref(),
            Some(["...".to_owned()].as_slice()),
            "print is variadic"
        );

        let text = "local s = string.upper(\"a\")\n";
        let index = convert::LineIndex::new(text);
        let analysis = analyze(text);
        let span = token_of(text, &analysis, "upper").1;
        assert_eq!(builtin_path(text, &index, span), "string.upper");
        assert_eq!(
            docs::value_builtin_params(&builtin_path(text, &index, span)).as_deref(),
            Some(["s".to_owned()].as_slice())
        );
    }

    #[test]
    fn a_standard_library_function_is_described_the_way_a_local_one_is() {
        let text = "function f(a: int): string return \"x\" end\n";
        let analysis = analyze(text);
        let local = symbol_of(text, &analysis, "f(a: int)");
        let local_signature = match &local.symbol_type {
            SymbolType::Function => format!("function f: {}", local.ty.as_deref().unwrap()),
            other => panic!("{other:?}"),
        };
        let view = docs::value_builtin_view("string.upper").expect("string.upper");
        let builtin_signature = view.signature.clone().expect("signature");
        assert!(
            builtin_signature.contains("-> string"),
            "{builtin_signature}"
        );
        // same skeleton: a name, a colon, then the rendered type, which is what
        // a hover for a function written in this project produces
        fn tail(signature: &str) -> &str {
            signature.split_once(": ").map(|(_, t)| t).unwrap_or("")
        }
        assert!(
            tail(&builtin_signature).starts_with("function("),
            "{builtin_signature}"
        );
        assert!(
            tail(&builtin_signature).starts_with("function(")
                == tail(&local_signature).starts_with("function("),
            "a builtin must be rendered in the same shape as a local one: {builtin_signature} vs {local_signature}"
        );
    }

    #[test]
    fn a_rest_parameter_is_not_a_parameter_of_its_own() {
        let text = "function B<T: [V], V>(a: T, b: array<T!>!, ...): (int, T, ...)\n    return 1, 2\nend\n";
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, "B<T: [V]");
        let ty = symbol.ty.as_deref().expect("signature");
        assert_eq!(ty, "function(T?, array<T>, ...) -> (int?, T?, ...)");
        assert!(
            !ty.contains("any"),
            "the rest parameter must not become a phantom slot: {ty}"
        );
    }

    #[test]
    fn an_attribute_is_documented_and_tinted() {
        // the rule is about the `@` in front of a name, not about one attribute
        for (text, name) in [
            ("@inline function f() end\n", "inline"),
            ("@const x = 1\n", "const"),
            ("@data object A\nend\n", "data"),
            ("@whatever function g() end\n", "whatever"),
        ] {
            let analysis = analyze(text);
            let semantic = semantics(text, &analysis);
            assert_eq!(
                types_of(&semantic, name),
                vec![convert::SEMANTIC_ATTRIBUTE],
                "{name} should be an attribute: {semantic:?}"
            );
            let view = docs::attr_view(name);
            assert!(
                view.render().contains(&format!("{name}")) || view.render().contains("@"),
                "{name}: {}",
                view.render()
            );
        }
        assert!(
            docs::attr_view("const").content.contains("immutable"),
            "a documented attribute keeps its prose"
        );
    }

    #[test]
    fn a_standard_library_name_resolves_without_a_symbol() {
        for path in ["print", "os", "os.clock", "table.has", "io.File"] {
            assert!(
                docs::value_builtin_view(path).is_some(),
                "{path} has no documentation"
            );
        }
        assert!(docs::value_builtin_view("nope.nope").is_none());
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn a_name_is_offered_once_however_many_sources_know_it() {
        let analysis = analyze("local a = 1\n");
        let (items, truncated) = completion_items(&analysis, CompletionContext::Value, "");
        assert!(!truncated);
        let labels = labels(&items);
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            labels.len(),
            unique.len(),
            "a name appeared more than once: {labels:?}"
        );
        // `print` is both a declared builtin and a documented one, and the one
        // item carries the type from the analyser and the prose from the
        // metadata
        let print = items.iter().find(|i| i.label == "print").expect("print");
        assert!(print.detail.as_deref().is_some_and(|d| !d.is_empty()));
        assert!(print.documentation.is_some(), "print carries no prose");
    }

    #[test]
    fn an_attribute_is_one_form_whatever_it_is_named_or_given() {
        let text = "@inline function f() end\n@data(frozen: bool) object A\nend\n@returns(result) function g() end\n";
        let analysis = analyze(text);
        let types = semantics(text, &analysis);
        // the sigil, the name and the arguments all belong to the attribute
        for name in ["inline", "data", "frozen", "bool", "returns", "result"] {
            assert_eq!(
                types_of(&types, name),
                vec![convert::SEMANTIC_ATTRIBUTE],
                "{name} should be part of the attribute: {types:?}"
            );
        }
        // and what follows it is not
        assert_eq!(types_of(&types, "f"), vec![convert::SEMANTIC_FUNCTION]);
        assert_eq!(types_of(&types, "A"), vec![convert::SEMANTIC_TYPE]);
    }

    #[test]
    fn keyword_doc_hover_is_available() {
        let view = docs::keyword_view("if").expect("if doc");
        assert_eq!(view.name, "if");
        let value = view.render();
        assert!(value.contains("```duka"), "{value}");
        assert!(value.contains("if"), "{value}");
    }

    #[test]
    fn type_keyword_doc_hover_is_available() {
        let ty = Type::from_keyword("int").expect("int type");
        let value = docs::type_view(&ty).expect("int doc").render();
        assert!(value.contains("Integer"), "{value}");
    }

    #[test]
    fn attr_doc_hover_is_available() {
        let view = docs::attr_view("const");
        assert!(view.content.contains("immutable"), "{}", view.content);
        let value = view.render();
        assert!(value.contains("```duka"), "{value}");
    }

    #[test]
    fn a_standard_library_function_has_a_signature_and_prose() {
        let view = docs::value_builtin_view("print").expect("print");
        assert_eq!(view.name, "print");
        let signature = view.signature.as_deref().expect("signature");
        assert!(
            signature.starts_with("function print: function("),
            "{signature}"
        );
        assert!(!view.content.is_empty(), "{}", view.content);
    }

    #[test]
    fn a_standard_library_member_is_reachable_by_its_path() {
        let view = docs::value_builtin_view("string.upper").expect("string.upper");
        assert_eq!(view.name, "upper");
        let signature = view.signature.as_deref().expect("signature");
        assert!(
            signature.starts_with("function upper: function("),
            "{signature}"
        );
    }

    #[test]
    fn the_standard_library_is_offered_in_the_value_namespace() {
        let names: Vec<String> = docs::value_builtin_globals()
            .iter()
            .map(|v| v.name.clone().into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "print"), "{names:?}");
        assert!(names.iter().any(|n| n == "string"), "{names:?}");
        let members: Vec<String> = docs::value_builtin_members("string")
            .iter()
            .map(|v| v.name.clone().into_owned())
            .collect();
        assert!(members.iter().any(|n| n == "upper"), "{members:?}");
    }

    #[test]
    fn a_type_builtin_is_only_in_the_type_namespace() {
        let names: Vec<String> = docs::type_builtin_views()
            .iter()
            .map(|v| v.name.clone().into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "Error"), "{names:?}");
        assert!(
            docs::value_builtin_globals()
                .iter()
                .all(|v| &*v.name != "Error"),
            "a type builtin must not show up as a value"
        );
    }

    #[test]
    fn builtin_parameter_names_come_from_the_signature() {
        assert_eq!(
            docs::value_builtin_params("string.upper").as_deref(),
            Some(["s".to_owned()].as_slice())
        );
        assert!(docs::value_builtin_params("print").is_some());
        assert!(docs::value_builtin_params("nope.nope").is_none());
    }

    #[test]
    fn a_standard_library_call_keeps_its_declared_return_type() {
        let text = "local a = os.clock()\nlocal b = string.upper(\"x\")\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let module = table.lookup("os").expect("os is declared");
        assert!(
            module.ty_value.is_some(),
            "a module has to carry its members to be readable"
        );
        assert!(matches!(
            table.lookup("print").map(|s| &s.symbol_type),
            Some(SymbolType::Function)
        ));

        let a = symbol_of(text, &analysis, "a = os");
        assert_eq!(a.ty.as_deref(), Some("float?"), "os.clock returns a float");
        let b = symbol_of(text, &analysis, "b = string");
        assert_eq!(
            b.ty.as_deref(),
            Some("string"),
            "string.upper returns a string"
        );
    }

    #[test]
    fn a_type_builtin_is_a_symbol_but_not_a_type() {
        let text = "local e: Error = nil\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        assert!(
            table.lookup("Error").is_none(),
            "a type builtin must not resolve as a value"
        );
        let error = table
            .resolve_at((2, 0), "Error")
            .expect("Error is declared in the type namespace");
        assert!(
            matches!(error.symbol_type, SymbolType::TypeBuiltin),
            "{:?}",
            error.symbol_type
        );
        assert!(!error.is_value());
    }

    #[test]
    fn a_name_the_file_declares_itself_wins_over_a_builtin() {
        let text = "local print = 1\nlocal a = print\n";
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, "print = 1");
        assert!(matches!(symbol.symbol_type, SymbolType::Variable));
        assert_eq!(
            analysis.scope.symbols.lookup("print").map(|s| s.id),
            Some(symbol.id)
        );
    }

    #[test]
    fn completion_context_follows_the_annotation_it_is_in() {
        // `in` stands for where the cursor is: the token in front of it is what
        // decides the namespace
        let cases = [
            ("local a: in", CompletionContext::Type),
            ("local a = 1", CompletionContext::Value),
            ("type A = in", CompletionContext::Type),
            ("function f() -> in", CompletionContext::Type),
            ("local a = 1 + in", CompletionContext::Value),
            ("local in", CompletionContext::Declaration),
            ("global in", CompletionContext::Declaration),
            ("function in", CompletionContext::Declaration),
            ("export in", CompletionContext::Declaration),
            ("local a: int = in", CompletionContext::Value),
        ];
        for (text, expected) in cases {
            let analysis = analyze(&format!("{text}\n"));
            let at = analysis
                .tokens
                .tokens
                .iter()
                .rposition(|(_, span)| span.start.line == 1)
                .expect("a token on the first line");
            assert_eq!(
                completion_context(&analysis.tokens.tokens, at),
                expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn only_the_type_namespace_is_offered_where_a_type_is_expected() {
        assert_eq!(
            completion_kind(&SymbolType::TypeAlias(0), CompletionContext::Type),
            Some(CompletionItemKind::STRUCT)
        );
        assert_eq!(
            completion_kind(&SymbolType::Variable, CompletionContext::Type),
            None,
            "a local is not a type"
        );
        assert_eq!(
            completion_kind(&SymbolType::Function, CompletionContext::Type),
            None,
            "a function is not a type"
        );
        assert_eq!(
            completion_kind(&SymbolType::Function, CompletionContext::Value),
            Some(CompletionItemKind::FUNCTION)
        );
        assert_eq!(
            completion_kind(&SymbolType::TypeAlias(0), CompletionContext::Value),
            None,
            "a type alias is not a value"
        );
    }

    /// Line and column of the first occurrence of `needle`, counted in
    /// characters from the start of its line.
    fn offset_of(text: &str, needle: &str) -> (u32, u32) {
        for (i, line) in text.lines().enumerate() {
            if let Some(at) = line.find(needle) {
                return (i as u32, at as u32);
            }
        }
        panic!("no {needle} in fixture");
    }

    fn token_of<'a>(
        text: &str,
        analysis: &'a compile::DocAnalysis,
        needle: &str,
    ) -> &'a duka_frontend::lexer::token::Token {
        let (line, character) = offset_of(text, needle);
        convert::token_at(text, Position { line, character }, &analysis.tokens.tokens)
            .unwrap_or_else(|| panic!("no token at {needle}"))
    }

    /// The symbol a token names, resolved through the same three steps the
    /// hover handler uses: the span map, the recorded uses, then the table by
    /// position for a name that was never an expression.
    fn symbol_of<'a>(
        text: &str,
        analysis: &'a compile::DocAnalysis,
        needle: &str,
    ) -> &'a duka_shared::utils::Symbol {
        let token = token_of(text, analysis, needle);
        let table = &analysis.scope.symbols;
        table
            .symbol_at_span(token.1)
            .or_else(|| {
                analysis
                    .scope
                    .uses
                    .get(&token.1)
                    .and_then(|id| table.symbol_by_id(*id))
            })
            .or_else(|| {
                ident_name(&token.0).and_then(|name| {
                    table.resolve_at((token.1.start.line, token.1.start.column), name)
                })
            })
            .unwrap_or_else(|| panic!("{needle} resolved to nothing"))
    }

    fn hover_value(hover: &Hover) -> String {
        match &hover.contents {
            HoverContents::Markup(m) => m.value.clone(),
            _ => panic!("expected markup"),
        }
    }

    #[test]
    fn parameter_is_a_symbol_of_its_own() {
        let text = "function f(x: int, y)\n    return x\nend\n";
        let analysis = analyze(text);
        let x = symbol_of(text, &analysis, "x: int");
        assert!(
            matches!(x.symbol_type, SymbolType::Parameter(Type::Int)),
            "{:?}",
            x.symbol_type
        );
        let y = symbol_of(text, &analysis, "y)");
        assert!(
            matches!(y.symbol_type, SymbolType::Parameter(Type::Any)),
            "{:?}",
            y.symbol_type
        );
    }

    #[test]
    fn parameter_hover_reads_the_symbol() {
        let text = "function f(x: int)\n    return x\nend\n";
        let analysis = analyze(text);
        let value = hover_value(
            &symbol_hover(
                &analysis,
                text,
                token_of(text, &analysis, "x: int"),
                symbol_of(text, &analysis, "x: int"),
                "x",
                None,
            )
            .expect("param hover"),
        );
        assert!(value.contains("(param) x"), "{value}");
        assert!(value.contains("int"), "{value}");
    }

    #[test]
    fn type_parameter_carries_its_bound_and_default() {
        // an annotation is nullable unless it says otherwise, so a bound
        // written `num` is carried as the type it actually means
        let text = "function f<T: num, U = int>(a: T)\n    return a\nend\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let t = table.resolve_at((2, 13), "T").expect("T declared");
        assert_eq!(
            t.symbol_type,
            SymbolType::TypeParam {
                bound: Some("float?".into()),
                default: None
            }
        );
        let u = table.resolve_at((2, 13), "U").expect("U declared");
        assert_eq!(
            u.symbol_type,
            SymbolType::TypeParam {
                bound: None,
                default: Some("int?".into())
            }
        );
    }

    #[test]
    fn type_parameter_is_not_a_value() {
        let text = "function f<T: num>(a: T)\n    return a\nend\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        assert!(table.lookup("T").is_none(), "T resolved as a value");
        assert!(table.lookup_unambiguous("T").is_none());
        let declaration = token_of(text, &analysis, "T: num").1;
        assert!(table.symbol_at_span(declaration).is_some());
    }

    #[test]
    fn a_type_parameter_used_in_an_annotation_resolves_by_position() {
        let text = "function f<T: num>(a: T)\n    return a\nend\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let use_site = token_of(text, &analysis, "T)").1;
        assert!(
            table.symbol_at_span(use_site).is_none(),
            "the annotation is not a declaration"
        );
        assert!(
            analysis.scope.uses.get(&use_site).is_none(),
            "an annotation never went through the expression resolver"
        );
        let resolved = table
            .resolve_at((use_site.start.line, use_site.start.column), "T")
            .expect("the type parameter of this function");
        assert_eq!(
            resolved.symbol_type,
            SymbolType::TypeParam {
                bound: Some("float?".into()),
                default: None
            }
        );
        assert_eq!(symbol_of(text, &analysis, "T)").id, resolved.id);
    }

    #[test]
    fn a_type_parameter_use_hovers_as_a_type_parameter() {
        let text = "function f<T: num>(a: T)\n    return a\nend\n";
        let analysis = analyze(text);
        let value = hover_value(
            &symbol_hover(
                &analysis,
                text,
                token_of(text, &analysis, "T)"),
                symbol_of(text, &analysis, "T)"),
                "T",
                None,
            )
            .expect("type parameter hover"),
        );
        assert!(value.contains("(type parameter) T"), "{value}");
        assert!(value.contains("bound: `float?`"), "{value}");
    }

    #[test]
    fn record_fields_are_members_of_the_alias() {
        let text = "type Person = {\n    name: string,\n    age: int\n}\n";
        let analysis = analyze(text);
        let alias = symbol_of(text, &analysis, "Person =");
        assert!(matches!(alias.symbol_type, SymbolType::TypeAlias(_)));
        for (name, ty) in [("name", Type::String), ("age", Type::Int)] {
            let field = symbol_of(text, &analysis, name);
            assert_eq!(field.owner, Some(alias.id), "{name}");
            assert_eq!(field.symbol_type, SymbolType::Field(ty), "{name}");
        }
    }

    #[test]
    fn a_record_field_is_not_a_variable() {
        let text = "type Person = {\n    age: int\n}\nlocal age = 1\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let local = symbol_of(text, &analysis, "age = 1");
        assert!(matches!(local.symbol_type, SymbolType::Variable));
        let field = symbol_of(text, &analysis, "age:");
        assert_ne!(field.id, local.id);
        assert_eq!(table.lookup("age").map(|s| s.id), Some(local.id));
    }

    #[test]
    fn record_field_hover_names_its_owner() {
        let text = "type Person = {\n    name: string\n}\n";
        let analysis = analyze(text);
        let value = hover_value(
            &symbol_hover(
                &analysis,
                text,
                token_of(text, &analysis, "name: string"),
                symbol_of(text, &analysis, "name: string"),
                "name",
                None,
            )
            .expect("field hover"),
        );
        assert!(value.contains("(field) Person.name"), "{value}");
        assert!(value.contains("string"), "{value}");
    }

    #[test]
    fn object_properties_are_members_of_the_class() {
        let text = "object Counter\n    count: int\n    function :bump()\n        return self\n    end\nend\n";
        let analysis = analyze(text);
        let class = symbol_of(text, &analysis, "Counter");
        assert!(matches!(class.symbol_type, SymbolType::ObjectClass(_)));
        let count = symbol_of(text, &analysis, "count:");
        assert_eq!(count.owner, Some(class.id));
        assert_eq!(count.symbol_type, SymbolType::Field(Type::Int));
    }
}
