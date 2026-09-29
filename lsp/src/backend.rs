//! The Duka language server backend.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use duka_frontend::lexer::token::TokenKind;
use duka_shared::{
    docs::{attr_doc, keyword_doc, type_doc},
    dtype::Type,
    errors::Span,
    utils::SymbolType,
};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::{
    compile, convert,
    workspace::{SharedWorkspace, Workspace, lock},
};

/// Upper bound on the items a single completion request returns. Hitting it
/// flips the response to `isIncomplete`, so the client asks again as the user
/// narrows the prefix instead of us shipping every symbol in scope.
const MAX_COMPLETION_ITEMS: usize = 1000;

pub struct Backend {
    client: Client,
    workspace: Arc<SharedWorkspace>,
    /// files that currently carry published diagnostics, so stale ones can be
    /// cleared once a change moves the errors elsewhere
    published: Arc<Mutex<HashSet<Url>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: Arc::new(Mutex::new(Workspace::default())),
            published: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn text(&self, uri: &Url) -> Option<String> {
        lock(&self.workspace).text(uri)
    }

    /// Groups the errors of a document by the file they belong to, so an error
    /// inside a required module is reported on that module. The locks are
    /// released before awaiting, the guards must not cross an await point.
    fn diagnostics_of(&self, uri: &Url) -> (HashMap<Url, Vec<Diagnostic>>, HashSet<Url>) {
        let mut groups: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
        {
            let mut workspace = lock(&self.workspace);
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
        let published = self
            .published
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        (groups, published)
    }

    async fn publish(&self, uri: &Url) {
        let (groups, published) = self.diagnostics_of(uri);
        // a file that no longer has errors still needs an empty list, otherwise
        // the client keeps showing the previous run
        for stale in published.iter() {
            if !groups.contains_key(stale) {
                let _ = self
                    .client
                    .publish_diagnostics(stale.clone(), vec![], None)
                    .await;
            }
        }
        for (target, diagnostics) in &groups {
            let _ = self
                .client
                .publish_diagnostics(target.clone(), diagnostics.clone(), None)
                .await;
        }
        *self.published.lock().unwrap_or_else(|e| e.into_inner()) = groups.into_keys().collect();
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
        self.publish(&uri).await;
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
        if let Some(doc) = kind
            .is_keyword()
            .then_some(())
            .and_then(|_| keyword_doc(kind.name()))
            .or_else(|| {
                let TokenKind::Ident(name) = kind else {
                    return None;
                };
                type_doc(&Type::from_keyword(name)?).or_else(|| {
                    idx.checked_sub(1)
                        .and_then(|i| analysis.tokens.tokens.get(i))
                        .filter(|(k, _)| matches!(k, TokenKind::Less))
                        .and_then(|_| attr_doc(name))
                })
            })
        {
            return Ok(Some(convert::to_doc_hover(&text, token, doc)));
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
        let ty = table.symbol_at_span(*span).or_else(|| {
            analysis
                .scope
                .uses
                .get(span)
                .and_then(|id| table.symbol_by_id(*id))
        });
        // what the type parameters of a generic call solved to, plus a `where`
        // line for the ones declared with a bound. This is an addition to the
        // usual hover, never a replacement: the resolved signature matters more.
        let generic_note = generic_binding_note(&analysis, *span);
        if let Some(symbol) = ty {
            if let SymbolType::TypeFunction(_) | SymbolType::InlineTypeFunction(_) =
                symbol.symbol_type
            {
                let name = ident_name(kind).unwrap_or_default();
                let mut value = match symbol.ty.as_deref().filter(|t| !t.is_empty()) {
                    Some(ty) => format!("```duka\n(type function) {name}{ty}\n```"),
                    None => format!("```duka\n(type function) {name}\n```"),
                };
                if let Some(doc) = type_builtin_doc(name) {
                    value.push_str(&format!("\n{doc}"));
                }
                return Ok(Some(convert::to_markup_hover(&text, token, &value)));
            }
        }
        // a type parameter in `<T: num>` is a declaration, not a reference
        if type_param_spans(&analysis.tokens.tokens).contains(span) {
            let name = ident_name(kind).unwrap_or_default();
            let mut value = format!("```duka\n(type parameter) {name}\n```");
            let line = &convert::lines_of(&text)[(span.start.line as usize).saturating_sub(1)];
            // the bound is whatever follows the name inside `<...>`; a
            // parameter without one leaves nothing after the colon
            if let Some(bound) = line
                .split_once(&format!("{name}:"))
                .map(|(_, rest)| rest.split([',', '>']).next().unwrap_or("").trim())
                .filter(|b| !b.is_empty())
            {
                value.push_str(&format!("\n\nbound: `{bound}`"));
            }
            return Ok(Some(convert::to_markup_hover(&text, token, &value)));
        }

        if let Some((owner, field)) = record_field_owner(&analysis.tokens.tokens)
            .get(span)
            .cloned()
        {
            // the alias carries the record as a structured type, so the field
            // type comes from there rather than from re-parsing text
            let detail = analysis
                .scope
                .symbols
                .lookup_named(&owner)
                .and_then(|s| s.ty_value.as_deref())
                .and_then(|t| convert::member_of_type(t, &field))
                .map(|t| t.to_string());
            if let Some(detail) = detail {
                return Ok(Some(convert::to_member_hover(
                    &text, token, &owner, &detail,
                )));
            }
        }

        if let Some(symbol) = ty
            && param_spans(&analysis.tokens.tokens).contains(&symbol.span)
        {
            let mut hover = convert::to_param_hover(&text, token, symbol.ty.as_deref());
            if let Some(note) = &generic_note {
                convert::append_markup(&mut hover, note);
            }
            return Ok(Some(hover));
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
                });
            let Some(decl) = decl else { continue };
            // the declaration itself is not a call site
            if decl.start == span.start {
                continue;
            }
            if decl.start.line >= text.lines().count() as u32 {
                continue;
            }
            let Some(params) = param_names(&analysis.tokens.tokens, decl) else {
                continue;
            };
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

        let mut items: Vec<CompletionItem> = vec![];
        let mut truncated = false;

        'scopes: for scope in analysis.scope.symbols.scopes.iter().rev() {
            for (name, syms) in &scope.symbols {
                if items.len() >= MAX_COMPLETION_ITEMS {
                    truncated = true;
                    break 'scopes;
                }
                let Some(sym) = syms.last() else { continue };
                let kind = match sym.symbol_type {
                    duka_shared::utils::SymbolType::Function
                    | duka_shared::utils::SymbolType::TypeFunction(_)
                    | duka_shared::utils::SymbolType::InlineTypeFunction(_) => {
                        CompletionItemKind::FUNCTION
                    }
                    duka_shared::utils::SymbolType::ObjectClass(_) => CompletionItemKind::CLASS,
                    duka_shared::utils::SymbolType::TypeAlias(_) => CompletionItemKind::STRUCT,
                    duka_shared::utils::SymbolType::Constant(_) => CompletionItemKind::CONSTANT,
                    _ => CompletionItemKind::VARIABLE,
                };
                items.push(CompletionItem {
                    label: name.to_string(),
                    kind: Some(kind),
                    detail: sym.ty.as_deref().map(|t| t.to_string()),
                    ..Default::default()
                });
            }
        }

        for doc in duka_shared::docs::KEYWORD_DOCS {
            let duka_shared::docs::KeywordDoc::Keyword { keyword, doc } = doc else {
                continue;
            };
            if items.len() >= MAX_COMPLETION_ITEMS {
                truncated = true;
                break;
            }
            items.push(CompletionItem {
                label: (*keyword).to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(doc.title.to_string()),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.content.to_string(),
                })),
                ..Default::default()
            });
        }

        if truncated {
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: true,
                items,
            })));
        }
        Ok(Some(CompletionResponse::Array(items)))
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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

    let mut note = String::from("\n\n---\n\n```duka\n");
    for b in bindings {
        note.push_str(&format!("{} = {}\n", b.name, b.value));
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
        note.push_str(&format!("where {}", bounds.join(", ")));
    }
    note.push_str("\n```");
    Some(note)
}

