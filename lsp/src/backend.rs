//! The Duka language server backend.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use duka_lib::duka_frontend::analyzer::ObjectType;
use duka_lib::duka_frontend::lexer::token::TokenKind;
use duka_lib::duka_shared::constants::csugar;
use duka_lib::duka_shared::{dtype::Type, errors::Span, utils::SymbolType};
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

/// Which generated file the editor is asking about.
///
/// The uri arrives as an object rather than a bare string because that is what a
/// named parameter is: a request whose params are not an object gets wrapped in
/// an array on the way out, and this server would then be asked to read a list
/// where a uri is.
#[derive(serde::Deserialize)]
pub struct DeclaredFileParams {
    pub uri: String,
}
impl Backend {
    /// The text of one of the generated declaration files.
    ///
    /// The editor cannot read a file that does not exist, so it asks for the text
    /// and serves it from its own content provider. Nothing is written: these
    /// files are what the standard library looks like as declarations, and the
    /// only copy of them is the answer to this request.

    pub async fn declared_file(&self, params: DeclaredFileParams) -> Result<Option<String>> {
        let name = params
            .uri
            .split_once("duka-virtual:")
            .map(|(_, rest)| rest)
            .unwrap_or_default()
            .trim_start_matches('/');
        Ok(crate::declared::generated()
            .into_iter()
            .find(|file| file.name == name)
            .map(|file| file.text))
    }

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

    /// The inlay hints of one document: what a declaration inferred, what each
    /// call site's parameters are called, and where a `goto` lands.
    fn inlay_hints_for(
        analysis: &compile::DocAnalysis,
        text: &str,
        range: Range,
    ) -> Vec<InlayHint> {
        let index = convert::LineIndex::new(text);
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
            let Some(ty) = symbol.ty_display.as_deref().filter(|t| !t.is_empty()) else {
                continue;
            };
            if !matches!(
                symbol.symbol_type,
                SymbolType::Variable | SymbolType::Constant(_)
            ) {
                continue;
            }
            let position = index.position(text, span.end.line, span.end.column);
            if !in_range(index.range(text, *span)) {
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
            // `f.<T>(x)` writes the type arguments between the name and the
            // call, so the paren has to be found past them
            let mut paren = i + 1;
            if matches!(
                analysis.tokens.tokens.get(paren).map(|(t, _)| t),
                Some(TokenKind::Dot)
            ) && matches!(
                analysis.tokens.tokens.get(paren + 1).map(|(t, _)| t),
                Some(TokenKind::Less)
            ) {
                paren += 2;
                let mut depth = 1usize;
                while let Some((tk, _)) = analysis.tokens.tokens.get(paren) {
                    match tk {
                        TokenKind::Less => depth += 1,
                        TokenKind::Greater => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    paren += 1;
                }
                if depth != 0 {
                    continue;
                }
                paren += 1;
            }
            let Some((TokenKind::LParen, _)) = analysis.tokens.tokens.get(paren) else {
                continue;
            };
            // A standard library function has no declaration to read the
            // parameter names off, its signature is where they live. A `@declare`
            // has one and it already names them, so the hint would repeat what is
            // written on the declaration a few lines away.
            if in_a_declaration(text, *span) {
                continue;
            }
            let params =
                docs::value_builtin_params(&builtin_path(text, &index, *span)).or_else(|| {
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
            for (j, (k, s)) in analysis.tokens.tokens.iter().enumerate().skip(paren) {
                // an argument may open a bracket of its own: `[1]` deepens the
                // walk on its first token, which is the token the hint belongs on
                let opener = matches!(
                    k,
                    TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace
                );
                let outer = depth;
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
                let at_args = if opener { outer == 1 } else { depth == 1 };
                if !at_args {
                    continue;
                }
                let starts_arg = j == paren + 1
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
                let position = index.position(text, s.start.line, s.start.column);
                if !in_range(index.range(text, *s)) {
                    continue;
                }
                hints.push(InlayHint {
                    position,
                    label: InlayHintLabel::String(format!("{name}: ")),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(false),
                    padding_right: Some(false),
                    data: None,
                });
            }
        }

        // where a `goto` lands, which is the one thing a label has that the
        // source does not already show
        for (use_span, decl) in analysis.scope.symbols.goto_targets() {
            if !in_range(index.range(text, use_span)) {
                continue;
            }
            hints.push(InlayHint {
                position: index.position(text, use_span.end.line, use_span.end.column),
                label: InlayHintLabel::String(format!(" -> line {}", decl.start.line)),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::String(format!(
                    "go to line {}",
                    decl.start.line
                ))),
                padding_left: Some(false),
                padding_right: Some(true),
                data: None,
            });
        }

        hints.sort_by_key(|h| (h.position.line, h.position.character));
        hints
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
                                    SemanticTokenType::DECORATOR,
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
        // A builtin is written in Rust, so there is no span in this document for
        // it and nothing to jump to here. What it does have is a place of its own
        // -- a generated file of declarations -- so that is where it goes.
        //
        // The name has to be the path it is written with. `os.exit` is declared in
        // the file about `os`, and the bare `exit` is declared nowhere: it is only
        // an `os` member, so looking it up on its own finds nothing and leaves the
        // reader with no way to move.
        if let TokenKind::Ident(name) = &token.0 {
            let name = name.clone();
            let line_index = convert::LineIndex::new(&text);
            let path =
                member_path_before(&line_index, &text, token.1.start.line, token.1.start.column)
                    .map(|base| format!("{base}.{name}"))
                    .unwrap_or(name);
            if let Some((uri, line)) = crate::declared::locate(&path) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri,
                    range: Range::new(Position::new(line, 0), Position::new(line, u32::MAX)),
                })));
            }
        }
        // `Span::EMPTY` is what a builtin and a `@declare` both carry, and its
        // line is 1 -- so a span that merely reads "inside this file" lets those
        // through and lands the reader on the first character of the first line.
        let here = |span: &duka_lib::duka_shared::errors::Span| {
            span != &Span::EMPTY && span.start.line < line_count
        };
        for link in &analysis.scope.links {
            if link.name_span == token.1 && here(&link.decl_span) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range: convert::lsp_range(&text, link.decl_span),
                })));
            }
        }
        let table = &analysis.scope.symbols;
        // a `goto` lands on the label it names, and labels live beside the
        // symbols in the table rather than among them
        if table.goto_label_at(token.1).is_some()
            && let Some(decl) = table.goto_target(token.1)
            && here(&decl)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, decl),
            })));
        }
        let sym = resolve_symbol(analysis, token.1);
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
            // Off every token there is no name to ask about, so the answer is
            // what the file as a whole documented.
            return Ok(analysis.scope.chunk_doc.as_ref().map(|doc| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("---\n{doc}\n---\n"),
                }),
                range: None,
            }));
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
                duka_lib::duka_shared::dtype::Type::from_keyword(name)
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
            object_member(analysis, *span, ident_name(kind).unwrap_or_default())
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
            if let Some(container) = path_type(analysis, &base)
                && let Some((_, detail)) = convert::record_members(&container)
                    .into_iter()
                    .find(|(name, _)| Some(name.as_str()) == ident_name(kind))
            {
                // a member reached through a `require` may be one the module only
                // declared, and `Type` has no room to say so
                let declared = ident_name(kind)
                    .and_then(|member| module_is_declared(analysis, &base, member))
                    .unwrap_or(false);
                return Ok(Some(convert::member_hover(
                    &text, token, &base, &detail, declared,
                )));
            }
            // an object value carries its type as a class, and a class that came
            // through a `require` is read out of the module that declared it, so
            // the members come from that class rather than from record text
            if let Some(resolved) = path_resolved(analysis, &base)
                && let Some((class, objects)) = resolved.class_table(analysis)
                && let Some((owner, detail)) =
                    member_of_class(objects, class, ident_name(kind))
            {
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
        // A member read in a type position binds nothing, so there is no symbol
        // for any of the steps below to find. The pass that resolved it recorded
        // what it resolved to, and that is the answer -- which is also how
        // `RequireType("m").Alias`, a name in a file this one has never read,
        // gets described at all.
        if let Some(ty) = analysis.scope.type_at_span.get(span) {
            let name = ident_name(kind).unwrap_or_default();
            return Ok(Some(convert::to_markup_hover(
                &text,
                token,
                &format!("```duka\n(type) {name}: {ty}\n```"),
            )));
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
        let generic_note = generic_binding_note(analysis, *span);
        let call_bindings = generic_call_bindings(analysis, *span);
        let name = ident_name(kind).unwrap_or_default();
        if let Some(symbol) = ty
            && let Some(hover) = symbol_hover(
                analysis,
                &text,
                token,
                symbol,
                name,
                generic_note.as_deref(),
                call_bindings,
            )
        {
            return Ok(Some(hover));
        }
        // a label and a `goto` are neither symbols nor members, so they are
        // described from what the analyser recorded about the pair
        if let Some(hover) = label_hover(analysis, &text, *span, token) {
            return Ok(Some(hover));
        }
        // What is left is a name the analyser never resolved: a type context builtin
        // that is not a symbol at all, and a standard library name reached
        // without one. The standard library *does* register symbols for the
        // global half of itself, so this is reached only for the module half
        // and for a name that resolved to a symbol with nothing to say. A local
        // of the same name would have resolved above and shadows them, which is
        // the order the language gives.
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
                            detail: symbol
                                .and_then(|s| s.ty_display.as_deref().map(str::to_string)),
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
        // a label is not a symbol, so what references it has are the `goto`s
        // of the same function that land on it
        let label_uses = analysis.scope.symbols.label_uses(*span);
        if !label_uses.is_empty() {
            let mut out: Vec<Location> = label_uses
                .iter()
                .map(|use_span| Location {
                    uri: uri.clone(),
                    range: index.range(&text, *use_span),
                })
                .collect();
            out.sort_by_key(|l| (l.range.start.line, l.range.start.character));
            return Ok(Some(out));
        }

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
        Ok(Some(Backend::inlay_hints_for(
            analysis,
            &text,
            params.range,
        )))
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
                // inside `require "..."` and `RequireType("...")` the useful
                // completion is the module name, everything else stays quiet
                // inside a string
                let at = analysis
                    .tokens
                    .tokens
                    .iter()
                    .position(|(_, s)| s == string_span);
                if !at.is_some_and(|i| names_a_module(&analysis.tokens.tokens, i)) {
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

        if let Some(AccessBase::Path(base)) = member_access_base(&index, &text, pos) {
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
            let Some(resolved) = path_resolved(analysis, &base) else {
                return Ok(None);
            };
            // a class is not record text, so its fields and methods come from the
            // class itself -- which is also how a class that arrived through a
            // `require` is reachable at all
            let members = match resolved.class_table(analysis) {
                Some((class, objects)) => class_members(objects, class),
                None => match resolved.structured.as_deref() {
                    Some(ty) => convert::record_members(ty),
                    None => Vec::new(),
                },
            };
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
        // A type position reads off a module, which no symbol describes
        if let Some(AccessBase::RequireType { module, members }) =
            member_access_base(&index, &text, pos)
        {
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items: require_type_members(analysis, &module, &members),
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
        let (items, truncated) = completion_items(analysis, context, &word);
        if truncated {
            return Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: true,
                items,
            })));
        }
        Ok(Some(CompletionResponse::Array(items)))
    }
}

/// The names `RequireType("m").` can be followed by.
///
/// This lists rather than resolves, which is the whole of it: a module already
/// says which names it answers for in a type position, and the type behind each
/// one was recorded where it was written rather than computed a second time here.
/// What is offered is the name, the kind the module's own symbol table gives it,
/// and where it lives -- a name with a module behind it is not something the
/// reader can find on their own.
fn require_type_members(
    analysis: &compile::DocAnalysis,
    module_name: &str,
    read: &[String],
) -> Vec<CompletionItem> {
    let Some(resolver) = analysis.modules.as_ref() else {
        return vec![];
    };
    let Some(module) = resolver.resolve(&analysis.scope.modules, module_name) else {
        return vec![];
    };
    // a member read off the module narrows it to one type, and the names below
    // that one are the members of that type
    let mut ty: Option<Type> = None;
    for member in read {
        let Some(current) = ty.clone() else {
            let resolved = module
                .exported
                .get(member.as_str())
                .map(|_| Type::Any)
                .or_else(|| module.value.members.get(member.as_str()).cloned());
            ty = resolved;
            continue;
        };
        ty = current.field(member);
    }

    module
        .type_position_names()
        .into_iter()
        .filter_map(|name| {
            let kind = module
                .analysis
                .symbols
                .lookup_anywhere(name)
                .and_then(|sym| completion_kind(&sym.symbol_type, CompletionContext::Type))
                .or(Some(CompletionItemKind::TYPE_PARAMETER))?;
            // below a member read only what is inside that type is on offer
            if let Some(ty) = &ty {
                ty.field(name)?;
            }
            Some(CompletionItem {
                label: name.to_owned(),
                kind: Some(kind),
                detail: Some(format!("{module_name}.{name}")),
                ..Default::default()
            })
        })
        .take(MAX_COMPLETION_ITEMS)
        .collect()
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

            if csugar::HIDDENS.contains(&&**name) {
                continue;
            }

            let Some(sym) = syms.iter().rev().next() else {
                continue;
            };
            let Some(kind) = completion_kind(&sym.symbol_type, context) else {
                continue;
            };
            let kind = if sym.attributes.has_keywordish() {
                // a declaration written `@keywordish` reads as a keyword, so it
                // is offered as one
                CompletionItemKind::KEYWORD
            } else {
                kind
            };
            // a declaration is offered with what it is, so the list answers
            // "which of these can I actually call" without a hover
            let detail = sym.ty_display.as_deref().map(|t| t.to_string());
            let detail = match (&sym.attributes, &detail) {
                (a, _) if a.has_declare() => Some(match detail {
                    Some(ty) => format!("@declare {ty}"),
                    None => "@declare".to_owned(),
                }),
                (_, Some(ty)) => Some(ty.clone()),
                _ => None,
            };
            at.insert(name.to_string(), items.len());
            items.push(CompletionItem {
                label: name.to_string(),
                kind: Some(kind),
                detail,
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
        for doc in duka_lib::duka_shared::docs::KEYWORD_DOCS {
            let duka_lib::duka_shared::docs::KeywordDoc::Keyword { keyword, doc } = doc else {
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
    tokens: &[duka_lib::duka_frontend::lexer::token::Token],
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
    // A generated declaration file is not the reader's code. They opened it
    // because they jumped to a builtin, and an error in it is either a bug in
    // this generator or a diagnostic about a declaration, which is not something
    // the reader can act on.
    if uri.scheme() == crate::declared::SCHEME {
        return (groups, HashSet::new());
    }
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
            let span = err.span();
            let key = (
                span.start.line,
                span.start.column,
                span.end.line,
                span.end.column,
                err.kind().to_string(),
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

/// Whether the name at `span` sits inside a declaration rather than at a call.
///
/// A declaration writes out every parameter name already, so a hint beside one
/// says the reader what the line above them already says. This is read off the
/// text of the line rather than the analysis, because the reader is looking at the
/// declaration when they ask this and the declaration is the line they are on.
fn in_a_declaration(text: &str, span: Span) -> bool {
    let Some(line) = text.lines().nth(span.start.line.saturating_sub(1) as usize) else {
        return false;
    };
    line.contains("@declare")
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
/// Whether the string at `at` is the module name of a `require` or a
/// `RequireType`.
///
/// The two spellings put the name on opposite sides of the argument -- one is a
/// bare call with a string argument, the other a name with a bracketed one --
/// and both mean that a string here is a module. Nowhere else is one worth
/// completing, which is the rest of the rule.
fn names_a_module(tokens: &[duka_lib::duka_frontend::lexer::token::Token], at: usize) -> bool {
    let name_of = |i: usize| match tokens.get(i).map(|(k, _)| k) {
        Some(TokenKind::Ident(n)) => Some(n.as_str()),
        _ => None,
    };
    let preceded_by = |i: usize| match tokens.get(i).map(|(k, _)| k) {
        // `RequireType("m")` and `require("m")` both put a bracket before the name
        Some(TokenKind::LParen) => name_of(i.checked_sub(1)?),
        // `require "m"` has the name before the string and no bracket at all
        _ => name_of(i),
    };
    let Some(before) = at.checked_sub(1) else {
        return false;
    };
    preceded_by(before).is_some_and(|name| {
        name == "require" || name == duka_lib::duka_shared::constants::ctype::REQUIRE
    })
}

/// What a `.` in front of the cursor reads off.
///
/// A dotted path is the ordinary case and needs nothing but the text. The other
/// one is a call: `RequireType("m").Alias` reads off a module and the scanner has
/// to know that a `)` can be what a `.` follows, which is why this is not a
/// `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AccessBase {
    Path(String),
    RequireType {
        module: String,
        /// members already read off the module, nearest last
        members: Vec<String>,
    },
}

fn member_access_base(index: &convert::LineIndex, text: &str, pos: Position) -> Option<AccessBase> {
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
    if path_end < start - 1 {
        return Some(AccessBase::Path(
            chars[path_end..start - 1].iter().collect(),
        ));
    }
    require_type_head(&chars, path_end)
        .map(|(module, members)| AccessBase::RequireType { module, members })
}

/// The `RequireType("m")` ending at `end`, with the module it names and the
/// members already read off it.
///
/// `end` is where the head stops and the `.` would begin. The argument is a
/// string literal, so the name is taken out of the quotes rather than read as an
/// expression -- which is also what makes this the same name `require "m"` uses.
fn require_type_head(chars: &[char], end: usize) -> Option<(String, Vec<String>)> {
    if end == 0 || chars[end - 1] != ')' {
        return None;
    }
    let mut depth = 0i32;
    let mut open = None;
    let mut i = end;
    while i > 0 {
        i -= 1;
        match chars[i] {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(i);
                    break;
                }
            }
            _ => (),
        }
    }
    let open = open?;
    let mut name_start = open;
    while name_start > 0 && is_ident_char(chars[name_start - 1]) {
        name_start -= 1;
    }
    let head: String = chars[name_start..open].iter().collect();
    if head != duka_lib::duka_shared::constants::ctype::REQUIRE {
        return None;
    }
    let argument: String = chars[open + 1..end - 1].iter().collect();
    let module = argument.trim().trim_matches(['"', '\'']).to_owned();
    if module.is_empty() {
        return None;
    }
    // whatever is written before the head and touches it is a chain of members
    let before: String = chars[..name_start]
        .iter()
        .rev()
        .take_while(|c| !c.is_whitespace() && !matches!(**c, '=' | '(' | ',' | '['))
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let members = before
        .split('.')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    Some((module, members))
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
fn table_key_at(tokens: &[duka_lib::duka_frontend::lexer::token::Token], at: usize) -> bool {
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

/// Looks a member up on a class, and says which class answered.
///
/// The class is already resolved by the caller -- from the type, which is the
/// only thing that says which module declared it -- so this only has to look the
/// name up inside it.
fn member_of_class(
    objects: &[ObjectType],
    class: &ObjectType,
    member: Option<&str>,
) -> Option<(String, String)> {
    let member = member?;
    let mut cur = Some(class);
    while let Some(c) = cur {
        for m in &c.members {
            if &*m.name == member {
                return Some((class.name.to_string(), m.ty.to_string()));
            }
        }
        for m in &c.methods {
            if &*m.name == member {
                return Some((
                    class.name.to_string(),
                    Type::Function(Some(m.sig.clone())).to_string(),
                ));
            }
        }
        cur = c.base.and_then(|b| objects.get(b));
    }
    None
}

/// Every name a class offers: its fields, then its methods as functions
fn class_members(objects: &[ObjectType], class: &ObjectType) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur = Some(class);
    while let Some(c) = cur {
        for m in c.members.iter() {
            if !out.iter().any(|(name, _)| name == m.name.as_ref()) {
                out.push((m.name.to_string(), m.ty.to_string()));
            }
        }
        for m in c.methods.iter() {
            if !out.iter().any(|(name, _)| name == m.name.as_ref()) {
                out.push((
                    m.name.to_string(),
                    Type::Function(Some(m.sig.clone())).to_string(),
                ));
            }
        }
        cur = c.base.and_then(|b| objects.get(b));
    }
    out
}

/// The `T = int` / `where T: num` block for a generic call, or `None` when the
/// hovered token is not inside one
/// What the type parameters of a generic call solved to, recorded against the
/// call that produced them. A reader points at `B`, and the recorded span has to
/// contain that.
fn generic_call_bindings(
    analysis: &compile::DocAnalysis,
    span: Span,
) -> Option<&[duka_lib::duka_frontend::analyzer::GenericBinding]> {
    let bindings = analysis
        .scope
        .generic_bindings
        .iter()
        .find(|(call, _)| call.start <= span.start && span.end <= call.end)
        .map(|(_, b)| b.as_slice())
        .filter(|b| !b.is_empty())?;
    Some(bindings)
}

fn generic_binding_note(analysis: &compile::DocAnalysis, span: Span) -> Option<String> {
    use duka_lib::duka_frontend::analyzer::BindingKind;

    let bindings = generic_call_bindings(analysis, span)?;

    // What this call filled in, and what the `where` below the signature had to
    // say. The two are kept apart because they answer different questions: the
    // first is what happened here, the second is what the declaration promised
    // for any call. The list after the name is left out of both -- it holds the
    // same bound the `where` holds when it was written there, and saying it
    // twice would be two answers to one question.
    let mut note = String::from("\n\n---\n\n\n");
    for b in bindings.iter().filter(|b| b.kind == BindingKind::Parameter) {
        note.push_str(&format!("- `{} = {}`\n", b.name, b.value));
    }
    let mut where_line = String::new();
    for b in bindings {
        let clause = match b.kind {
            BindingKind::Parameter => match &b.bound {
                Some(bound) => format!("{}: {}", b.name, bound),
                None => continue,
            },
            BindingKind::TypeBinding => format!("{} = {}", b.name, b.value),
            BindingKind::Concept => b.name.to_string(),
        };
        if !where_line.is_empty() {
            where_line.push_str(", ");
        }
        where_line.push_str(&clause);
    }
    if !where_line.is_empty() {
        note.push('\n');
        note.push_str(&format!("`where {where_line}`"));
    }
    Some(note)
}
/// The signature of a type function, read off the body the analyzer kept.
///
/// A type function's signature is its `FuncBody`: the symbol carries only the
/// index into `type_fns`, and the parameters and the return annotation are read
/// from the body rather than back out of the text.
fn type_fn_signature(
    analysis: &compile::DocAnalysis,
    symbol: &duka_lib::duka_shared::utils::Symbol,
    name: &str,
) -> String {
    use duka_lib::duka_frontend::analyzer::typechecker::normalize_generic_names;
    use duka_lib::duka_frontend::parser::ast::{Param, ReturnAnnotation, TypeDesc};

    // a name in a type position that is one of this function's own parameters is
    // that parameter, not a lookup that finds nothing
    fn type_param_names(body: &duka_lib::duka_frontend::parser::ast::FuncBody) -> Vec<Box<str>> {
        body.1
            .iter()
            .map(
                |duka_lib::duka_frontend::parser::ast::TypeParam((n, _), ..)| {
                    n.as_str().to_owned().into_boxed_str()
                },
            )
            .collect()
    }
    fn show(ty: &TypeDesc, names: &[Box<str>]) -> String {
        let borrowed: Vec<&str> = names.iter().map(|n| &**n).collect();
        let normalized = normalize_generic_names(ty, &borrowed);
        // A rest element has no `Type` to read back out of it -- `as_type`
        // answers `any` for one -- and a signature reads the way it was
        // written, so a `...` inside an annotation stays a `...`.
        if has_splice(&normalized) {
            normalized.to_string()
        } else {
            normalized.as_type().to_string()
        }
    }
    fn has_splice(ty: &TypeDesc) -> bool {
        match ty {
            TypeDesc::Splice(..) => true,
            TypeDesc::Generic(_, args, _) => args.iter().any(has_splice),
            TypeDesc::Array(Some(inner)) => has_splice(inner),
            TypeDesc::Table(k, v) => [k.as_deref(), v.as_deref()]
                .into_iter()
                .flatten()
                .any(has_splice),
            TypeDesc::Union(ts) | TypeDesc::TypeTuple(ts, _) => ts.iter().any(has_splice),
            TypeDesc::TypeTable(fields) => fields.iter().any(|f| has_splice(&f.ty)),
            TypeDesc::Function(Some(ft)) => {
                ft.params.iter().any(has_splice) || ft.returns.iter().any(has_splice)
            }
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                has_splice(inner)
            }
            TypeDesc::Function(None)
            | TypeDesc::Pure(_)
            | TypeDesc::Named(..)
            | TypeDesc::TypeOf(..)
            | TypeDesc::Expr(_)
            | TypeDesc::FnLit(_)
            | TypeDesc::Array(None) => false,
        }
    }
    fn params(list: &[Param], names: &[Box<str>]) -> String {
        list.iter()
            .map(|p| match p {
                Param::Typed((n, _), ty) => format!("{n}: {}", show(ty, names)),
                Param::Name((n, _)) => n.to_string(),
                Param::Var(_, Some((n, _))) => format!("...: {n}"),
                Param::Pack((n, _), _) => format!("...{n}"),
                _ => "...".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn returns(ret: Option<&ReturnAnnotation>, names: &[Box<str>]) -> String {
        match ret {
            None => String::new(),
            Some(r) if r.tys.is_empty() && !r.var_arg => String::new(),
            Some(r) => {
                let mut v = r
                    .tys
                    .iter()
                    .map(|t| show(t, names))
                    .collect::<Vec<_>>()
                    .join(", ");
                if r.var_arg {
                    v.push_str(if v.is_empty() { "..." } else { ", ..." });
                }
                v
            }
        }
    }

    let generics = declared_generics(analysis, symbol).unwrap_or_default();
    let found = match &symbol.symbol_type {
        SymbolType::TypeFunction(id) => analysis.scope.type_fns.get(*id).map(|f| {
            let names = type_param_names(&f.body);
            (
                params(&f.body.0, &names),
                returns(f.body.2.as_ref(), &names),
            )
        }),
        SymbolType::InlineTypeFunction(id) => analysis.scope.inline_type_fns.get(*id).map(|f| {
            // the body is a single type, and a name inside it that the
            // parameters introduce is that parameter -- leaving it unnamed
            // would resolve it to `any` at every use, in the hover too
            let names: Vec<Box<str>> = f
                .params
                .iter()
                .filter_map(|p| match p {
                    Param::Name((n, _)) | Param::Typed((n, _), _) => {
                        Some(n.as_str().to_owned().into_boxed_str())
                    }
                    Param::Var(_, Some((n, _))) | Param::Pack((n, _), _) => {
                        Some(n.as_str().to_owned().into_boxed_str())
                    }
                    _ => None,
                })
                .collect();
            (params(&f.params, &names), show(&f.ret_ty, &names))
        }),
        _ => None,
    };
    let (params, returns) = found.unwrap_or_else(|| {
        (
            symbol.ty_display.clone().unwrap_or_default().into(),
            String::new(),
        )
    });
    match returns.is_empty() {
        true => format!("type function {name}{generics}({params})"),
        false => format!("type function {name}{generics}({params}): {returns}"),
    }
}

/// The `<T, ...V = int>` a declaration was written with, as recorded.
///
/// Placeholders and their defaults, in the order they were written, and nothing
/// else: a bound is a constraint rather than a slot, so it belongs to the `where`
/// line -- which carries all of them, in order, whether they were written after
/// the name or below the signature. The parser folds the two spellings into one
/// so nothing has to answer for both.
///
/// The list is read from the analysis, not from the text: the analyzer keeps each
/// declaration's parameters as they were written, because order and the `...` in
/// front of a parameter are part of what was written and a name-keyed table
/// cannot answer them.
fn declared_generics(
    analysis: &compile::DocAnalysis,
    symbol: &duka_lib::duka_shared::utils::Symbol,
) -> Option<String> {
    let list = analysis
        .scope
        .declared_type_params
        .get(&symbol.id)?
        .iter()
        .map(|p| {
            let mut one = String::new();
            if p.pack {
                one.push_str("...");
            }
            one.push_str(&p.name);
            if let Some(default) = &p.default {
                one.push_str(" = ");
                one.push_str(default);
            }
            one
        })
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then(|| format!("<{list}>"))
}

/// The hover for a symbol that has a shape of its own: a type function, a type
/// parameter, a member or a parameter. Everything else is left to the generic
/// rendering, which reads `SymbolType` on its own.
/// Spells a type out, or names it when spelling it out stops being useful.
///
/// A record renders as one field per line, and a standard library value like
/// `string` is a record of forty methods: printing those turns a hover into a
/// wall of text that says less than the name does. A small record still gets
/// written out, because an anonymous one has no name to fall back on.
fn type_label<'a>(name: &'a str, rendered: &'a str) -> Cow<'a, str> {
    let fields = rendered.lines().count();
    if fields > 6 {
        Cow::Borrowed(name)
    } else {
        Cow::Borrowed(rendered)
    }
}

fn symbol_hover(
    analysis: &compile::DocAnalysis,
    text: &str,
    token: &duka_lib::duka_frontend::lexer::token::Token,
    symbol: &duka_lib::duka_shared::utils::Symbol,
    name: &str,
    generic_note: Option<&str>,
    call_bindings: Option<&[duka_lib::duka_frontend::analyzer::GenericBinding]>,
) -> Option<Hover> {
    let mut hover = match &symbol.symbol_type {
        SymbolType::TypeFunction(_) | SymbolType::InlineTypeFunction(_) => {
            let mut view = docs::DocView {
                name: Cow::Borrowed(name),
                signature: Some(type_fn_signature(analysis, symbol, name)),
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
        SymbolType::TypeBinding { source } => {
            // Reads as a binding rather than a parameter: `where type V = E` is
            // not a slot with a bound and a default, it is a name standing for a
            // type that was written somewhere above.
            let value = format!("```duka\n(type binding) {name} -- from {source}\n```");
            convert::to_markup_hover(text, token, &value)
        }
        SymbolType::Field(declared) => {
            let owner = symbol
                .owner
                .and_then(|id| analysis.scope.symbols.symbol_by_id(id))
                .map(|owner| symbol_name(analysis, owner))
                .unwrap_or_default();
            let rendered = symbol.ty_display.as_deref().unwrap_or_default();
            let detail = if rendered.is_empty() {
                declared.to_string()
            } else {
                rendered.to_string()
            };
            convert::to_member_hover(text, token, &owner, &detail)
        }
        SymbolType::Parameter(_) => {
            convert::to_param_hover(text, token, symbol.ty_display.as_deref())
        }
        SymbolType::Function => {
            // The type parameters belong in the signature, not in a footnote.
            //
            // They are written in the declaration right after the name, so
            // `function B<T: [V], V>` reads as `B<T, V>(...)` -- the same
            // generic name you would write to call it. Leaving them out made the
            // signature read as a monomorphic function, which is a different
            // function: `B` here accepts a `[int]` and a `[string]` alike, and
            // the hover said it took exactly one shape.
            let generics = declared_generics(analysis, symbol);
            // At a call the signature that answers is the one this call produced,
            // not the one the declaration was written with -- so the solution goes
            // into the analysed type, which is the only one of the two a type can
            // be walked through.
            let called = call_bindings
                .and_then(|bs| {
                    let bindings: std::collections::HashMap<_, _> = bs
                        .iter()
                        .filter_map(|b| {
                            b.value_type
                                .as_deref()
                                .cloned()
                                .map(|t| (b.name.clone(), t))
                        })
                        .collect();
                    (!bindings.is_empty()).then_some(bindings)
                })
                .map(|bindings| duka_lib::duka_frontend::solver::Solution {
                    bindings,
                    ..Default::default()
                })
                .and_then(|solution| {
                    symbol
                        .ty
                        .as_deref()
                        .map(|ty| solution.substitute(ty).to_string())
                });
            let rendered = called.or_else(|| {
                symbol
                    .ty_display
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
            });
            let signature = match (generics, rendered) {
                (Some(g), Some(ty)) => format!("{name}{g}: {}", type_label(name, &ty)),
                (Some(g), None) => format!("{name}{g}"),
                (None, Some(ty)) => format!("{name}: {}", type_label(name, &ty)),
                (None, None) => name.to_string(),
            };
            // A function the runtime provides is registered in the global scope,
            // so it *is* a symbol and this branch is reached -- which is why its
            // documentation has to be read here. A module builtin never gets this
            // far: `os.exit` is a member, read through the path.
            //
            // An empty span is what tells them apart: a symbol written in the
            // source has one.
            if symbol.span == Span::EMPTY
                && let Some(builtin) = docs::value_builtin_view(name)
            {
                let mut hover = convert::to_markup_hover(text, token, &builtin.render());
                // a builtin is a declaration in the only sense the language has
                // for it: something exists, and nothing here is built
                convert::append_markup(&mut hover, convert::DECLARED_NOTE);
                if let Some(note) = generic_note {
                    convert::append_markup(&mut hover, note);
                }
                return Some(hover);
            }
            let mut hover =
                convert::to_markup_hover(text, token, &format!("```duka\n{signature}\n```"));
            if symbol.attributes.has_declare() {
                // the signature is the whole of a declaration, so the note says
                // what the absence of a body means rather than what the body was
                convert::append_markup(&mut hover, convert::DECLARED_NOTE);
            }
            hover
        }
        SymbolType::Variable | SymbolType::Constant(_) => {
            // A module is a value whose type is a record of what is in it, so it
            // reads through the same builtin documentation a function does. Without
            // this a module says nothing about itself: the record is too large to
            // write out, and its own name is all that would be left.
            if symbol.span == Span::EMPTY
                && let Some(builtin) = docs::value_builtin_view(name)
            {
                let mut hover = convert::to_markup_hover(text, token, &builtin.render());
                convert::append_markup(&mut hover, convert::DECLARED_NOTE);
                return Some(hover);
            }
            let ty = symbol.ty_display.as_deref().filter(|t| !t.is_empty());
            let signature = match ty {
                Some(ty) => format!("{name}: {}", type_label(name, ty)),
                // a name the pass could not type yet is still a name, and showing
                // it beats showing nothing
                None => name.to_string(),
            };
            convert::to_markup_hover(text, token, &format!("```duka\n{signature}\n```"))
        }
        _ => return None,
    };
    if let Some(note) = generic_note {
        convert::append_markup(&mut hover, note);
    }
    // What the declaration had to say about itself, carried on the symbol so it
    // answers the same whether the cursor sits on the declaration or a use.
    if let Some(doc) = symbol.attributes.get_doc() {
        convert::append_markup(&mut hover, &format!("\n\n---\n{doc}\n---\n\n"));
    }
    Some(hover)
}

/// The declaration a position is about: the token's own declaration, else a use
/// recorded for that token.
fn resolve_symbol<'a>(
    analysis: &'a compile::DocAnalysis,
    span: Span,
) -> Option<&'a duka_lib::duka_shared::utils::Symbol> {
    let table = &analysis.scope.symbols;
    table
        .symbol_at_span(span)
        .or_else(|| analysis.scope.uses.get(&span).and_then(|id| table.symbol_by_id(*id)))
}

/// The name a symbol was written under, read back from the token that carries
/// its declaration span.
fn symbol_name(
    analysis: &compile::DocAnalysis,
    symbol: &duka_lib::duka_shared::utils::Symbol,
) -> String {
    analysis
        .tokens
        .tokens
        .iter()
        .find(|(_, span)| *span == symbol.span)
        .and_then(|(kind, _)| ident_name(kind))
        .unwrap_or_default()
        .to_string()
}

/// A label, or a `goto` and where it lands. Neither is a symbol, so what the
/// table recorded about them is all there is to describe them with.
fn label_hover(
    analysis: &compile::DocAnalysis,
    text: &str,
    span: Span,
    token: &duka_lib::duka_frontend::lexer::token::Token,
) -> Option<Hover> {
    let table = &analysis.scope.symbols;
    if let Some(name) = table.label_at(span) {
        let value = format!(
            "```duka\n(label) {name}\n```\n\n`label`\n\nReached by {} `goto`.",
            table.label_uses(span).len()
        );
        return Some(convert::to_markup_hover(text, token, &value));
    }
    let name = table.goto_label_at(span)?;
    let target = match table.goto_target(span) {
        Some(decl) => format!("Jumps to line {}.", decl.start.line),
        None => "No label of that name is in reach.".to_owned(),
    };
    Some(convert::to_markup_hover(
        text,
        token,
        &format!("```duka\n(goto) {name}\n```\n\n`goto`\n\n{target}"),
    ))
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
fn param_names(
    tokens: &[duka_lib::duka_frontend::lexer::token::Token],
    decl: Span,
) -> Option<Vec<String>> {
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

/// Whether `container.member` is a member another module only declared.
///
/// `require "m"` names a module and the map of modules is keyed by the path that
/// name resolved to, so this has to ask the provider again rather than look the
/// name up -- the same question the type checker asks when it reads the same
/// member. `container` is dotted, so only the first segment is a module name and
/// the last is the member; anything longer is a record or an object on the way to
/// one, which is not a module and says nothing.
fn module_is_declared(
    analysis: &compile::DocAnalysis,
    container: &str,
    member: &str,
) -> Option<bool> {
    let mut segments = container.split('.');
    let name = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let resolver = analysis.modules.as_ref()?;
    let module = resolver.resolve(&analysis.scope.modules, name)?;
    Some(module.value_is_declared(member))
}

/// The class a type names, whichever module declared it.
///
/// A class that crossed a `require` says which module's table of classes its id
/// indexes, and the answer is read from there rather than from this file. Two
/// files may both declare a `Point` -- and both number it `0` -- so looking a
/// class up by name would answer with whichever came first.
fn class_table<'a>(
    analysis: &'a compile::DocAnalysis,
    ty: &Type,
) -> Option<(&'a ObjectType, &'a [ObjectType])> {
    match ty {
        Type::Union(ts) => ts.iter().find_map(|t| class_table(analysis, t)),
        Type::Object { id, module, .. } => match module {
            None => Some((analysis.scope.objects.get(*id)?, &analysis.scope.objects)),
            Some(key) => {
                let m = analysis.scope.modules.get(key.as_ref())?;
                Some((m.analysis.objects.get(*id)?, &m.analysis.objects))
            }
        },
        _ => None,
    }
}

/// A dotted path resolved to a member, as text and as a real type.
///
/// Both are answers to one question and they are found together: a record is a
/// table with literal keys, so the analyser already holds the type and the text
/// is only its rendering. Which one a caller needs depends on what it is looking
/// at -- text is enough to read a record, and a class is only reachable through
/// the type.
struct Resolved {
    structured: Option<std::sync::Arc<Type>>,
}

impl Resolved {
    fn class_table<'a>(
        &self,
        analysis: &'a compile::DocAnalysis,
    ) -> Option<(&'a ObjectType, &'a [ObjectType])> {
        class_table(analysis, self.structured.as_ref()?)
    }
}

/// Resolves `a.b.c` to the type of the member.
///
/// The walk is on the analysed type, which is the only place the field names are
/// unambiguous. Reading them out of the rendered text instead is how a member
/// whose type happens to contain a brace went missing from a completion.
fn path_resolved(analysis: &compile::DocAnalysis, path: &str) -> Option<Resolved> {
    let mut ty: Option<std::sync::Arc<Type>> = None;
    for (i, seg) in path.split('.').enumerate() {
        let next = match i {
            0 => analysis.scope.symbols.lookup_unambiguous(seg)?.ty.clone(),
            _ => field_through(ty.as_deref()?, seg).map(std::sync::Arc::new),
        };
        ty = Some(next?);
    }
    let structured = ty?;
    Some(Resolved {
        structured: Some(structured),
    })
}

/// The type of a named field, looking through a union so a nilable record still
/// answers with the field its non-nil arm carries.
fn field_through(ty: &Type, name: &str) -> Option<Type> {
    match ty {
        Type::Union(items) => items.iter().find_map(|item| field_through(item, name)),
        _ => ty.field(name),
    }
}

/// The type of `a.b.c`.
fn path_type(analysis: &compile::DocAnalysis, path: &str) -> Option<std::sync::Arc<Type>> {
    path_resolved(analysis, path).and_then(|r| r.structured)
}

#[cfg(test)]
mod tests {
    use crate::declared::SCHEME;

    use super::*;

    fn analyze(text: &str) -> compile::DocAnalysis {
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        compile::analyze(text, "test.duka", None, &HashMap::new(), &mut cache)
    }

    #[test]
    fn a_call_with_type_arguments_gets_parameter_hints() {
        let src = "local function push<T>(arr: array<T>, val: T)\nend\nlocal t = { push = push }\nt.push.<string>([1], 13)\nreturn 0\n";
        let analysis = analyze(src);
        let range = Range {
            start: Position::new(0, 0),
            end: Position::new(100, 0),
        };
        let hints = Backend::inlay_hints_for(&analysis, src, range);
        let params: Vec<_> = hints
            .iter()
            .filter(|h| matches!(h.kind, Some(InlayHintKind::PARAMETER)))
            .collect();
        let label = |h: &&InlayHint| match &h.label {
            InlayHintLabel::String(s) => s.clone(),
            other => format!("{other:?}"),
        };
        assert_eq!(params.len(), 2, "{params:?}");
        assert_eq!(label(&params[0]), "arr: ");
        assert_eq!(params[0].position, Position::new(3, 16));
        assert_eq!(label(&params[1]), "val: ");
        assert_eq!(params[1].position, Position::new(3, 21));
    }

    #[test]
    fn a_destructured_name_is_told_the_type_of_the_field_it_read() {
        // A destructuring reads its pattern apart against one value, so a name
        // takes what that value holds at that place. Reading it off the expanded
        // shape instead gives `any`, because that shape reads the pattern into
        // `tmp.a` and `tmp` is a table of no particular type.
        for (src, at, name, want) in [
            (
                "local { a, b } = { a = true, b = 1 }\nreturn a\n",
                "a, b",
                "a",
                "a: bool",
            ),
            (
                "local { a, b } = { a = true, b = 1 }\nreturn a\n",
                "b }",
                "b",
                "b: int",
            ),
            (
                "local { inner } = { inner = { a = true } }\nreturn a\n",
                "inner }",
                "inner",
                "inner: {",
            ),
        ] {
            let analysis = analyze(src);
            let hover = symbol_hover(
                &analysis,
                src,
                token_of(src, &analysis, at),
                symbol_of(src, &analysis, at),
                name,
                None,
                None,
            )
            .expect("a hover");
            assert!(
                hover_value(&hover).contains(want),
                "{name} should read {want}: {}",
                hover_value(&hover)
            );
        }
    }

    /// An array is one element type rather than a shape, so every name in an
    /// array pattern is told what the list holds.
    #[test]
    fn an_array_destructuring_tells_every_name_the_element_type() {
        let src = "local [ a, b ] = [1, 2]\nreturn a\n";
        let analysis = analyze(src);
        for (at, name) in [("a,", "a"), ("b ]", "b")] {
            let hover = symbol_hover(
                &analysis,
                src,
                token_of(src, &analysis, at),
                symbol_of(src, &analysis, at),
                name,
                None,
                None,
            )
            .expect("a hover");
            assert!(
                hover_value(&hover).contains("int"),
                "{name} should be an int: {}",
                hover_value(&hover)
            );
        }
    }

    #[test]
    fn a_type_function_hover_shows_the_signature_it_was_declared_with() {
        // the signature is the body the analyzer kept, not text read back out:
        // `a` in `Pair(a, b)` is the parameter it was declared beside, and a
        // lookup would find nothing and answer `any`
        //
        // what the function answers is computed from the arguments, so it is
        // `any` here and not the types it was called with
        let src = "type function Pair(a, b): [a, b]\n    return a\nend\nreturn 1\n";
        let markup = hover_markup(src, "Pair", "Pair(a");
        assert!(markup.contains("type function Pair(a, b)"), "{markup}");
    }

    #[test]
    fn an_inline_type_function_hover_shows_its_signature() {
        let src = "type function Pick(a, b) = a\nreturn 1\n";
        let markup = hover_markup(src, "Pick", "Pick(a");
        assert!(markup.contains("type function Pick(a, b)"), "{markup}");
    }

    #[test]
    fn a_variadic_type_function_hover_keeps_its_rest_parameter() {
        let src = concat!(
            "type function FFFF(...ts) = [int, ...ts]\n",
            "type TEST = FFFF(int, false, Iterator(true))\n",
            "return 1\n",
        );
        let markup = hover_markup(src, "FFFF", "FFFF(...");
        assert!(
            markup.contains("type function FFFF(...ts): [int, ...ts]"),
            "{markup}"
        );
    }

    #[test]
    fn a_nested_variadic_generic_call_survives_inlay_hints() {
        let src = concat!(
            "function TESTTEST<...Ts>(...: Ts): ...Ts\n",
            "    return t\n",
            "end\n",
            "\n",
            "local bcbcbcbcb, asdds = TESTTEST(12312, 123123, TESTTEST())\n",
            "return 0\n",
        );
        let analysis = analyze(src);
        let range = Range {
            start: Position::new(0, 0),
            end: Position::new(100, 0),
        };
        let hints = Backend::inlay_hints_for(&analysis, src, range);
        assert!(!hints.is_empty(), "expected hints on the generic call");
    }

    #[test]
    fn a_nested_variadic_generic_call_survives_hover() {
        let src = concat!(
            "function TESTTEST<...Ts>(...: Ts): ...Ts\n",
            "    return t\n",
            "end\n",
            "\n",
            "local bcbcbcbcb, asdds = TESTTEST(12312, 123123, TESTTEST())\n",
            "return 0\n",
        );
        let analysis = analyze(src);
        let markup = hover_markup(src, "TESTTEST", "TESTTEST(12312");
        assert!(markup.contains("TESTTEST"), "{markup}");
        let decl = hover_markup(src, "TESTTEST", "TESTTEST<...");
        assert!(decl.contains("TESTTEST"), "{decl}");
        let _ = generic_binding_note(&analysis, token_of(src, &analysis, "TESTTEST(12312").1);
        let _ = semantics(src, &analysis);
    }

    #[test]
    fn a_nested_variadic_generic_call_survives_diagnostics() {
        let src = concat!(
            "function TESTTEST<...Ts>(...: Ts): ...Ts\n",
            "    return t\n",
            "end\n",
            "\n",
            "local bcbcbcbcb, asdds = TESTTEST(12312, 123123, TESTTEST())\n",
            "return 0\n",
        );
        let analysis = analyze(src);
        let uri = Url::parse("file:///test.duka").expect("uri");
        for err in &analysis.errors {
            let text = convert::error_text(err);
            let _ = convert::error_uri(err, &uri);
            let _ = convert::to_diagnostic(&text, &uri, err);
        }
        let (items, _) = completion_items(&analysis, CompletionContext::Value, "TEST");
        assert!(!items.is_empty(), "expected completion items");
    }

    #[test]
    fn a_nested_variadic_generic_call_survives_incremental_analysis() {
        let src = concat!(
            "function TESTTEST<...Ts>(...: Ts): ...Ts\n",
            "    return t\n",
            "end\n",
            "\n",
            "local bcbcbcbcb, asdds = TESTTEST(12312, 123123, TESTTEST())\n",
            "return 0\n",
        );
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        let first = compile::analyze(src, "test.duka", None, &HashMap::new(), &mut cache);
        let mut edited = src.to_string();
        edited.push_str("-- x\n");
        let second = compile::analyze_incremental(
            &edited,
            "test.duka",
            None,
            &HashMap::new(),
            &mut cache,
            first.parse_cache,
            Some(edited.len() as u32 - 1),
        );
        let _ = &second.errors;
    }

    #[test]
    fn a_type_parameter_in_a_list_is_described_as_a_type_parameter() {
        // `T` and `...V` are both parameters and both say so. A list that the
        // reader was already looking at was no reason to show anything else.
        let src = concat!(
            "function B<T, V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
            "    where T: [V], type C = [V, T]\n",
            "    return 1, false\n",
            "end\n",
        );
        for (at, name) in [("T, V>(a", "T"), ("V>(a: T", "V")] {
            let markup = hover_markup(src, name, at);
            assert!(
                markup.contains(&format!("(type parameter) {name}")),
                "{at}: {markup}"
            );
            assert!(
                !markup.contains('@'),
                "{at} shows the marker's prefix instead of a parameter: {markup}"
            );
        }
    }

    #[test]
    fn a_where_clause_is_answered_in_one_place() {
        // `where T: [V]` and `<T: [V]>` are one statement written two ways, so
        // the answer belongs in one place: the list after the name holds what was
        // written there and the `where` holds every obligation, joined.
        let src = concat!(
            "function B<T, ...V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
            "    where T: [V], type C = [V, T]\n",
            "    return 1, false\n",
            "end\n",
            "B([12312], [])\n",
        );
        let a = analyze(src);
        // the list is what the declaration wrote, and nothing more
        assert_eq!(
            declared_generics(&a, symbol_of(src, &a, "B<T, ...V>(a")).as_deref(),
            Some("<T, ...V>")
        );
        // and the where answers for the bound, the binding, and what the call
        // filled in -- one list, not a bound in two places
        let note = generic_binding_note(&a, token_of(src, &a, "B([12312]").1).expect("a note");
        assert!(
            note.contains("`where T: [int], C = [int, [int]]`"),
            "{note}"
        );
        assert!(note.contains("- `T = [int]`"), "{note}");
        assert!(note.contains("- `V = int`"), "{note}");
        assert!(
            !note.contains("<"),
            "the bound was answered in the list as well: {note}"
        );
    }

    #[test]
    fn a_big_record_is_named_instead_of_written_out() {
        let src = "local big = { a = 1, b = 2, c = 3, d = 4, e = 5, f = 6, g = 7 }\n\
                   return big\n";
        assert!(hover_markup(src, "big", "big").contains("big: big"));

        let small = "local small = { a = 1, b = 2 }\nreturn small\n";
        let markup = hover_markup(small, "small", "small");
        assert!(markup.contains("a: int"), "{markup}");
        assert!(markup.contains("b: int"), "{markup}");
    }

    #[test]
    fn a_class_of_another_module_can_be_applied_to_type_arguments() {
        // `.Pair<int>` rather than a bare `Pair<int>`: the arguments belong to a
        // member of something else, and the dot is what says so.
        let module = concat!(
            "export object Pair\n",
            "    left: any\n",
            "    right: any\n",
            "end\n",
        );
        let text = concat!(
            "local p: RequireType(\"host\").Pair<int> = nil\n",
            "local q: RequireType(\"host\").Pair = nil\n",
            "return p, q\n",
        );
        let analysis = analyze_with_module(text, module);
        // `nil` is not a `Pair`, so the mismatch is the point: both read as the
        // class rather than as `any`, and `any` would accept `nil` and say
        // nothing. Two of them is what says the arguments did not break the path.
        let mismatches: Vec<String> = analysis
            .errors
            .iter()
            .map(|e| e.to_string())
            .filter(|e| e.contains("Pair") && e.contains("nil"))
            .collect();
        assert_eq!(mismatches.len(), 2, "{:?}", analysis.errors);
    }

    #[test]
    fn a_bare_angle_bracket_in_a_value_position_is_a_comparison() {
        // The reason the dot is there. In an expression `f < x` is a comparison,
        // so type arguments on a class are read in a type position only, and a
        // name applied to arguments in a value position is not that name applied.
        let text = "local n = 1\nreturn n < 2\n";
        let a = analyze(text);
        assert!(a.errors.is_empty(), "{:?}", a.errors);
    }

    #[test]
    fn an_underscore_and_a_rest_are_not_names() {
        // Neither is a binding: `_` reads a place and drops it, and `...rest`
        // takes what is left. Read as names, `_` would be in scope afterwards and
        // `...` would be looked up and find nothing.
        let src = "local { a, _ } = { a = 1, b = 2 }\nreturn a\n";
        let a = analyze(src);
        assert!(
            a.scope.symbols.lookup_unambiguous("_").is_none(),
            "`_` must not enter scope"
        );
        assert!(a.scope.symbols.lookup_unambiguous("a").is_some());

        let rest = "local [ a, ...rest ] = xs\nreturn rest\n";
        let a = analyze(rest);
        assert!(
            a.scope.symbols.lookup_unambiguous("rest").is_some(),
            "`...rest` is a binding"
        );
        assert!(
            a.scope.symbols.lookup_unambiguous("...").is_none(),
            "`...` on its own binds nothing"
        );
    }

    #[test]
    fn a_pattern_binds_each_name_to_its_own_place() {
        // `type C = T` and `type [C, _] = T` are different statements: the first
        // binds the whole of `T`, the second binds its first part and throws the
        // rest away. A pattern that gave `C` the whole value in both was reading
        // the name as if no brackets had been written.
        let src = concat!(
            "function f<T, ...V>(a: T): (int, C, ...)\n",
            "    where type [C, _] = [int, [int]]\n",
            "    return 1\n",
            "end\n",
            "f(1)\n",
        );
        let a = analyze(src);
        let note = generic_binding_note(&a, token_of(src, &a, "f(1)").1).expect("a note");
        assert!(note.contains("`where C = int`"), "{note}");
        assert!(!note.contains("[int, [int]]"), "{note}");
    }

    #[test]
    fn the_angle_brackets_hold_slots_and_the_where_line_holds_the_rest() {
        // `<>` is where the slots were written and nothing else; a bound is a
        // constraint, and every constraint is on the one `where` line, in the
        // order it was written. The two places a bound can be written are the
        // same statement, and this is what makes them read the same.
        for (src, list, constraints) in [(
            concat!(
                "function B<T, ...V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
                "    where T: [V], type C = [V, T]\n",
                "    return 1, false\n",
                "end\n",
                "B([12312], [])\n",
            ),
            "<T, ...V>",
            "`where T: [int], C = [int, [int]]`",
        )] {
            let a = analyze(src);
            let at = "B(";
            let token = token_of(src, &a, at);
            let note = generic_binding_note(&a, token.1);
            let markup = match symbol_hover(
                &a,
                src,
                token,
                symbol_of(src, &a, at),
                "B",
                note.as_deref(),
                generic_call_bindings(&a, token.1),
            ) {
                Some(h) => hover_value(&h),
                None => String::new(),
            };
            assert!(markup.contains(list), "{list} is missing from: {markup}");
            assert!(
                markup.contains(constraints),
                "{constraints} missing: {markup}"
            );
            assert!(
                !markup.contains(": [V]>") && !markup.contains(": int>"),
                "a bound is in the list as well as the where line: {markup}"
            );
        }
    }

    #[test]
    fn a_concept_the_call_discharged_is_named_in_the_where() {
        // A concept is not a type, so there is nothing to substitute into it and
        // nothing to print -- but it is one of the obligations the call had to
        // meet, and leaving it out made the where shorter than it was written.
        let src = concat!(
            "function C<T>(a: T): int\n",
            "    where T: int, Sized(T)\n",
            "    return 1\n",
            "end\n",
            "C(1)\n",
        );
        let a = analyze(src);
        let note = generic_binding_note(&a, token_of(src, &a, "C(1").1).expect("a note");
        assert!(note.contains("`where T: int, Sized`"), "{note}");
    }

    #[test]
    fn a_declared_pack_parameter_is_told_apart_from_a_fixed_one() {
        // `...V` binds the rest of the type arguments as a list and `V` binds
        // one of them, which the hover used to flatten into the same `V`.
        let cases = [
            ("function B<T, V>(a: T)\n    return 1\nend\n", "<T, V>"),
            (
                "function B<T, ...V>(a: T)\n    return 1\nend\n",
                "<T, ...V>",
            ),
        ];
        for (src, expected) in cases {
            let a = analyze(src);
            let sym = symbol_of(src, &a, "B<T");
            assert_eq!(
                declared_generics(&a, sym).as_deref(),
                Some(expected),
                "{src}"
            );
        }
    }

    #[test]
    fn a_default_is_not_a_parameter_of_its_own() {
        // The `int` of `<T = int>` ends with the same `>` a parameter name does,
        // so it used to be collected as a second parameter.
        let src = "function B<T = int, ...V>(a: T)\n    return 1\nend\n";
        let a = analyze(src);
        let sym = symbol_of(src, &a, "B<T");
        assert_eq!(
            declared_generics(&a, sym).as_deref(),
            Some("<T = int, ...V>")
        );
    }

    #[test]
    fn a_bound_written_only_in_the_where_is_still_a_bound() {
        // `where T: [V]` is the same statement as `<T: [V]>`. Reading only the
        // list left the parameter looking unconstrained, and left `V` with
        // nothing to solve it from.
        let src = concat!(
            "function B<T, ...V>(a: T): int\n",
            "    where T: [V]\n",
            "    return 1\n",
            "end\n",
            "B([12312])\n",
        );
        let a = analyze(src);
        let bindings = generic_call_bindings(&a, token_of(src, &a, "B([12312]").1)
            .expect("the call solved its parameters");
        let t = bindings.iter().find(|b| &*b.name == "T").expect("T");
        assert_eq!(&*t.value, "[int]", "T is what the argument was");
        assert_eq!(
            t.bound.as_deref(),
            Some("[int]"),
            "the where bound, substituted"
        );
        let v = bindings.iter().find(|b| &*b.name == "V").expect("V");
        assert_eq!(&*v.value, "int", "`T: [V]` with `T = [int]` determines V");
    }

    #[test]
    fn a_call_shows_what_this_call_decided() {
        // The declaration's signature is not the answer to a question about a
        // call: a signature still naming `T` describes a function nobody called.
        let src = concat!(
            "function B<T, ...V>(a: T): int\n",
            "    where T: [V]\n",
            "    return 1\n",
            "end\n",
            "B([12312])\n",
        );
        let a = analyze(src);
        let at = "B([12312]";
        let token = token_of(src, &a, at);
        let markup = match symbol_hover(
            &a,
            src,
            token,
            symbol_of(src, &a, at),
            "B",
            generic_binding_note(&a, token.1).as_deref(),
            generic_call_bindings(&a, token.1),
        )
        .expect("a hover")
        .contents
        {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(markup.contains("function([int]?) -> int"), "{markup}");
        assert!(
            !markup.contains("function(T?"),
            "the signature still names the parameter rather than the argument: {markup}"
        );
    }

    /// The markup a hover for `name` would show, answered the way the server
    /// answers it rather than through one lower layer
    fn hover_markup(text: &str, name: &str, at: &str) -> String {
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, at);
        match symbol_hover(
            &analysis,
            text,
            token_of(text, &analysis, at),
            symbol,
            name,
            None,
            None,
        )
        .expect("a hover")
        .contents
        {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        }
    }

    #[test]
    fn a_builtin_in_the_global_scope_still_carries_its_documentation() {
        // The global half of the standard library is registered as symbols, so a
        // hover on one is answered from the symbol -- and used to stop there,
        // which left every global builtin showing a signature and nothing else.
        for name in ["print", "assert", "clone", "expect", "try"] {
            let view = docs::value_builtin_view(name).unwrap_or_else(|| panic!("{name}"));
            assert!(!view.content.is_empty(), "{name} has no prose to show");
            let text = format!("{name}(1)\nreturn 1\n");
            let markup = hover_markup(&text, name, name);
            assert!(
                markup.contains(view.content.lines().next().unwrap_or_default()),
                "{name} lost its prose: {markup}"
            );
        }
    }

    #[test]
    fn a_function_the_project_writes_keeps_its_own_signature() {
        // The same name written in the source is not the builtin's declaration,
        // and the builtin's prose must not be shown in its place.
        let text = "function print(a: int): string\n    return \"x\"\nend\nprint(1)\n";
        let markup = hover_markup(text, "print", "print(a: int)");
        assert!(
            markup.contains("print: function(int?) -> string"),
            "{markup}"
        );
        assert!(
            !markup.contains("Prints to standard output"),
            "the builtin's prose was shown for a function the project wrote: {markup}"
        );
    }

    #[test]
    fn a_call_hover_answers_the_same_wherever_it_is_written() {
        let text = concat!(
            "@inline\n",
            "function B<T: [V], V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
            "    where type C = T\n",
            "    return 1, 2\n",
            "end\n",
            "B([12312], [])\n",
            "local _, c = B([12312], [])\n",
        );
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;

        // The same call, once as a statement of its own and once as the value of
        // an assignment. A reader asking "what is this" at either one is asking
        // the same question, so the two have to answer the same.
        let mut answers = vec![];
        // `B(` opens on line 5 at column 0 as a statement, and on line 6 at column
        // 13 as the value of an assignment.
        for (what, position) in [
            (
                "statement",
                Position {
                    line: 5,
                    character: 0,
                },
            ),
            (
                "in an expression",
                Position {
                    line: 6,
                    character: 13,
                },
            ),
        ] {
            let token = convert::token_at(text, position, &analysis.tokens.tokens)
                .unwrap_or_else(|| panic!("no token for the {what} call"));
            let span = token.1;
            let name = ident_name(&token.0).unwrap_or_default();

            // The same three steps the hover handler takes: the span map, the
            // recorded uses, then the table by position.
            let via_decl = table
                .symbol_at_span(span)
                .and_then(|s| s.ty_display.clone());
            let via_uses = analysis
                .scope
                .uses
                .get(&span)
                .and_then(|id| table.symbol_by_id(*id))
                .and_then(|s| s.ty_display.clone());
            let via_pos = table
                .resolve_at((span.start.line, span.start.column), &name)
                .and_then(|s| s.ty_display.clone());
            let chosen = via_decl
                .clone()
                .or_else(|| via_uses.clone())
                .or_else(|| via_pos.clone())
                .unwrap_or_else(|| panic!("{what}: resolved to nothing"));
            answers.push((what.to_owned(), chosen));
        }

        assert_eq!(
            answers[0].1, answers[1].1,
            "`{}` and `{}` are the same call and must hover the same",
            answers[0].0, answers[1].0
        );
    }

    #[test]
    fn a_generic_function_hovers_with_its_type_parameters() {
        let text = concat!(
            "@inline\n",
            "function B<T: [V], V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
            "    where type C = T\n",
            "    return 1, 2\n",
            "end\n",
            "return nil\n",
        );
        let analysis = analyze(text);
        let decl = token_of(text, &analysis, "B<T: [V]");
        let hover = symbol_hover(
            &analysis,
            text,
            decl,
            symbol_of(text, &analysis, "B<T: [V]"),
            "B",
            None,
            None,
        )
        .expect("a hover for the declaration");
        let rendered = hover.contents;
        let text = match rendered {
            HoverContents::Markup(markup) => markup.value,
            other => format!("{other:?}"),
        };

        // The list is what the reader would write to name this function, so
        // leaving it out made the signature read as a monomorphic function --
        // one shape, rather than a shape per type argument.
        assert!(
            text.contains("<T") && text.contains("V"),
            "the type parameters belong in the signature: {text}"
        );
        // `C` is not a type parameter of `B`, it is a `where` binding, so a `<>`
        // here would be claiming a generic that does not exist.
        assert!(!text.contains("<C"), "{text}");
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
            duka_lib::duka_frontend::lexer::token::TokenKind::Ident(_)
        ));
        assert_eq!(token.1.start.line, 2);

        let pos = Position {
            line: 3,
            character: 6,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("line4 token");
        assert!(matches!(
            token.0,
            duka_lib::duka_frontend::lexer::token::TokenKind::Ident(_)
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
    fn an_inferred_local_shows_what_it_started_with() {
        let text = r#"
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
local c: string? = "x"
if c ~= nil then
    local be = c
else
    local ve = c
end
if not_nil(c) then
    local bg = c
else
    local vg = c
end
"#;
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "be = c").ty_display.as_deref(),
            Some("string"),
            "the then of a nil check starts out non-nil"
        );
        assert_eq!(
            symbol_of(text, &analysis, "ve = c").ty_display.as_deref(),
            Some("nil"),
            "the else starts out nil"
        );
        assert_eq!(
            symbol_of(text, &analysis, "bg = c").ty_display.as_deref(),
            Some("string"),
            "the then of an ensure starts out non-nil"
        );
        assert_eq!(
            symbol_of(text, &analysis, "vg = c").ty_display.as_deref(),
            Some("nil"),
            "the else of an ensure starts out nil"
        );
    }

    #[test]
    fn a_boolean_check_leaves_the_other_literal() {
        let text = r#"
local c: bool? = false
if c ~= true and c ~= nil then
    local only = c
end
if c ~= false and c ~= nil then
    local yes = c
end
"#;
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "only = c").ty_display.as_deref(),
            Some("false"),
            "true and nil excluded leaves false"
        );
        assert_eq!(
            symbol_of(text, &analysis, "yes = c").ty_display.as_deref(),
            Some("true"),
            "false and nil excluded leaves true"
        );
    }

    #[test]
    fn a_builtin_member_solves_its_type_parameter() {
        let text = r#"
local a: array<int> = [1, 2, 3]
local x = array.pop(a)
"#;
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "x = array.pop(a)")
                .ty_display
                .as_deref(),
            Some("int"),
            "the element type of the array is what comes back"
        );
    }

    #[test]
    fn a_coroutine_and_a_userdata_have_types_of_their_own() {
        let text = r#"
local function worker()
    return 1
end
local co = spawn(worker)
local re = regex.compile("a+")
"#;
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "co = spawn(worker)")
                .ty_display
                .as_deref(),
            Some("coroutine"),
            "a spawned coroutine is a coroutine"
        );
        assert_eq!(
            symbol_of(text, &analysis, "re = regex.compile")
                .ty_display
                .as_deref(),
            Some("CompiledRegex"),
            "a userdata is the userdata it is"
        );
    }

    #[test]
    fn a_local_initialized_with_nil_is_any() {
        let text = "local a = nil\nlocal b\n";
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "a = nil").ty_display.as_deref(),
            Some("any"),
            "a literal nil says nothing about what the name may hold later"
        );
        assert_eq!(
            symbol_of(text, &analysis, "b").ty_display.as_deref(),
            Some("any"),
            "a local with no initializer starts the same way"
        );
    }

    #[test]
    fn an_inferred_local_shows_the_literal_it_was_narrowed_to() {
        let text = r#"
local c: "1" | "2" = "1"
if c == "1" then
    local ok = c
else
    local err = c
end
if c ~= "1" then
    local other = c
else
    local same = c
end
"#;
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "ok = c").ty_display.as_deref(),
            Some("\"1\""),
            "the then of an equality is the literal"
        );
        assert_eq!(
            symbol_of(text, &analysis, "err = c").ty_display.as_deref(),
            Some("\"2\""),
            "the else is the other member"
        );
        assert_eq!(
            symbol_of(text, &analysis, "other = c")
                .ty_display
                .as_deref(),
            Some("\"2\""),
            "the then of an inequality is the other member"
        );
        assert_eq!(
            symbol_of(text, &analysis, "same = c").ty_display.as_deref(),
            Some("\"1\""),
            "the else of an inequality is the literal"
        );
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
                duka_lib::duka_frontend::lexer::token::TokenKind::Ident(n) => n.as_str(),
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
            SymbolType::Function => format!("function f: {}", local.ty_display.as_deref().unwrap()),
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
        let ty = symbol.ty_display.as_deref().expect("signature");
        // Where the `?` falls is the whole rule: nilability belongs to a
        // declaration of a value, and a return annotation is not one.
        //
        // `a: T` is a parameter, so it starts out accepting `nil` and reads
        // `T?`. `array<T!>!` is a type argument written by the programmer, so it
        // reads exactly as written. `(int, T, ...)` is a type context -- it says
        // what comes back -- and neither the `int` nor the `T` is quietly widened
        // to accept a `nil` the signature never promised.
        assert_eq!(ty, "function(T?, array<T>, ...) -> (int, T, ...)");
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
    fn a_keywordish_declaration_reads_as_a_keyword() {
        let text = "@keywordish function f() end\nlocal x = f()\n";
        let analysis = analyze(text);
        let semantic = semantics(text, &analysis);
        // the declaration and the call both read as the keyword, wherever the
        // name is mentioned
        assert_eq!(
            types_of(&semantic, "f"),
            vec![convert::SEMANTIC_KEYWORD, convert::SEMANTIC_KEYWORD],
            "{semantic:?}"
        );
        let (items, _) = completion_items(&analysis, CompletionContext::Value, "f");
        let entry = items.iter().find(|i| i.label == "f").expect("f");
        assert_eq!(entry.kind, Some(CompletionItemKind::KEYWORD));
    }

    #[test]
    fn a_plain_declaration_does_not_read_as_a_keyword() {
        let text = "function g() end\nlocal x = g()\n";
        let analysis = analyze(text);
        let semantic = semantics(text, &analysis);
        assert_eq!(
            types_of(&semantic, "g"),
            vec![convert::SEMANTIC_FUNCTION, convert::SEMANTIC_FUNCTION],
            "{semantic:?}"
        );
    }

    #[test]
    fn a_declared_name_says_it_is_one() {
        // every form of declaration answers the same way: a hover says the name
        // is a declaration, and the completion offers it as one. An object is a
        // type-namespace name, so it is offered in the namespace it lives in.
        let text = "@declare function strlen(s: string): int\n\
                    @declare global HOSTNAME: string\n\
                    @declare object Point\n\
                    \x20   x: int\n\
                    end\n\
                    function built() return 1 end\n";
        let analysis = analyze(text);

        for (name, context) in [
            ("strlen", CompletionContext::Value),
            ("HOSTNAME", CompletionContext::Value),
            ("Point", CompletionContext::Type),
        ] {
            let token = token_of(text, &analysis, name);
            let symbol = symbol_of(text, &analysis, name);
            let hover = convert::to_hover(text, token, Some(symbol));
            let rendered = hover_text(&hover);
            assert!(
                rendered.contains("@declare"),
                "{name} should read as a declaration: {rendered}"
            );
            let (items, _) = completion_items(&analysis, context, name);
            let entry = items.iter().find(|i| i.label == name).expect(name);
            assert!(
                entry
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.starts_with("@declare")),
                "{name} should be offered as a declaration: {entry:?}"
            );
        }

        let plain = token_of(text, &analysis, "built");
        let plain_symbol = symbol_of(text, &analysis, "built");
        let hover = convert::to_hover(text, plain, Some(plain_symbol));
        assert!(
            !hover_text(&hover).contains("@declare"),
            "a function with a body is not a declaration"
        );
        let (items, _) = completion_items(&analysis, CompletionContext::Value, "built");
        let entry = items.iter().find(|i| i.label == "built").expect("built");
        assert!(
            entry
                .detail
                .as_deref()
                .is_none_or(|d| !d.starts_with("@declare")),
            "and is not offered as one: {entry:?}"
        );
    }

    fn hover_text(hover: &Hover) -> String {
        match &hover.contents {
            HoverContents::Markup(m) => m.value.clone(),
            other => format!("{other:?}"),
        }
    }

    /// An analysis of `text` as the `main.duka` of a directory holding a `host`
    /// module, so `require "host"` reaches something on disk the way it does in
    /// a real workspace.
    ///
    /// An analysis of `text` as the `main.duka` of a workspace whose `host`
    /// module is `module`.
    ///
    /// The module is handed over as an open document rather than written out, so
    /// a test needs no directory: the provider is asked for `host.duka` by path
    /// and answers from what it was given. Which is also what makes it a test of
    /// the thing that matters here -- that a module is reachable by name.
    fn analyze_with_module(text: &str, module: &str) -> compile::DocAnalysis {
        let dir = std::path::PathBuf::from("/workspace");
        let mut open = HashMap::new();
        open.insert(dir.join("modules").join("host.duka"), module.to_owned());
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        compile::analyze(
            text,
            "main.duka",
            Some(&dir.join("main.duka")),
            &open,
            &mut cache,
        )
    }
    #[test]
    fn a_declared_export_of_another_module_reads_as_one() {
        let module =
            "@declare export function strlen(s: string): int\nexport function real() return 1 end";
        for (member, declared) in [("strlen", true), ("real", false)] {
            let text = format!("local host = require(\"host\")\nlocal n = host.{member}\n");
            let analysis = analyze_with_module(&text, module);
            assert!(
                analysis.errors.is_empty(),
                "{member}: {:?}",
                analysis.errors
            );
            let token = token_of(&text, &analysis, member);
            // the container a member read resolves to, and then the module it
            // came from: the same two steps the handler takes, with the base
            // being the local the `require` was assigned to
            let container = path_type(&analysis, "host").expect("the module value");
            let (_, detail) = convert::record_members(&container)
                .into_iter()
                .find(|(name, _)| name == member)
                .unwrap_or_else(|| panic!("{member} is a member: {container}"));
            let hover = convert::member_hover(
                &text,
                token,
                "host",
                &detail,
                module_is_declared(&analysis, "host", member).unwrap_or(false),
            );
            assert!(
                hover_text(&hover).contains("@declare") == declared,
                "{member} should {}read as a declaration: {}",
                if declared { "" } else { "not " },
                hover_text(&hover)
            );
        }
    }

    #[test]
    fn a_member_of_something_that_is_not_a_module_says_nothing() {
        let analysis = analyze("local t = { x = 1 }\nlocal n = t.x\n");
        assert_eq!(module_is_declared(&analysis, "t", "x"), None);
        assert_eq!(
            module_is_declared(&analysis, "t.y", "x"),
            None,
            "only the first segment of a dotted path is a module name"
        );
    }

    #[test]
    fn a_name_in_a_type_position_hovers_with_what_it_resolved_to() {
        let module = concat!(
            "@declare export function strlen(s: string): int\n",
            "export type Alias = { x: int, y: string }\n",
            "export type Outer = { inner: { deep: bool } }\n",
        );
        let entry = concat!(
            "local a: RequireType(\"host\").Alias = { x = 1, y = \"s\" }\n",
            "local b: RequireType(\"host\").Outer.inner.deep = true\n",
            "local function f(s: string) return 1 end\n",
            "local c: RequireType(\"host\").strlen = f\n",
            "local d: int = 1\n",
        );
        let analysis = analyze_with_module(entry, module);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);

        // every member is a reference, so every member is describable -- and the
        // one that is only a bare `int` is not a reference and says nothing
        let on = |line: u32| -> Vec<String> {
            let mut v: Vec<String> = analysis
                .scope
                .type_at_span
                .iter()
                .filter(|(span, _)| span.start.line == line)
                .map(|(_, ty)| ty.to_string())
                .map(|ty| ty.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            on(1),
            vec!["{ x: int, y: string }".to_owned()],
            "{:?}",
            analysis.scope.type_at_span
        );
        assert_eq!(
            on(2),
            vec![
                "bool".to_owned(),
                "{ deep: bool }".to_owned(),
                "{ inner: { deep: bool } }".to_owned(),
            ],
            "each dot is a reference of its own: {:?}",
            analysis.scope.type_at_span
        );
        assert_eq!(
            on(4),
            vec!["function(string?) -> int".to_owned()],
            "{:?}",
            analysis.scope.type_at_span
        );
        assert!(on(5).is_empty(), "a bare type is not a reference");
    }

    #[test]
    fn a_type_position_completes_from_what_the_module_answers_for() {
        let module = concat!(
            "export type Alias = { x: int }\n",
            "export type function Pick(a, b)\n    return a\nend\n",
            "export object Point\n    y = 1\nend\n",
            "@declare export function strlen(s: string): int\n",
            "export function mine() return 1 end\n",
        );
        // the module is only loaded because something in the file asks for it,
        // and the second line is what the completion is for: it parses to nothing
        // and says nothing, which is the point
        let entry = concat!(
            "local a: RequireType(\"host\").Alias = { x = 1 }\n",
            "local b: RequireType(\"host\").\n",
        );
        let analysis = analyze_with_module(entry, module);
        let items = require_type_members(&analysis, "host", &[]);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // what the module exports as a type, plus what it only declared
        assert_eq!(
            labels,
            vec!["Alias", "Pick", "Point", "strlen"],
            "{items:#?}"
        );
        assert!(
            items
                .iter()
                .all(|i| { i.detail.as_deref() == Some(format!("host.{}", i.label).as_str()) }),
            "each says which module it lives in: {items:#?}"
        );
    }

    #[test]
    fn a_type_position_of_a_module_that_is_not_there_completes_to_nothing() {
        let analysis = analyze_with_module("local a: RequireType(\"nope\").\n", "");
        assert!(require_type_members(&analysis, "nope", &[]).is_empty());
        assert!(require_type_members(&analysis, "host", &[]).is_empty());
    }

    #[test]
    fn a_string_inside_require_type_names_a_module() {
        use duka_lib::duka_shared::types::{DukaLexer, SourceName};
        let tokens = |src: &str| {
            duka_lib::duka_frontend::lexer::LexerWithMacro::new(
                std::io::Cursor::new(src),
                SourceName::Unnamed,
                Default::default(),
            )
            .tokenize()
            .unwrap()
            .tokens
        };
        let string_at = |ts: &[duka_lib::duka_frontend::lexer::token::Token], name: &str| {
            ts.iter().position(
                |(k, _)| matches!(k, TokenKind::String(s) if s.as_ref() == name.as_bytes()),
            )
        };
        // both spellings of "this string is a module"
        let sugar = tokens("local m = require \"a\"\n");
        assert!(names_a_module(&sugar, string_at(&sugar, "a").unwrap()));
        let called = tokens("local m = require(\"b\")\n");
        assert!(names_a_module(&called, string_at(&called, "b").unwrap()));
        let typed = tokens("local a: RequireType(\"c\").Alias = nil\n");
        assert!(names_a_module(&typed, string_at(&typed, "c").unwrap()));
        // a string that is not a module name is not one
        let plain = tokens("local s = \"hello\"\n");
        assert!(!names_a_module(&plain, string_at(&plain, "hello").unwrap()));
        let other = tokens("local s = other(\"hello\")\n");
        assert!(!names_a_module(&other, string_at(&other, "hello").unwrap()));
    }

    #[test]
    fn a_class_from_another_module_is_reachable_in_the_editor() {
        let module = concat!(
            "export object Point\n",
            "    x: int\n",
            "    function :len() return self.x end\n",
            "end\n",
            "export object Box\n",
            "    w: int\n",
            "end\n",
        );
        let entry = concat!(
            "local m = require(\"host\")\n",
            "local p = m.Point.new(1)\n",
            "local n: int = p.x\n",
            "return p:len()\n",
        );
        let analysis = analyze_with_module(entry, module);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);

        // the class is resolved through the module that declared it, so the
        // members of `p` are read out of that module's table
        let resolved = path_resolved(&analysis, "p").expect("p is a class");
        let (class, objects) = resolved.class_table(&analysis).expect("p is a class");
        assert_eq!(&*class.name, "Point");
        assert_eq!(
            member_of_class(objects, class, Some("x")).map(|(_, ty)| ty),
            Some("int?".to_owned()),
            "a field of a class from another module keeps its type"
        );
        assert!(
            member_of_class(objects, class, Some("len"))
                .is_some_and(|(_, ty)| ty.starts_with("function")),
            "and so does a method"
        );

        // two files may both declare a `Point`; the answer is the one this
        // receiver was built from, not the first one in this file
        assert_eq!(class_members(objects, class).len(), 2);

        // a class is offered as a member of the module, and as a class
        let (items, _) = completion_items(&analysis, CompletionContext::Value, "Point");
        let _ = items;
    }

    #[test]
    fn a_class_is_told_apart_from_a_class_of_the_same_name_elsewhere() {
        let analysis = analyze_with_module(
            "local m = require(\"host\")\nlocal p = m.Point.new(1)\nreturn p\n",
            "export object Point\n    x: int\nend\nexport object Box\n    w: int\nend\n",
        );
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        let (class, objects) = path_resolved(&analysis, "p")
            .and_then(|r| r.class_table(&analysis))
            .expect("p is a class");
        assert_eq!(&*class.name, "Point");
        // `Box` is a different class and is not a member of `Point`
        assert!(member_of_class(objects, class, Some("w")).is_none());
        assert!(
            class_table(
                &analysis,
                &Type::Object {
                    id: 0,
                    name: "Point".into(),
                    base: None,
                    args: [].into(),
                    module: Some("not-a-module".into()),
                    mt: None,
                }
            )
            .is_none(),
            "a class whose module is not loaded is not a class here"
        );
    }

    #[test]
    fn a_label_and_a_goto_describe_each_other() {
        let text = "for i = 1, 3 do\n\
                    \x20   if i == 2 then goto ::done:: end\n\
                    end\n\
                    ::done::\n\
                    return 1\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        // the goto is written before the label, so it can only be followed
        // once the whole function has been walked
        let goto_span = span_under(&analysis, text, "done");
        assert_eq!(table.goto_label_at(goto_span), Some("done"));
        let decl = table
            .goto_target(goto_span)
            .expect("a forward goto resolves");
        assert_eq!(table.label_at(decl), Some("done"));
        assert_eq!(table.label_uses(decl), vec![goto_span]);
        assert_eq!(table.goto_targets(), vec![(goto_span, decl)]);

        let on_label = hover_value(
            &label_hover(
                &analysis,
                text,
                decl,
                token_under(&analysis, text, "::done::"),
            )
            .expect("a label is described"),
        );
        assert!(on_label.contains("(label) done"), "{on_label}");
        assert!(on_label.contains("Reached by 1"), "{on_label}");

        let on_goto = hover_value(
            &label_hover(
                &analysis,
                text,
                goto_span,
                token_under(&analysis, text, "goto"),
            )
            .expect("a goto is described"),
        );
        assert!(on_goto.contains("(goto) done"), "{on_goto}");
        assert!(on_goto.contains("Jumps to line 4"), "{on_goto}");
    }

    #[test]
    fn a_goto_with_no_label_in_reach_is_still_described() {
        let text = "goto ::nowhere::\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let goto_span = span_under(&analysis, text, "nowhere");
        assert_eq!(table.goto_label_at(goto_span), Some("nowhere"));
        assert_eq!(table.goto_target(goto_span), None);
        let value = hover_value(
            &label_hover(
                &analysis,
                text,
                goto_span,
                token_under(&analysis, text, "nowhere"),
            )
            .expect("a goto is described even with nothing to land on"),
        );
        assert!(
            value.contains("No label of that name is in reach"),
            "{value}"
        );
    }

    #[test]
    fn a_label_of_one_function_is_not_reached_from_another() {
        let text = "function a() ::spot:: return 1 end\n\
                    function b() goto ::away:: end\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let goto_span = span_under(&analysis, text, "away");
        assert!(
            analysis
                .errors
                .iter()
                .any(|e| format!("{:?}", e.kind()).contains("InvisibleGotoLabel")),
            "the checker refuses the jump as well: {:?}",
            analysis.errors
        );
        assert_eq!(table.goto_label_at(goto_span), Some("away"));
        assert_eq!(
            table.goto_target(goto_span),
            None,
            "a label does not cross functions"
        );
    }

    /// The token a needle sits on, the way the hover handler picks it.
    fn token_under<'a>(
        analysis: &'a compile::DocAnalysis,
        text: &str,
        needle: &str,
    ) -> &'a duka_lib::duka_frontend::lexer::token::Token {
        let (line, character) = offset_of(text, needle);
        convert::token_at(text, Position { line, character }, &analysis.tokens.tokens)
            .unwrap_or_else(|| panic!("no token at {needle}"))
    }

    /// The span of the token a needle sits on.
    fn span_under(analysis: &compile::DocAnalysis, text: &str, needle: &str) -> Span {
        token_under(analysis, text, needle).1
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

    /// There are two kinds of type builtin and a reader needs both.
    ///
    /// `Error` is written in Rust and carries documentation; `Pick` is written in
    /// the type prelude and does not. Listing only the first kind meant half the
    /// names callable in a type position could not be offered, could not be
    /// hovered, and did not exist as far as the editor was concerned -- while
    /// working perfectly well in the compiler.
    #[test]
    fn both_kinds_of_type_builtin_are_listed() {
        let names: Vec<String> = docs::type_builtin_views()
            .iter()
            .map(|v| v.name.clone().into_owned())
            .collect();
        // the Rust half
        assert!(names.iter().any(|n| n == "Error"), "{names:?}");
        assert!(names.iter().any(|n| n == "Assert"), "{names:?}");
        // the DUKA half, read from `builtin.duka`
        for expected in [
            "Optional",
            "NonNil",
            "Pick",
            "Omit",
            "Partial",
            "Required",
            "Record",
            "BuildObject",
            "TypeList",
            "TypeTree",
            "SameAs",
            "Integral",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "{expected} missing: {names:?}"
            );
        }
    }

    /// The arity is what the completion item shows, and it has to come from the
    /// declaration rather than be guessed -- `Pick` takes two and `Omit` takes two,
    /// and neither is inferable from the name.
    #[test]
    fn a_duka_type_builtin_carries_its_arity() {
        let find = |name: &str| {
            docs::type_builtin_views()
                .into_iter()
                .find(|v| &*v.name == name)
        };
        for (name, arity) in [("Pick", 2), ("Omit", 2), ("SameAs", 2), ("Integral", 1)] {
            let view = find(name).unwrap_or_else(|| panic!("{name} missing"));
            let params = vec!["any"; arity].join(", ");
            assert_eq!(
                view.signature.as_deref(),
                Some(format!("type function {name}({params}): any").as_str()),
                "{name} arity"
            );
        }
    }

    /// Hover on a type builtin has to find it in either list. This one is the DUKA
    /// kind, which had no metadata at all.
    #[test]
    fn a_duka_type_builtin_has_a_hover() {
        let text = "type P = Pick([int, string], [0])\nreturn nil\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let pick = table
            .resolve_at((2, 0), "Pick")
            .unwrap_or_else(|| panic!("the prelude declares Pick in the type namespace"));
        assert!(
            matches!(pick.symbol_type, SymbolType::TypeFunction(_)),
            "{:?}",
            pick.symbol_type
        );
        let token = token_of(text, &analysis, "Pick");
        let hover = symbol_hover(&analysis, text, &token, pick, "Pick", None, None)
            .expect("a hover for a type builtin");
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("markup")
        };
        assert!(
            markup.value.contains("Pick"),
            "the hover has to name it: {}",
            markup.value
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
        let module = table.lookup_anywhere("os").expect("os is declared");
        assert!(
            module.ty.is_some(),
            "a module has to carry its members to be readable"
        );
        assert!(matches!(
            table.lookup_anywhere("print").map(|s| &s.symbol_type),
            Some(SymbolType::Function)
        ));

        let a = symbol_of(text, &analysis, "a = os");
        assert_eq!(
            a.ty_display.as_deref(),
            Some("float?"),
            "os.clock returns a float"
        );
        let b = symbol_of(text, &analysis, "b = string");
        assert_eq!(
            b.ty_display.as_deref(),
            Some("string"),
            "an inferred local starts out with the type of what was put in it"
        );
    }

    #[test]
    fn a_name_the_file_declares_itself_wins_over_a_builtin() {
        let text = "local print = 1\nlocal a = print\n";
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, "print = 1");
        assert!(matches!(symbol.symbol_type, SymbolType::Variable));
        assert_eq!(
            analysis
                .scope
                .symbols
                .lookup_anywhere("print")
                .map(|s| s.id),
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
    ) -> &'a duka_lib::duka_frontend::lexer::token::Token {
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
    ) -> &'a duka_lib::duka_shared::utils::Symbol {
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
                None,
            )
            .expect("param hover"),
        );
        assert!(value.contains("(param) x"), "{value}");
        assert!(value.contains("int"), "{value}");
    }

    #[test]
    fn type_parameter_carries_its_bound_and_default() {
        // A bound says what a type has to be, so it is read as a type and not as a
        // place holding a value: `T: num` means T must be a number, not a
        // number-or-nil. The `float?` this used to expect was the annotation
        // default reaching into a position that never asked for it.
        let text = "function f<T: num, U = int>(a: T)\n    return a\nend\n";
        let analysis = analyze(text);
        let table = &analysis.scope.symbols;
        let t = table.resolve_at((2, 13), "T").expect("T declared");
        assert_eq!(
            t.symbol_type,
            SymbolType::TypeParam {
                bound: Some("float".into()),
                default: None
            }
        );
        let u = table.resolve_at((2, 13), "U").expect("U declared");
        // `U = int` is the other half of the same rule: a default says what the
        // type is, so it is read as `int`.
        assert_eq!(
            u.symbol_type,
            SymbolType::TypeParam {
                bound: None,
                default: Some("int".into())
            }
        );
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
        // `num` is a bound, so it reads as the type it means rather than as a
        // value that may be nil.
        assert_eq!(
            resolved.symbol_type,
            SymbolType::TypeParam {
                bound: Some("float".into()),
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
                None,
            )
            .expect("type parameter hover"),
        );
        assert!(value.contains("(type parameter) T"), "{value}");
        // `num` is a bound and so is read as the type it means.
        assert!(value.contains("bound: `float`"), "{value}");
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
    /// A module is a record of what is in it, so it is too large to write out --
    /// and its own name is all that would be left. It reads through the builtin
    /// documentation instead, the same way a function does.
    fn a_module_hover_describes_the_module_and_not_a_record_of_it() {
        let src = "local x = os.exit\n";
        let a = analyze(src);
        let hover = symbol_hover(
            &a,
            src,
            token_of(src, &a, "os.exit"),
            symbol_of(src, &a, "os.exit"),
            "os",
            None,
            None,
        )
        .expect("a hover");
        let shown = hover_value(&hover);
        assert!(shown.contains("module os"), "{shown}");
        assert!(
            !shown.contains("clock:") && !shown.contains("execute:"),
            "and not a record of its members: {shown}"
        );
        assert!(
            shown.contains("`@declare`"),
            "a module the runtime provides is a declaration like any other: {shown}"
        );
    }

    /// A global builtin is a member of core, and `os.exit` is a member of `os`. The
    /// bare `exit` is declared nowhere -- it is only reached through its module --
    /// so looking it up on its own finds nothing and leaves the reader stuck.
    #[test]
    fn a_builtin_member_is_found_by_the_path_it_is_written_with() {
        for (path, file) in [
            ("os.exit", "os.duka"),
            ("string.upper", "string.duka"),
            ("print", "core.duka"),
        ] {
            let (uri, _) = crate::declared::locate(path).unwrap_or_else(|| panic!("{path}"));
            assert_eq!(uri.as_str(), format!("{SCHEME}:/{file}"));
        }
    }

    /// The generated file declares names the analyzer already put in scope --
    /// `require` and `typeof` are two -- so analysing one reports a name being
    /// preserved. A reader opened it by jumping to a builtin and can do nothing
    /// about that, which is why a generated file has no diagnostics of its own.
    #[test]
    fn a_generated_file_reports_what_it_cannot_act_on() {
        let mut errors = vec![];
        for file in crate::declared::generated() {
            let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
            let analysis =
                compile::analyze(&file.text, &file.name, None, &HashMap::new(), &mut cache);
            errors.extend(analysis.errors.iter().map(|e| e.to_string()));
        }
        assert!(
            errors.iter().all(|e| e.contains("is preserved")),
            "only the names the analyzer already has: {errors:?}"
        );
        assert!(
            errors_of_generated_are_not_published(),
            "and none of them reaches the reader"
        );
    }

    fn errors_of_generated_are_not_published() -> bool {
        // `diagnostics_of` is what decides, and it decides on the scheme
        uri_of_generated().starts_with(&format!("{}:", SCHEME))
    }

    fn uri_of_generated() -> String {
        format!("{}:/os.duka", SCHEME)
    }

    #[test]
    fn a_document_may_stand_where_a_neighbour_is_looked_for() {
        // The token before `@name` and the token after it are both read by
        // position. A `---` between them is a document about the thing being
        // declared, and it must not read as "the sigil introduces nothing".
        let src = concat!(
            "@---\n",
            "@declare function strlen(s: string): int\n",
            "@declare global FLAG: bool\n",
            "return FLAG\n",
        );
        let a = analyze(src);
        assert!(
            !a.errors.is_empty(),
            "a `@` followed by a document is not an attribute: {:?}",
            a.errors
        );
    }

    #[test]
    fn a_document_reaches_the_hover_of_what_it_documents() {
        let src = concat!(
            "--- Returns the number of items.\n",
            "--- It walks the array once.\n",
            "function count(items): int\n",
            "    return 1\n",
            "end\n",
            "return count([])\n",
        );
        let a = analyze(src);
        // the same answer at the declaration and at the use of it
        for at in ["count(items)", "count([]"] {
            let hover = symbol_hover(
                &a,
                src,
                token_of(src, &a, at),
                symbol_of(src, &a, at),
                "count",
                None,
                None,
            )
            .expect("a hover");
            let shown = hover_value(&hover);
            assert!(
                shown.contains("Returns the number of items."),
                "{at}: {shown}"
            );
            assert!(shown.contains("It walks the array once."), "{at}: {shown}");
        }
    }

    #[test]
    fn a_blank_line_ends_a_document() {
        // There is no token for a blank line, so the lines the run covers are what
        // says it ended -- and a run further away than one line is a remark that
        // happened to be near, not documentation.
        let src = concat!(
            "--- This is about the function below.\n",
            "\n",
            "function f(): int\n",
            "    return 1\n",
            "end\n",
            "return f()\n",
        );
        let a = analyze(src);
        let hover = symbol_hover(
            &a,
            src,
            token_of(src, &a, "f()"),
            symbol_of(src, &a, "f()"),
            "f",
            None,
            None,
        )
        .expect("a hover");
        assert!(
            !hover_value(&hover).contains("about the function"),
            "a remark above a blank line documents nothing: {}",
            hover_value(&hover)
        );
    }

    #[test]
    fn an_ordinary_comment_documents_nothing() {
        let src = concat!(
            "-- a remark, not a document\n",
            "function f(): int\n",
            "    return 1\n",
            "end\n",
            "return f()\n",
        );
        let a = analyze(src);
        let hover = symbol_hover(
            &a,
            src,
            token_of(src, &a, "f()"),
            symbol_of(src, &a, "f()"),
            "f",
            None,
            None,
        )
        .expect("a hover");
        assert!(
            !hover_value(&hover).contains("a remark"),
            "{}",
            hover_value(&hover)
        );
    }

    #[test]
    fn a_declaration_does_not_have_its_parameter_names_repeated_at_a_call() {
        // The signature already names them, so a hint beside a call inside one
        // says what the line above the reader already says.
        assert!(in_a_declaration(
            "@declare function exit(code: int | nil)\n",
            Span::default()
        ));
        assert!(!in_a_declaration("local n = exit(0)\n", Span::default()));
    }
    #[test]
    fn a_builtin_lands_on_a_declaration_of_its_own() {
        // A builtin is written in Rust, so there is no span for it in this file.
        // What it does have is a generated file of declarations, and landing on the
        // line that declares it reads better than not moving at all.
        let (uri, line) = crate::declared::locate("os.exit").expect("os.exit is somewhere");
        let file = crate::declared::generated()
            .into_iter()
            .find(|f| f.name == "os.duka")
            .expect("os.duka");
        assert_eq!(
            file.text.lines().nth(line as usize).unwrap().trim(),
            "@declare function exit(code: int | nil)"
        );
        assert_eq!(uri.scheme(), crate::declared::SCHEME);
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
        assert_eq!(table.lookup_anywhere("age").map(|s| s.id), Some(local.id));
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