/// Spans of the type parameters declared in `<...>` of a function or type
/// function, so hovering `T` can describe the parameter rather than resolving a
/// symbol that does not exist.
fn type_param_spans(tokens: &[duka_frontend::lexer::token::Token]) -> HashSet<Span> {
    let mut out = HashSet::new();
    for (i, (kind, _)) in tokens.iter().enumerate() {
        if !matches!(kind, TokenKind::Function | TokenKind::Fn) {
            continue;
        }
        let Some(open) = tokens[i + 1..]
            .iter()
            .position(|(k, _)| matches!(k, TokenKind::Less))
        else {
            continue;
        };
        let mut depth = 0usize;
        for (kind, span) in tokens.iter().skip(i + 1 + open) {
            match kind {
                TokenKind::Less => depth += 1,
                TokenKind::Greater => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Ident(_) if depth == 1 => {
                    out.insert(*span);
                }
                _ => {}
            }
        }
    }
    out
}

/// Field declarations inside a record type, mapped to the type they belong to:
/// `type Obj = { a: int }` gives `a -> ("Obj", "a")`. Without this the fields
/// of every record type resolved to nothing, since a field is not a symbol.
fn record_field_owner(
    tokens: &[duka_frontend::lexer::token::Token],
) -> HashMap<Span, (String, String)> {
    let mut out = HashMap::new();
    for (i, (kind, _)) in tokens.iter().enumerate() {
        if !matches!(kind, TokenKind::Type) {
            continue;
        }
        let Some((TokenKind::Ident(name), _)) = tokens.get(i + 1) else {
            continue;
        };
        let owner = name.to_string();
        // `type Name = { ... }` has an `=` (and maybe `extends`) before the brace
        let Some(brace) = tokens[i + 2..]
            .iter()
            .position(|(k, _)| matches!(k, TokenKind::LBrace))
        else {
            continue;
        };
        let mut depth = 0usize;
        for (j, (kind, span)) in tokens.iter().skip(i + 2 + brace).enumerate() {
            match kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Ident(field) if depth == 1 => {
                    let declares = matches!(
                        tokens.get(i + 2 + brace + j + 1).map(|(k, _)| k),
                        Some(TokenKind::Colon)
                    );
                    if declares {
                        out.insert(*span, (owner.clone(), field.to_string()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The doc the builtin table carries for a type-context builtin such as
/// `Error` or `Assert`, keyed by name.
fn type_builtin_doc(name: &str) -> Option<&'static str> {
    use duka_frontend::analyzer::builtin::TYPE_BUILTINS_META;
    use duka_shared::docs::MetaItemInfo;
    let MetaItemInfo::Module { inner } = &TYPE_BUILTINS_META.info else {
        return None;
    };
    inner.iter().find(|m| m.name == name).map(|m| m.doc)
}

/// Spans of every declared parameter, found by walking each function
/// signature's parentheses. The symbol table has no parameter kind, so a
/// parameter would otherwise hover as the local it technically is.
fn param_spans(tokens: &[duka_frontend::lexer::token::Token]) -> HashSet<Span> {
    let mut out = HashSet::new();
    for (i, (kind, _)) in tokens.iter().enumerate() {
        if !matches!(kind, TokenKind::Function | TokenKind::Fn) {
            continue;
        }
        let Some(open) = tokens[i + 1..]
            .iter()
            .position(|(k, _)| matches!(k, TokenKind::LParen))
        else {
            continue;
        };
        let mut depth = 0usize;
        let mut annotated = false;
        for (kind, span) in tokens.iter().skip(i + 1 + open) {
            match kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::Colon => annotated = true,
                TokenKind::Comma => annotated = false,
                TokenKind::Ident(_) if !annotated && depth == 1 => {
                    out.insert(*span);
                }
                _ => {}
            }
        }
    }
    out
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
    fn keyword_doc_hover_is_available() {
        let doc = keyword_doc("if").expect("if doc");
        assert_eq!(doc.title, "if");
        let text = "if true then end\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 0,
            character: 0,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("if token");
        let hover = convert::to_doc_hover(text, token, doc);
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("```duka"), "{value}");
        assert!(value.contains("if"), "{value}");
    }

    #[test]
    fn type_keyword_doc_hover_is_available() {
        let ty = Type::from_keyword("int").expect("int type");
        let doc = type_doc(&ty).expect("int doc");
        let token = &duka_frontend::lexer::token::EMPTY_TOKEN;
        let hover = convert::to_doc_hover("int", token, doc);
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("Integer"), "{value}");
    }

    #[test]
    fn attr_doc_hover_is_available() {
        let doc = attr_doc("const").expect("const doc");
        assert!(doc.content.contains("immutable"));
        let token = &duka_frontend::lexer::token::EMPTY_TOKEN;
        let value = match convert::to_doc_hover("<const>", token, doc).contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("```duka"), "{value}");
    }
}
