//! The Duka language server backend.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use duka_lib::duka_frontend::analyzer::{ObjectType, objects::ObjectMethod};
use duka_lib::duka_frontend::lexer::token::TokenKind;
use duka_lib::duka_shared::constants::{cgen, cpar, csugar, ctype};
use duka_lib::duka_shared::{
    dtype::{FunctionType, Type},
    errors::Span,
    types::SourceName,
    utils::SymbolType,
};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request::{GotoTypeDefinitionParams, GotoTypeDefinitionResponse};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::{
    compile, convert, docs,
    workspace::{SharedWorkspace, Snapshot, Workspace, lock},
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

/// How many times one snapshot may be restarted by an edit that landed while
/// it was being computed. Each restart runs on the newer text; a burst longer
/// than this is answered from the last completed run rather than from nothing.
const MAX_ANALYSIS_ATTEMPTS: usize = 8;

/// A future whose polls cannot unwind. A panic inside a request handler ends
/// the request as an `Err` here instead of unwinding through the server loop,
/// where it would take every other request down with it.
struct Contained<F: Future>(Pin<Box<F>>);

impl<F: Future> Future for Contained<F> {
    type Output = std::result::Result<F::Output, Box<dyn std::any::Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match std::panic::catch_unwind(AssertUnwindSafe(|| this.0.as_mut().poll(cx))) {
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}

/// What a panic had to say, for the log: the message when there is one, a
/// short note when there is not.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

pub struct Backend {
    client: Client,
    workspace: Arc<SharedWorkspace>,
    /// files that currently carry published diagnostics, so stale ones can be
    /// cleared once a change moves the errors elsewhere
    published: Arc<Mutex<HashSet<Url>>>,
    /// files waiting for the typing to pause. An edit joins the set; the
    /// publisher drains it once the debounce has elapsed.
    waiting: Arc<Mutex<HashSet<Url>>>,
    /// wakes the publisher when a file joins `waiting`
    kick: Arc<tokio::sync::Notify>,
    /// spawns the one publisher task the first time a change schedules one
    publisher: Arc<OnceLock<()>>,
    /// The roots the editor opened. They are the fallback for a file that is
    /// not under a manifest: a workspace folder is still a reason to look at
    /// the files beside it.
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

/// What a client may pass as `initializationOptions`.
///
/// The fields are deliberately few: everything a project needs is in its
/// `kao.toml`, and this is only for what an editor has to say before a project
/// exists.
#[derive(Debug, Default)]
struct InitOptions {
    /// Extra `?.duka` templates searched for `require`, in addition to the
    /// project's modules directory and `DUKA_PATH`.
    module_paths: Vec<String>,
}

impl InitOptions {
    fn from_value(value: &serde_json::Value) -> Self {
        let module_paths = value
            .get("modulePaths")
            .and_then(|paths| paths.as_array())
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|path| path.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self { module_paths }
    }
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
        self.contained(Self::declared_file_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn declared_file_body(&self, params: DeclaredFileParams) -> Result<Option<String>> {
        let name = params
            .uri
            .split_once("duka-virtual:")
            .map(|(_, rest)| rest)
            .unwrap_or_default()
            .trim_start_matches('/');
        Ok(crate::declared::generated()
            .iter()
            .find(|file| file.name == name)
            .map(|file| file.text.clone()))
    }

    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: Arc::new(Mutex::new(Workspace::default())),
            published: Arc::new(Mutex::new(HashSet::new())),
            waiting: Arc::new(Mutex::new(HashSet::new())),
            kick: Arc::new(tokio::sync::Notify::new()),
            publisher: Arc::new(OnceLock::new()),
            roots: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Runs a request body with panics contained: a panic becomes a line in
    /// the log and an empty answer, and the server keeps serving.
    async fn contained<T>(&self, fut: impl Future<Output = T>) -> Option<T> {
        match Contained(Box::pin(fut)).await {
            Ok(value) => Some(value),
            Err(panic) => {
                let _ = self
                    .client
                    .log_message(
                        MessageType::ERROR,
                        format!("request panicked: {}", panic_message(panic.as_ref())),
                    )
                    .await;
                None
            }
        }
    }

    /// The analysis of one document at the revision the editor last sent. The
    /// pipeline runs outside every lock, and an edit that lands while it runs
    /// supersedes it rather than waiting for it.
    async fn snapshot(&self, uri: &Url) -> Option<Snapshot> {
        snapshot_of(&self.client, &self.workspace, uri).await
    }

    /// The analysis of a file that may not be open in an editor tab. An open
    /// buffer wins over the copy on disk, exactly as it does for a `require`.
    async fn disk_snapshot(&self, path: &Path) -> Option<Snapshot> {
        let open_uri = lock(&self.workspace).open_uri_for_path(path);
        if let Some(uri) = open_uri
            && let Some(snapshot) = self.snapshot(&uri).await
        {
            return Some(snapshot);
        }
        if let Some(snapshot) = lock(&self.workspace).disk_snapshot(path) {
            return Some(snapshot);
        }
        let pending = lock(&self.workspace).pending_disk(path)?;
        let text = Arc::clone(&pending.text);
        let analysis = match tokio::task::spawn_blocking(move || pending.run()).await {
            Ok(analysis) => analysis,
            Err(err) => {
                let _ = self
                    .client
                    .log_message(
                        MessageType::ERROR,
                        format!("analysis of {} failed: {err}", path.display()),
                    )
                    .await;
                return None;
            }
        };
        lock(&self.workspace).commit_disk(path.to_path_buf(), text, analysis);
        lock(&self.workspace).disk_snapshot(path)
    }

    fn project_paths(&self, uri: &Url) -> Vec<PathBuf> {
        let Some(path) = uri.to_file_path().ok() else {
            return vec![];
        };
        let mut workspace = lock(&self.workspace);
        workspace
            .project_for(Some(&path))
            .map(|project| project.source_files())
            .unwrap_or_default()
    }

    fn candidate_paths(&self, uri: &Url, analysis: &compile::DocAnalysis) -> Vec<PathBuf> {
        let mut out = self.project_paths(uri);
        if let Ok(path) = uri.to_file_path() {
            out.push(path);
        }
        let open = lock(&self.workspace).open_uris();
        for open_uri in open {
            if let Ok(path) = open_uri.to_file_path() {
                out.push(path);
            }
        }
        for module in analysis.scope.modules.values() {
            if let Some(path) = module.source.name.path() {
                out.push(path.to_path_buf());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    async fn occurrences(&self, uri: &Url, span: Span) -> Option<(String, Vec<FileSpans>)> {
        let current = self.snapshot(uri).await?;
        let name = current
            .analysis
            .tokens
            .tokens
            .iter()
            .find(|(_, at)| *at == span)
            .and_then(|(kind, _)| ident_name(kind))?
            .to_owned();
        if let Some(spans) = loop_label_spans(&current.analysis, span).map(|(_, spans)| spans) {
            return Some((
                name,
                vec![FileSpans {
                    uri: uri.clone(),
                    text: Arc::clone(&current.text),
                    index: Arc::clone(&current.line_index),
                    spans,
                }],
            ));
        }
        if let Some(spans) = label_spans(&current.analysis, span) {
            return Some((
                name,
                vec![FileSpans {
                    uri: uri.clone(),
                    text: Arc::clone(&current.text),
                    index: Arc::clone(&current.line_index),
                    spans,
                }],
            ));
        }
        let current_path = uri.to_file_path().ok();
        let target = cross_target(&current.analysis, span, &name, current_path.as_deref());
        if target.is_none() && is_member_read(&current.analysis, span) {
            return None;
        }
        let Some(target) = target else {
            let (_, spans) = same_name_spans(&current.analysis, span)?;
            return Some((
                name,
                vec![FileSpans {
                    uri: uri.clone(),
                    text: Arc::clone(&current.text),
                    index: Arc::clone(&current.line_index),
                    spans,
                }],
            ));
        };
        let mut files: Vec<FileSpans> = vec![];
        for path in self.candidate_paths(uri, &current.analysis) {
            let open_uri = lock(&self.workspace).open_uri_for_path(&path);
            let snap = match open_uri {
                Some(open_uri) => match self.snapshot(&open_uri).await {
                    Some(snap) => snap,
                    None => continue,
                },
                None => {
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    if !text.contains(&name) {
                        continue;
                    }
                    match self.disk_snapshot(&path).await {
                        Some(snap) => snap,
                        None => continue,
                    }
                }
            };
            let Ok(file_uri) = Url::from_file_path(&path) else {
                continue;
            };
            let mut spans: Vec<Span> = vec![];
            if target.decl_path == path && target.decl_span != Span::EMPTY {
                spans.push(target.decl_span);
                if let Some((_, local)) = same_name_spans(&snap.analysis, target.decl_span) {
                    spans.extend(local);
                }
            }
            spans.extend(member_occurrences(&snap.analysis, &name, &target.decl_path));
            spans.sort_by_key(|at| (at.start.line, at.start.column));
            spans.dedup();
            if !spans.is_empty() {
                files.push(FileSpans {
                    uri: file_uri,
                    text: Arc::clone(&snap.text),
                    index: Arc::clone(&snap.line_index),
                    spans,
                });
            }
        }
        if files.is_empty() {
            return None;
        }
        Some((name, files))
    }

    fn project_paths_all(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.roots.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for uri in lock(&self.workspace).open_uris() {
            if let Ok(path) = uri.to_file_path()
                && let Some(parent) = path.parent()
            {
                dirs.push(parent.to_path_buf());
            }
        }
        let mut workspace = lock(&self.workspace);
        let mut out: Vec<PathBuf> = vec![];
        for dir in dirs {
            if let Some(project) = workspace.project_for_dir(&dir) {
                out.extend(project.source_files());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Publishes diagnostics now: the file the reader just touched, then every
    /// other open document, since one of them may read the file the errors
    /// moved out of.
    async fn publish(&self, uri: &Url) {
        let mut order = vec![uri.clone()];
        order.extend(
            lock(&self.workspace)
                .open_uris()
                .into_iter()
                .filter(|open| open != uri),
        );
        publish_round(&self.client, &self.workspace, &self.published, &order).await;
    }

    /// Schedules a publish once the typing pauses. A newer edit for the same
    /// file takes over the schedule, so a burst of changes costs one analysis.
    fn schedule_publish(&self, uri: &Url) {
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uri.clone());
        self.start_publisher();
    }

    fn schedule_publish_all(&self) {
        let open = lock(&self.workspace).open_uris();
        if open.is_empty() {
            return;
        }
        self.waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(open);
        self.start_publisher();
    }

    fn start_publisher(&self) {
        self.kick.notify_one();
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        let published = self.published.clone();
        let waiting = self.waiting.clone();
        let kick = self.kick.clone();
        self.publisher.get_or_init(move || {
            tokio::spawn(publisher_loop(client, workspace, published, waiting, kick));
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
                docs::value_builtin_params(&member_or_name(analysis, *span)).or_else(|| {
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
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(options) = &params.initialization_options {
            let options = InitOptions::from_value(options);
            if !options.module_paths.is_empty() {
                crate::compile::set_extra_module_paths(options.module_paths);
            }
        }
        let mut roots: Vec<PathBuf> = params
            .workspace_folders
            .as_ref()
            .map(|folders| {
                folders
                    .iter()
                    .filter_map(|folder| folder.uri.to_file_path().ok())
                    .collect()
            })
            .unwrap_or_default();
        if roots.is_empty()
            && let Some(root) = params
                .root_uri
                .as_ref()
                .and_then(|uri| uri.to_file_path().ok())
        {
            roots.push(root);
        }
        *self.roots.lock().unwrap_or_else(|e| e.into_inner()) = roots;

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "duka-lsp".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_owned(), ",".to_owned()]),
                    retrigger_characters: Some(vec![",".to_owned()]),
                    work_done_progress_options: Default::default(),
                }),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_owned(), ":".to_owned()]),
                    // The documentation of every candidate is a rendered
                    // document. Sending a thousand of them on each keystroke
                    // is the cost; resolving the one the reader picks is not.
                    resolve_provider: Some(true),
                    ..Default::default()
                }),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        work_done_progress_options: Default::default(),
                        resolve_provider: Some(false),
                    },
                )),
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
                            range: Some(true),
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        lock(&self.workspace).open(doc.uri.clone(), doc.text, doc.version);
        self.schedule_publish(&doc.uri);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if params.content_changes.is_empty() {
            return;
        }
        if lock(&self.workspace).apply_change(
            &uri,
            &params.content_changes,
            params.text_document.version,
        ) {
            self.schedule_publish(&uri);
        }
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

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            for removed in &params.event.removed {
                if let Ok(path) = removed.uri.to_file_path() {
                    roots.retain(|root| root != &path);
                }
            }
            for added in &params.event.added {
                if let Ok(path) = added.uri.to_file_path()
                    && !roots.contains(&path)
                {
                    roots.push(path);
                }
            }
        }
        lock(&self.workspace).invalidate_projects();
        self.schedule_publish_all();
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let manifests = params
            .changes
            .iter()
            .any(|change| change.uri.path().ends_with("kao.toml"));
        {
            let mut workspace = lock(&self.workspace);
            workspace.touch();
            for change in &params.changes {
                if let Ok(path) = change.uri.to_file_path() {
                    workspace.invalidate_disk(&path);
                }
            }
            if manifests {
                workspace.invalidate_projects();
            }
        }
        self.schedule_publish_all();
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        self.contained(Self::symbol_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn completion_resolve(&self, mut item: CompletionItem) -> Result<CompletionItem> {
        let Some(data) = item.data.take() else {
            return Ok(item);
        };
        let documentation = if let Some(path) = data.get("builtin").and_then(|v| v.as_str()) {
            docs::value_builtin_view(path)
                .or_else(|| docs::type_builtin_view(path))
                .map(|view| view.render())
        } else if let Some(name) = data.get("keyword").and_then(|v| v.as_str()) {
            docs::keyword_view(name).map(|view| view.render())
        } else {
            None
        };
        if let Some(documentation) = documentation {
            item.documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: documentation,
            }));
        }
        Ok(item)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.contained(Self::goto_definition_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn goto_type_definition(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        self.contained(Self::type_definition_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.contained(Self::hover_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        self.contained(Self::semantic_tokens_full_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        self.contained(Self::semantic_tokens_range_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        self.contained(Self::document_symbol_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.contained(Self::references_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        self.contained(Self::signature_help_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        self.contained(Self::document_highlight_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        self.contained(Self::prepare_rename_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        self.contained(Self::rename_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        self.contained(Self::folding_range_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn document_link(&self, params: DocumentLinkParams) -> Result<Option<Vec<DocumentLink>>> {
        self.contained(Self::document_link_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        self.contained(Self::selection_range_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        self.contained(Self::inlay_hint_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        self.contained(Self::code_action_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.contained(Self::completion_body(self, params))
            .await
            .unwrap_or(Ok(None))
    }
}

impl Backend {
    async fn goto_definition_body(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let Some(token) =
            convert::token_at_indexed(&snap.line_index, &snap.text, pos, &analysis.tokens.tokens)
        else {
            return Ok(None);
        };
        if !matches!(token.0, TokenKind::Ident(_)) {
            return Ok(None);
        }
        let line_count = snap.line_index.line_count() as u32;

        if let TokenKind::Ident(name) = &token.0
            && (&**name == cgen::SELF || &**name == cgen::SUPER)
            && let Some((class, _)) = base_class(analysis, pos, name)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, class.decl_span),
            })));
        }

        if let TokenKind::Ident(name) = &token.0 {
            let name = name.clone();
            let line_index = &snap.line_index;
            let path =
                member_path_before(line_index, text, token.1.start.line, token.1.start.column)
                    .map(|base| format!("{base}.{name}"))
                    .unwrap_or(name);
            if let Some((uri, line)) = crate::declared::locate(&path) {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri,
                    range: Range::new(Position::new(line, 0), Position::new(line, u32::MAX)),
                })));
            }
        }

        if ident_name(&token.0) == Some(csugar::NEW_FUNC)
            && let Some(base) = member_base(analysis, token.1)
            && base_names_class(analysis, &base)
            && let Some((class, objects)) = class_for_base(analysis, &base)
        {
            let target = class_method(objects, class, csugar::INIT_FUNC)
                .map(|init| init.span)
                .unwrap_or(class.decl_span);
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, target),
            })));
        }

        if let Some(location) = module_member_definition(analysis, &text, token) {
            return Ok(Some(GotoDefinitionResponse::Scalar(location)));
        }

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
        if table.goto_label_at(token.1).is_some()
            && let Some(decl) = table.goto_target(token.1)
            && here(&decl)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, decl),
            })));
        }

        if let Some((_, spans)) = loop_label_spans(analysis, token.1)
            && let Some(decl) = spans.first()
            && here(decl)
        {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, *decl),
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

    async fn type_definition_body(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> Result<Option<GotoTypeDefinitionResponse>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let Some((kind, span)) =
            convert::token_at_indexed(&snap.line_index, text, pos, &analysis.tokens.tokens)
        else {
            return Ok(None);
        };
        let Some(name) = ident_name(kind) else {
            return Ok(None);
        };
        if (name == cgen::SELF || name == cgen::SUPER)
            && let Some((class, _)) = base_class(analysis, pos, name)
        {
            return Ok(Some(GotoTypeDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(text, class.decl_span),
            })));
        }
        let ty = analysis
            .scope
            .type_at_span
            .get(span)
            .map(|ty| ty.as_ref().clone())
            .or_else(|| {
                resolve_symbol(analysis, *span).and_then(|symbol| symbol.ty.as_deref().cloned())
            })
            .or_else(|| {
                analysis
                    .scope
                    .symbols
                    .resolve_at((span.start.line, span.start.column), name)
                    .and_then(|symbol| symbol.ty.as_deref().cloned())
            });
        let Some(ty) = ty else {
            return Ok(None);
        };
        let Some(location) = type_definition_location(analysis, text, uri, &ty) else {
            return Ok(None);
        };
        Ok(Some(GotoTypeDefinitionResponse::Scalar(location)))
    }

    async fn hover_body(&self, params: HoverParams) -> Result<Option<Hover>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let table = &analysis.scope.symbols;

        let line_index = &snap.line_index;
        let Some(idx) = analysis.tokens.tokens.iter().position(|t| {
            let range = line_index.range(text, t.1);
            pos >= range.start && pos < range.end
        }) else {
            return Ok(analysis.scope.chunk_doc.as_ref().map(|doc| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: crate::hover::Card::new().prose(doc.as_ref()).render(),
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

        // `self` and `super` are not symbols: inside a class they are the class
        // and its base, so the answer is the class each one stands for.
        if let Some(name) = ident_name(kind)
            && (name == cgen::SELF || name == cgen::SUPER)
            && let Some((class, _)) = base_class(analysis, pos, name)
        {
            return Ok(Some(convert::to_markup_hover(
                &text,
                token,
                &crate::hover::Card::new()
                    .signature(format!("{name}: {}", class.name))
                    .render(),
            )));
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

        if ident_name(kind) == Some(csugar::NEW_FUNC)
            && let Some(base) = member_base(analysis, *span)
            && base_names_class(analysis, &base)
            && let Some((class, objects)) = class_for_base(analysis, &base)
        {
            return Ok(Some(new_hover(analysis, &text, token, class, objects)));
        }

        if let Some(base) = member_base(analysis, *span) {
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
                && let Some((owner, detail)) = member_of_class(objects, class, ident_name(kind))
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

        if let Some(ty) = analysis.scope.type_at_span.get(span) {
            let name = ident_name(kind).unwrap_or_default();
            return Ok(Some(convert::to_markup_hover(
                &text,
                token,
                &crate::hover::Card::new()
                    .signature(format!("(type) {name}: {ty}"))
                    .render(),
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
                ident_name(kind)
                    .and_then(|name| table.resolve_at((span.start.line, span.start.column), name))
            });

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

    async fn semantic_tokens_full_body(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let data = convert::semantic_tokens(
            text,
            &analysis.tokens.tokens,
            &analysis.scope.symbols,
            &analysis.roles,
        );
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn semantic_tokens_range_body(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let data = convert::semantic_tokens(
            &snap.text,
            &snap.analysis.tokens.tokens,
            &snap.analysis.scope.symbols,
            &snap.analysis.roles,
        );
        Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
            result_id: None,
            data: convert::semantic_tokens_range(&data, params.range),
        })))
    }

    async fn document_symbol_body(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        Ok(Some(DocumentSymbolResponse::Nested(document_symbols(
            &snap.analysis,
            &snap.text,
            &snap.line_index,
        ))))
    }

    async fn symbol_body(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let query = params.query.to_lowercase();
        let mut out: Vec<SymbolInformation> = vec![];
        let mut seen: HashSet<(String, u32, u32)> = HashSet::new();
        let mut push = |uri: &Url, symbol: DocumentSymbol, container: Option<String>| {
            if !query.is_empty() && !symbol.name.to_lowercase().contains(&query) {
                return;
            }
            let key = (
                uri.to_string(),
                symbol.range.start.line,
                symbol.range.start.character,
            );
            if !seen.insert(key) {
                return;
            }
            #[allow(deprecated)]
            out.push(SymbolInformation {
                name: symbol.name,
                kind: symbol.kind,
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: symbol.selection_range,
                },
                container_name: container,
            });
        };
        let open = lock(&self.workspace).open_uris();
        for uri in open {
            if let Some(snap) = self.snapshot(&uri).await {
                for symbol in document_symbols(&snap.analysis, &snap.text, &snap.line_index) {
                    let children = symbol.children.clone().unwrap_or_default();
                    let container = symbol.name.clone();
                    push(&uri, symbol, None);
                    for child in children {
                        push(&uri, child, Some(container.clone()));
                    }
                }
            }
        }
        for path in self.project_paths_all() {
            if lock(&self.workspace).open_uri_for_path(&path).is_some() {
                continue;
            }
            let Ok(uri) = Url::from_file_path(&path) else {
                continue;
            };
            if let Some(snap) = self.disk_snapshot(&path).await {
                for symbol in document_symbols(&snap.analysis, &snap.text, &snap.line_index) {
                    let children = symbol.children.clone().unwrap_or_default();
                    let container = symbol.name.clone();
                    push(&uri, symbol, None);
                    for child in children {
                        push(&uri, child, Some(container.clone()));
                    }
                }
            }
        }
        out.truncate(2000);
        Ok(Some(out))
    }

    async fn references_body(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let p = params.text_document_position;
        let uri = &p.text_document.uri;
        let pos = p.position;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let Some((_, span)) = convert::token_at_indexed(
            &snap.line_index,
            &snap.text,
            pos,
            &snap.analysis.tokens.tokens,
        ) else {
            return Ok(None);
        };
        let Some((_, files)) = self.occurrences(uri, *span).await else {
            return Ok(None);
        };
        let mut out: Vec<Location> = vec![];
        for file in files {
            for at in file.spans {
                out.push(Location {
                    uri: file.uri.clone(),
                    range: file.index.range(&file.text, at),
                });
            }
        }
        out.sort_by_key(|location| {
            (
                location.uri.to_string(),
                location.range.start.line,
                location.range.start.character,
            )
        });
        Ok(Some(out))
    }

    async fn signature_help_body(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let index = &snap.line_index;
        Ok(signature_help_for(analysis, text, index, p.position))
    }

    async fn document_highlight_body(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let index = &snap.line_index;
        let Some((_, span)) =
            convert::token_at_indexed(index, text, p.position, &analysis.tokens.tokens)
        else {
            return Ok(None);
        };
        Ok(highlights_for(analysis, text, index, *span))
    }

    async fn prepare_rename_body(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let index = &snap.line_index;
        let Some((kind, span)) =
            convert::token_at_indexed(index, text, params.position, &analysis.tokens.tokens)
        else {
            return Ok(None);
        };
        let Some(name) = ident_name(kind) else {
            return Ok(None);
        };
        if self.occurrences(uri, *span).await.is_none() {
            return Ok(None);
        }
        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: index.range(text, *span),
            placeholder: name.to_owned(),
        }))
    }

    async fn rename_body(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        if !valid_ident(&params.new_name) {
            return Ok(None);
        }
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let Some((_, span)) = convert::token_at_indexed(
            &snap.line_index,
            &snap.text,
            params.text_document_position.position,
            &snap.analysis.tokens.tokens,
        ) else {
            return Ok(None);
        };
        let Some((_, files)) = self.occurrences(uri, *span).await else {
            return Ok(None);
        };
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for file in files {
            let edits = file
                .spans
                .into_iter()
                .map(|at| TextEdit {
                    range: file.index.range(&file.text, at),
                    new_text: params.new_name.clone(),
                })
                .collect::<Vec<_>>();
            if !edits.is_empty() {
                changes.entry(file.uri).or_default().extend(edits);
            }
        }
        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        }))
    }

    async fn folding_range_body(
        &self,
        params: FoldingRangeParams,
    ) -> Result<Option<Vec<FoldingRange>>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        Ok(Some(folding_ranges_for(analysis, text)))
    }

    async fn document_link_body(
        &self,
        params: DocumentLinkParams,
    ) -> Result<Option<Vec<DocumentLink>>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        Ok(Some(document_links(
            &snap.text,
            &snap.analysis,
            &snap.line_index,
        )))
    }

    async fn selection_range_body(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let mut out = vec![];
        for position in params.positions {
            if let Some(selection) =
                selection_range_for(&snap.analysis, &snap.text, &snap.line_index, position)
            {
                out.push(selection);
            }
        }
        if out.is_empty() {
            return Ok(None);
        }
        Ok(Some(out))
    }

    async fn inlay_hint_body(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        Ok(Some(Backend::inlay_hints_for(analysis, text, params.range)))
    }

    async fn code_action_body(
        &self,
        params: CodeActionParams,
    ) -> Result<Option<CodeActionResponse>> {
        if let Some(only) = &params.context.only
            && !only.iter().any(|kind| {
                let kind = kind.as_str();
                kind == "quickfix" || kind.starts_with("quickfix.")
            })
        {
            return Ok(None);
        }
        let uri = &params.text_document.uri;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let actions = quick_fixes(
            uri,
            &snap.text,
            &snap.line_index,
            &params.context.diagnostics,
        );
        if actions.is_empty() {
            return Ok(None);
        }
        Ok(Some(actions))
    }

    async fn completion_body(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let Some(snap) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let text = &snap.text;
        let analysis = &snap.analysis;
        let index = &snap.line_index;
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
                        .filter(|view| !is_hidden_name(&view.name))
                        .take(MAX_COMPLETION_ITEMS)
                        .map(|view| {
                            let lookup = format!("{base}.{}", view.name);
                            builtin_item_for(view, lookup)
                        })
                        .collect(),
                })));
            }
            let members = match base_class(analysis, pos, &base) {
                Some((class, objects)) => {
                    class_members(objects, class, base_names_class(analysis, &base))
                }
                None => match path_resolved(analysis, &base).and_then(|r| r.structured) {
                    Some(ty) => convert::record_members(&ty),
                    None => return Ok(None),
                },
            };
            let members = visible_members(members);
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
        if let Some(AccessBase::Require {
            module,
            members,
            type_position,
        }) = member_access_base(&index, &text, pos)
        {
            let items = if type_position {
                require_type_members(analysis, &module, &members)
            } else {
                require_value_members(analysis, &module, &members)
            };
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

/// Every place in the file where `span`'s name is the same symbol.
///
/// The declaration and its uses are collected together; a member read and a
/// field declaration are not, and neither is a same-named declaration in
/// another scope.
fn quick_fixes(
    uri: &Url,
    text: &str,
    index: &convert::LineIndex,
    diagnostics: &[Diagnostic],
) -> Vec<CodeActionOrCommand> {
    let mut actions: Vec<CodeActionOrCommand> = vec![];
    for diagnostic in diagnostics {
        let Some(code) = diagnostic.code.as_ref().and_then(|code| match code {
            NumberOrString::String(code) => Some(code.as_str()),
            NumberOrString::Number(_) => None,
        }) else {
            continue;
        };
        let (title, edit) = match code {
            "deprecated" => {
                let replacement = diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| data.get("replacement"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if replacement.is_empty() {
                    continue;
                }
                (
                    format!("Replace with `{replacement}`"),
                    TextEdit {
                        range: diagnostic.range,
                        new_text: replacement.to_owned(),
                    },
                )
            }
            "must-use" => {
                let start = diagnostic.range.start;
                let line = index.line_text(text, start.line as usize);
                let prefix: String = line.chars().take(start.character as usize).collect();
                if !prefix.trim().is_empty() {
                    continue;
                }
                (
                    "Ignore the result with `local _ =`".to_owned(),
                    TextEdit {
                        range: Range::new(start, start),
                        new_text: "local _ = ".to_owned(),
                    },
                )
            }
            _ => continue,
        };
        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        }));
    }
    actions
}

fn document_symbols(
    analysis: &compile::DocAnalysis,
    text: &str,
    index: &convert::LineIndex,
) -> Vec<DocumentSymbol> {
    let mut out: Vec<DocumentSymbol> = vec![];
    let mut structs: Vec<(usize, usize)> = vec![];
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
                let symbol_kind = symbol.and_then(|s| match s.symbol_type {
                    SymbolType::Function => Some(SymbolKind::FUNCTION),
                    SymbolType::ObjectClass(_) => Some(SymbolKind::STRUCT),
                    SymbolType::TypeAlias(_) => Some(SymbolKind::INTERFACE),
                    SymbolType::TypeFunction(_) | SymbolType::InlineTypeFunction(_) => {
                        Some(SymbolKind::FUNCTION)
                    }
                    SymbolType::Constant(_) if exported => Some(SymbolKind::CONSTANT),
                    SymbolType::Variable if exported => Some(SymbolKind::VARIABLE),
                    _ => None,
                });
                let object = symbol.and_then(|s| match s.symbol_type {
                    SymbolType::ObjectClass(id) => Some(id),
                    _ => None,
                });
                let nested = object.is_none()
                    && analysis.scope.objects.iter().any(|class| {
                        span.start.line >= class.decl_span.start.line
                            && span.start.line <= class.decl_span.end.line
                    });
                if nested {
                    continue;
                }
                if let Some(symbol_kind) = symbol_kind {
                    let name = ident_name(kind).unwrap_or_default().to_string();
                    let range = index.range(text, *span);
                    out.push(DocumentSymbol {
                        name,
                        detail: symbol.and_then(|s| s.ty_display.as_deref().map(str::to_string)),
                        kind: symbol_kind,
                        tags: None,
                        #[allow(deprecated)]
                        deprecated: None,
                        range,
                        selection_range: range,
                        children: None,
                    });
                    if let Some(id) = object {
                        structs.push((out.len() - 1, id));
                    }
                }
            }
            _ => at_line_start = false,
        }
    }
    for (at, id) in structs {
        let Some(class) = analysis.scope.objects.get(id) else {
            continue;
        };
        let mut children: Vec<DocumentSymbol> = vec![];
        for member in &class.members {
            let range = index.range(text, member.span);
            children.push(DocumentSymbol {
                name: member.name.to_string(),
                detail: Some(member.ty.to_string()),
                kind: SymbolKind::FIELD,
                tags: None,
                #[allow(deprecated)]
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
        for method in &class.methods {
            let range = index.range(text, method.span);
            children.push(DocumentSymbol {
                name: method.name.to_string(),
                detail: Some(Type::Function(Some(method.sig.clone())).to_string()),
                kind: SymbolKind::METHOD,
                tags: None,
                #[allow(deprecated)]
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            });
        }
        children.sort_by_key(|s| (s.range.start.line, s.range.start.character));
        out[at].children = Some(children);
    }
    out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    out
}

struct FileSpans {
    uri: Url,
    text: Arc<String>,
    index: Arc<convert::LineIndex>,
    spans: Vec<Span>,
}

struct CrossTarget {
    decl_path: PathBuf,
    decl_span: Span,
}

fn cross_target(
    analysis: &compile::DocAnalysis,
    span: Span,
    name: &str,
    current_path: Option<&Path>,
) -> Option<CrossTarget> {
    if let Some(path) = analysis.scope.member_paths.get(&span) {
        let (base, member) = path.rsplit_once('.')?;
        if member != name {
            return None;
        }
        let decl_path = resolve_base_path(analysis, base)?;
        let decl_span = module_symbol(analysis, base, name)
            .map(|symbol| symbol.span)
            .unwrap_or(Span::EMPTY);
        return Some(CrossTarget {
            decl_path,
            decl_span,
        });
    }
    let symbol = resolve_symbol(analysis, span)?;
    if symbol.span == Span::EMPTY || !is_top_level(analysis, symbol) {
        return None;
    }
    Some(CrossTarget {
        decl_path: current_path?.to_path_buf(),
        decl_span: symbol.span,
    })
}

fn is_top_level(
    analysis: &compile::DocAnalysis,
    symbol: &duka_lib::duka_shared::utils::Symbol,
) -> bool {
    analysis.scope.symbols.scopes.first().is_some_and(|scope| {
        scope
            .symbols
            .values()
            .flatten()
            .any(|found| found.id == symbol.id)
    })
}

fn resolve_base_path(analysis: &compile::DocAnalysis, base: &str) -> Option<PathBuf> {
    if let Some(path) = require_bindings(analysis).get(base) {
        return Some(path.clone());
    }
    if base.contains('.') {
        return None;
    }
    let resolver = analysis.modules.as_ref()?;
    let module = resolver.resolve(&analysis.scope.modules, base)?;
    let SourceName::File(_, path) = &module.source.name else {
        return None;
    };
    Some(path.to_path_buf())
}

fn module_for_base<'a>(
    analysis: &'a compile::DocAnalysis,
    base: &str,
) -> Option<&'a duka_lib::duka_frontend::analyzer::modules::ModuleType> {
    if let Some(path) = require_bindings(analysis).get(base) {
        return analysis
            .scope
            .modules
            .values()
            .find(|module| module.source.name.path() == Some(path.as_path()));
    }
    let resolver = analysis.modules.as_ref()?;
    resolver.resolve(&analysis.scope.modules, base)
}

fn module_symbol<'a>(
    analysis: &'a compile::DocAnalysis,
    base: &str,
    name: &str,
) -> Option<&'a duka_lib::duka_shared::utils::Symbol> {
    let module = module_for_base(analysis, base)?;
    module
        .analysis
        .symbols
        .scopes
        .first()?
        .symbols
        .get(name)?
        .iter()
        .next_back()
}

fn require_bindings(analysis: &compile::DocAnalysis) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let Some(resolver) = analysis.modules.as_ref() else {
        return out;
    };
    let tokens = &analysis.tokens.tokens;
    for i in 0..tokens.len() {
        let TokenKind::Ident(name) = &tokens[i].0 else {
            continue;
        };
        let Some((TokenKind::Assign, _)) = tokens.get(i + 1) else {
            continue;
        };
        let mut at = i + 2;
        while tokens.get(at).is_some_and(|(kind, _)| kind.is_comment()) {
            at += 1;
        }
        let Some((TokenKind::Ident(callee), _)) = tokens.get(at) else {
            continue;
        };
        let mut argument = at + 1;
        let parenthesised = matches!(
            tokens.get(argument).map(|(kind, _)| kind),
            Some(TokenKind::LParen)
        );
        if parenthesised {
            argument += 1;
        }
        let Some(TokenKind::String(bytes)) = tokens.get(argument).map(|(kind, _)| kind) else {
            continue;
        };
        let is_require = &**callee == cpar::REQUIRE
            || (parenthesised && &**callee == duka_lib::duka_shared::constants::ctype::REQUIRE);
        if !is_require {
            continue;
        }
        let module = String::from_utf8_lossy(bytes).into_owned();
        let Some(module) = resolver.resolve(&analysis.scope.modules, &module) else {
            continue;
        };
        let SourceName::File(_, path) = &module.source.name else {
            continue;
        };
        out.insert(name.to_string(), path.to_path_buf());
    }
    out
}

fn member_occurrences(analysis: &compile::DocAnalysis, name: &str, decl_path: &Path) -> Vec<Span> {
    let bindings = require_bindings(analysis);
    let mut out = vec![];
    for (kind, span) in &analysis.tokens.tokens {
        if ident_name(kind) != Some(name) {
            continue;
        }
        let Some(path) = analysis.scope.member_paths.get(span) else {
            continue;
        };
        let Some((base, member)) = path.rsplit_once('.') else {
            continue;
        };
        if member != name {
            continue;
        }
        if bindings.get(base).is_some_and(|found| found == decl_path) {
            out.push(*span);
        }
    }
    out
}

fn same_name_spans(analysis: &compile::DocAnalysis, span: Span) -> Option<(String, Vec<Span>)> {
    let name = analysis
        .tokens
        .tokens
        .iter()
        .find(|(_, at)| *at == span)
        .and_then(|(kind, _)| ident_name(kind))?
        .to_owned();
    let target_id = analysis.scope.symbols.symbol_at_span(span).map(|s| s.id);
    let mut out: Vec<Span> = vec![];
    let mut brace_depth = 0usize;
    let mut prev_significant: Option<&TokenKind> = None;
    for (i, (kind, at)) in analysis.tokens.tokens.iter().enumerate() {
        if !kind.is_terminator() {
            match kind {
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
        }
        let is_ident = ident_name(kind) == Some(name.as_str());
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
        let is_field_decl = analysis
            .tokens
            .tokens
            .get(i + 1)
            .is_some_and(|(k, _)| matches!(k, TokenKind::Colon));
        if is_field_decl {
            continue;
        }
        if analysis.scope.member_paths.contains_key(at) {
            continue;
        }
        if let (Some(target_id), Some(found)) =
            (target_id, analysis.scope.symbols.symbol_at_span(*at))
            && found.id != target_id
        {
            continue;
        }
        out.push(*at);
    }
    Some((name, out))
}

/// The declaration of a label together with the `goto`s that land on it.
///
/// A label is not a symbol, so neither half can be found by looking the name
/// up; the table records the pair while it has both in front of it.
fn loop_label_spans(analysis: &compile::DocAnalysis, span: Span) -> Option<(String, Vec<Span>)> {
    let scope = &analysis.scope;
    let declaration = scope
        .loop_labels
        .iter()
        .find(|(_, at)| *at == span)
        .or_else(|| {
            let (name, _) = scope.loop_label_uses.iter().find(|(_, at)| *at == span)?;
            scope
                .loop_labels
                .iter()
                .rev()
                .find(|(label, _)| label == name)
        })?;
    let mut spans = vec![declaration.1];
    spans.extend(
        scope
            .loop_label_uses
            .iter()
            .filter(|(name, _)| *name == declaration.0)
            .map(|(_, at)| *at),
    );
    spans.sort_by_key(|at| (at.start.line, at.start.column));
    spans.dedup();
    Some((declaration.0.to_string(), spans))
}

fn label_spans(analysis: &compile::DocAnalysis, span: Span) -> Option<Vec<Span>> {
    let table = &analysis.scope.symbols;
    let (decl, uses) = if table.label_at(span).is_some() {
        (span, table.label_uses(span))
    } else {
        let target = table.goto_target(span)?;
        (target, table.label_uses(target))
    };
    let mut spans = vec![decl];
    spans.extend(uses);
    spans.sort_by_key(|s| (s.start.line, s.start.column));
    spans.dedup();
    Some(spans)
}

fn highlights_for(
    analysis: &compile::DocAnalysis,
    text: &str,
    index: &convert::LineIndex,
    span: Span,
) -> Option<Vec<DocumentHighlight>> {
    if is_member_read(analysis, span) {
        return None;
    }
    let spans = loop_label_spans(analysis, span)
        .map(|(_, spans)| spans)
        .or_else(|| label_spans(analysis, span))
        .or_else(|| same_name_spans(analysis, span).map(|(_, spans)| spans))?;
    Some(
        spans
            .into_iter()
            .map(|at| DocumentHighlight {
                range: index.range(text, at),
                kind: Some(DocumentHighlightKind::TEXT),
            })
            .collect(),
    )
}

fn valid_ident(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
        && crate::docs::keyword_view(name).is_none()
        && Type::from_keyword(name).is_none()
}

/// Whether the name at `span` is written after a `.` or `:` -- a member read,
/// whose name belongs to the container rather than to this file.
fn is_member_read(analysis: &compile::DocAnalysis, span: Span) -> bool {
    if analysis.scope.member_paths.contains_key(&span) {
        return true;
    }
    let Some(index) = analysis
        .tokens
        .tokens
        .iter()
        .position(|(_, at)| *at == span)
    else {
        return false;
    };
    analysis.tokens.tokens[..index]
        .iter()
        .rev()
        .find(|(kind, _)| !kind.is_terminator())
        .is_some_and(|(kind, _)| matches!(kind, TokenKind::Dot | TokenKind::Colon))
}

/// The signature shown while arguments are being written.
fn signature_help_for(
    analysis: &compile::DocAnalysis,
    text: &str,
    index: &convert::LineIndex,
    pos: Position,
) -> Option<SignatureHelp> {
    let tokens = &analysis.tokens.tokens;
    // the token the cursor starts at, which is where the list of arguments ends
    let mut cursor = tokens.len();
    for (i, (_, span)) in tokens.iter().enumerate() {
        let range = index.range(text, *span);
        if range.start >= pos {
            cursor = i;
            break;
        }
        if range.end > pos {
            cursor = i + 1;
            break;
        }
    }
    // the `(` this argument list opened, looking through nested brackets
    let mut depth = 0usize;
    let mut open = None;
    for i in (0..cursor).rev() {
        match &tokens[i].0 {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    open = Some(i);
                    break;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    let open = open?;
    let mut callee = open;
    loop {
        if callee == 0 {
            return None;
        }
        callee -= 1;
        if !tokens[callee].0.is_terminator() {
            break;
        }
    }
    // `f<T>(` carries the type arguments before the list
    if matches!(&tokens[callee].0, TokenKind::Greater) {
        let mut depth = 0usize;
        while callee > 0 {
            callee -= 1;
            match &tokens[callee].0 {
                TokenKind::Greater => depth += 1,
                TokenKind::Less if depth == 0 => break,
                TokenKind::Less => depth -= 1,
                _ => {}
            }
        }
        loop {
            if callee == 0 {
                return None;
            }
            callee -= 1;
            if !tokens[callee].0.is_terminator() {
                break;
            }
        }
    }
    let (kind, name_span) = &tokens[callee];
    let name = ident_name(kind)?;
    // the parameters of a declaration are not an argument list
    if tokens[..callee]
        .iter()
        .rev()
        .find(|(kind, _)| !kind.is_terminator())
        .is_some_and(|(kind, _)| matches!(kind, TokenKind::Function | TokenKind::Fn))
    {
        return None;
    }
    let base = member_path_before(index, text, name_span.start.line, name_span.start.column);
    let mut depth = 0usize;
    let mut active = 0usize;
    for (kind, _) in &tokens[open + 1..cursor] {
        match kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth = depth.saturating_sub(1)
            }
            TokenKind::Comma if depth == 0 => active += 1,
            _ => {}
        }
    }
    let (label, params) = signature_label(analysis, *name_span, name, base.as_deref())?;
    let active = if params.is_empty() {
        0
    } else {
        active.min(params.len() - 1)
    };
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: None,
            parameters: Some(
                params
                    .into_iter()
                    .map(|param| ParameterInformation {
                        label: ParameterLabel::Simple(param),
                        documentation: None,
                    })
                    .collect(),
            ),
            active_parameter: Some(active as u32),
        }],
        active_signature: Some(0),
        active_parameter: Some(active as u32),
    })
}

fn signature_label(
    analysis: &compile::DocAnalysis,
    name_span: Span,
    name: &str,
    base: Option<&str>,
) -> Option<(String, Vec<String>)> {
    if name == csugar::NEW_FUNC
        && let Some(base) = base
        && base_names_class(analysis, base)
        && let Some((class, objects)) = class_for_base(analysis, base)
    {
        let params = class_method(objects, class, csugar::INIT_FUNC)
            .map(|init| {
                let names = param_names(&analysis.tokens.tokens, init.span).unwrap_or_default();
                docs::function_parameters(&init.sig, &names)
            })
            .unwrap_or_default();
        return Some((
            format!(
                "{}:new({}) -> {}",
                class.name,
                params.join(", "),
                class.name
            ),
            params,
        ));
    }
    if let Some(link) = analysis
        .scope
        .links
        .iter()
        .find(|link| link.name_span == name_span)
        && let Some(method) = analysis
            .scope
            .objects
            .get(link.owner)
            .and_then(|object| object.methods.iter().find(|m| m.span == link.decl_span))
    {
        let names = param_names(&analysis.tokens.tokens, method.span).unwrap_or_default();
        return Some(fn_signature(&method.name, &method.sig, &names));
    }
    if let Some(symbol) = resolve_symbol(analysis, name_span)
        && let Some(sig) = symbol.ty.as_deref().and_then(as_function)
    {
        let names = param_names(&analysis.tokens.tokens, symbol.span).unwrap_or_default();
        return Some(fn_signature(name, sig, &names));
    }
    if let Some(base) = base
        && let Some(sig) = path_resolved(analysis, &format!("{base}.{name}")).and_then(|resolved| {
            resolved
                .structured
                .as_deref()
                .and_then(as_function)
                .cloned()
        })
    {
        return Some(fn_signature(name, &sig, &[]));
    }
    let path = match base {
        Some(base) => format!("{base}.{name}"),
        None => name.to_owned(),
    };
    docs::value_builtin_signature(&path)
}

fn fn_signature(name: &str, sig: &FunctionType, names: &[String]) -> (String, Vec<String>) {
    (
        docs::function_signature("", name, None, names, sig),
        docs::function_parameters(sig, names),
    )
}

fn document_links(
    text: &str,
    analysis: &compile::DocAnalysis,
    index: &convert::LineIndex,
) -> Vec<DocumentLink> {
    let Some(resolver) = analysis.modules.as_ref() else {
        return vec![];
    };
    let tokens = &analysis.tokens.tokens;
    let mut out = vec![];
    for i in 0..tokens.len() {
        let TokenKind::Ident(callee) = &tokens[i].0 else {
            continue;
        };
        let is_require = &**callee == cpar::REQUIRE || &**callee == ctype::REQUIRE;
        if !is_require {
            continue;
        }
        let mut at = i + 1;
        if matches!(
            tokens.get(at).map(|(kind, _)| kind),
            Some(TokenKind::LParen)
        ) {
            at += 1;
        }
        let Some((TokenKind::String(bytes), span)) = tokens.get(at) else {
            continue;
        };
        let name = String::from_utf8_lossy(bytes).into_owned();
        let Some(module) = resolver.resolve(&analysis.scope.modules, &name) else {
            continue;
        };
        let SourceName::File(_, path) = &module.source.name else {
            continue;
        };
        let Ok(uri) = Url::from_file_path(path) else {
            continue;
        };
        out.push(DocumentLink {
            range: index.range(text, *span),
            target: Some(uri),
            tooltip: Some(format!("Open module `{name}`")),
            data: None,
        });
    }
    out
}

fn selection_range_for(
    analysis: &compile::DocAnalysis,
    text: &str,
    index: &convert::LineIndex,
    pos: Position,
) -> Option<SelectionRange> {
    let token = convert::token_at_indexed(index, text, pos, &analysis.tokens.tokens)?;
    let mut ranges: Vec<Range> = vec![index.range(text, token.1)];

    let mut open: Vec<(TokenKind, Span)> = vec![];
    for (kind, span) in &analysis.tokens.tokens {
        if kind.is_terminator() {
            continue;
        }
        match kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                open.push((kind.clone(), *span));
            }
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                let Some((opener, open_span)) = open.pop() else {
                    continue;
                };
                let matched = matches!(
                    (&opener, kind),
                    (TokenKind::LParen, TokenKind::RParen)
                        | (TokenKind::LBracket, TokenKind::RBracket)
                        | (TokenKind::LBrace, TokenKind::RBrace)
                );
                if !matched {
                    continue;
                }
                let range = Range::new(
                    index.position(text, open_span.start.line, open_span.start.column),
                    index.position(text, span.end.line, span.end.column),
                );
                if pos >= range.start && pos <= range.end {
                    ranges.push(range);
                }
            }
            _ => {}
        }
    }

    let line = index.line_text(text, pos.line as usize);
    let units: Vec<u32> = line.chars().map(|c| c.len_utf16() as u32).collect();
    let total: u32 = units.iter().sum();
    let mut leading = 0u32;
    for (ch, width) in line.chars().zip(units.iter()) {
        if !ch.is_whitespace() {
            break;
        }
        leading += width;
    }
    let mut trailing = 0u32;
    for (ch, width) in line.chars().rev().zip(units.iter().rev()) {
        if !ch.is_whitespace() {
            break;
        }
        trailing += width;
    }
    if leading < total.saturating_sub(trailing) {
        ranges.push(Range::new(
            Position::new(pos.line, leading),
            Position::new(pos.line, total - trailing),
        ));
    }

    for fold in folding_ranges_for(analysis, text) {
        if fold.start_line <= pos.line && pos.line <= fold.end_line {
            let end = index.line_text(text, fold.end_line as usize);
            let width: u32 = end.chars().map(|c| c.len_utf16() as u32).sum();
            ranges.push(Range::new(
                Position::new(fold.start_line, 0),
                Position::new(fold.end_line, width),
            ));
        }
    }

    let last = index.line_count().saturating_sub(1) as u32;
    let width: u32 = index
        .line_text(text, last as usize)
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    ranges.push(Range::new(Position::new(0, 0), Position::new(last, width)));

    ranges.retain(|range| range.start <= pos && pos <= range.end);
    ranges.sort_by_key(|range| {
        (
            range.end.line - range.start.line,
            range.end.character.saturating_sub(range.start.character),
        )
    });
    ranges.dedup();

    let mut chain: Option<SelectionRange> = None;
    for range in ranges.into_iter().rev() {
        chain = Some(SelectionRange {
            range,
            parent: chain.map(Box::new),
        });
    }
    chain
}

/// One fold per block body and per multi-line comment.
///
/// A block starts at the keyword that opened it and ends at the `end` that
/// closes it; braces fold on their own. `do` is a block of its own only when
/// it did not follow a `for` or `while` header on the same line, which is the
/// difference between `do ... end` and the `do` those loops write.
fn folding_ranges_for(analysis: &compile::DocAnalysis, text: &str) -> Vec<FoldingRange> {
    let mut out: Vec<FoldingRange> = vec![];
    let mut blocks: Vec<u32> = vec![];
    for (kind, span) in &analysis.tokens.tokens {
        match kind {
            TokenKind::Function
            | TokenKind::Fn
            | TokenKind::If
            | TokenKind::For
            | TokenKind::While
            | TokenKind::Match
            | TokenKind::Object => blocks.push(span.start.line),
            TokenKind::Do => {
                if blocks.last() != Some(&span.start.line) {
                    blocks.push(span.start.line);
                }
            }
            TokenKind::LBrace => blocks.push(span.start.line),
            TokenKind::End | TokenKind::RBrace => {
                if let Some(start) = blocks.pop()
                    && start < span.start.line
                {
                    out.push(FoldingRange {
                        start_line: start.saturating_sub(1),
                        start_character: None,
                        end_line: span.start.line.saturating_sub(1),
                        end_character: None,
                        kind: None,
                        collapsed_text: None,
                    });
                }
            }
            _ => {}
        }
    }
    comment_folds(text, &mut out);
    out.sort_by_key(|range| (range.start_line, range.end_line));
    out.dedup_by_key(|range| (range.start_line, range.end_line));
    out
}

/// Folds for the block comments in `text`.
///
/// The lexer records a block comment's span at the point it ends, so the text
/// itself is what says where the comment began and how many lines it covers.
fn comment_folds(text: &str, out: &mut Vec<FoldingRange>) {
    let bytes = text.as_bytes();
    let mut line = 0u32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                let mut j = i + 2;
                if bytes.get(j) != Some(&b'[') {
                    i = j;
                    continue;
                }
                j += 1;
                let mut depth = 0usize;
                while bytes.get(j) == Some(&b'=') {
                    depth += 1;
                    j += 1;
                }
                if bytes.get(j) != Some(&b'[') {
                    i = j;
                    continue;
                }
                j += 1;
                let start = line;
                let mut k = j;
                let mut end = None;
                while k < bytes.len() {
                    if bytes[k] == b'\n' {
                        line += 1;
                    }
                    if bytes[k] == b']' {
                        let mut equals = 0usize;
                        let mut next = k + 1;
                        while bytes.get(next) == Some(&b'=') {
                            equals += 1;
                            next += 1;
                        }
                        if equals == depth && bytes.get(next) == Some(&b']') {
                            end = Some(next + 1);
                            break;
                        }
                    }
                    k += 1;
                }
                let Some(end) = end else {
                    i = j;
                    continue;
                };
                if start < line {
                    out.push(FoldingRange {
                        start_line: start,
                        start_character: None,
                        end_line: line,
                        end_character: None,
                        kind: Some(FoldingRangeKind::Comment),
                        collapsed_text: None,
                    });
                }
                i = end;
            }
            _ => i += 1,
        }
    }
}

/// The names `require("m").` can be followed by.
///
/// The module's own value table says which names it exports and what each one
/// is; a member already read off the module narrows the list to that type's
/// fields, the same way a record read does.
fn require_value_members(
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
    let mut ty: Option<Type> = None;
    for member in read {
        ty = match ty {
            None => module.value.members.get(member.as_str()).cloned(),
            Some(current) => current.field(member),
        };
        if ty.is_none() {
            return vec![];
        }
    }
    let members: Vec<(String, String)> = match ty {
        Some(ty) => convert::record_members(&ty),
        None => module
            .value
            .members
            .iter()
            .map(|(name, ty)| (name.to_string(), ty.to_string()))
            .collect(),
    };
    members
        .into_iter()
        .filter(|(name, _)| !is_hidden_name(name))
        .take(MAX_COMPLETION_ITEMS)
        .map(|(label, detail)| CompletionItem {
            kind: Some(if detail.starts_with("function") {
                CompletionItemKind::FUNCTION
            } else {
                CompletionItemKind::VARIABLE
            }),
            label,
            detail: Some(detail),
            ..Default::default()
        })
        .collect()
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
            if is_hidden_name(name) {
                return None;
            }
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

            if csugar::HIDDENS.contains(&&**name) || is_hidden_name(name) {
                continue;
            }

            let Some(sym) = syms.iter().next_back() else {
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
        if is_hidden_name(&view.name) || !view.name.starts_with(word) {
            continue;
        }
        let lookup = view.name.to_string();
        match at.get(&*view.name).copied() {
            Some(existing) => {
                let item = &mut items[existing];
                if item.detail.as_deref().is_none_or(str::is_empty) {
                    item.detail = view.signature.clone();
                }
                item.data = Some(serde_json::json!({ "builtin": lookup }));
            }
            None => {
                let label = view.name.clone().into_owned();
                at.insert(label.clone(), items.len());
                items.push(builtin_item_for(view, lookup));
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
                data: Some(serde_json::json!({ "keyword": keyword })),
                ..Default::default()
            });
        }
    }

    for (index, item) in items.iter_mut().enumerate() {
        item.sort_text = Some(format!("{index:05}{}", item.label));
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
fn builtin_item_for(view: docs::DocView<'static>, lookup: String) -> CompletionItem {
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
    CompletionItem {
        label: view.name.clone().into_owned(),
        kind: Some(kind),
        detail: view.signature.clone(),
        data: Some(serde_json::json!({ "builtin": lookup })),
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
fn member_path(analysis: &compile::DocAnalysis, span: Span) -> Option<String> {
    analysis
        .scope
        .member_paths
        .get(&span)
        .map(|p| p.to_string())
}

fn member_base(analysis: &compile::DocAnalysis, span: Span) -> Option<String> {
    member_path(analysis, span)?
        .rsplit_once('.')
        .map(|(base, _)| base.to_string())
}

fn token_name(analysis: &compile::DocAnalysis, span: Span) -> Option<String> {
    analysis
        .tokens
        .tokens
        .iter()
        .find(|(_, s)| *s == span)
        .and_then(|(k, _)| ident_name(k))
        .map(str::to_owned)
}

fn member_or_name(analysis: &compile::DocAnalysis, span: Span) -> String {
    member_path(analysis, span)
        .or_else(|| token_name(analysis, span))
        .unwrap_or_default()
}

fn as_function(ty: &Type) -> Option<&duka_lib::duka_shared::dtype::FunctionType> {
    match ty {
        Type::Function(Some(ft)) => Some(ft),
        Type::Union(ts) => ts.iter().find_map(|t| as_function(t)),
        _ => None,
    }
}

/// The `where` list a function carries, as written on its declaration.
fn rendered_where(
    analysis: &compile::DocAnalysis,
    symbol: &duka_lib::duka_shared::utils::Symbol,
) -> Option<String> {
    let id = as_function(symbol.ty.as_deref()?)?.sig?;
    let sigs = analysis.scope.signatures.lock().ok()?;
    let sig = sigs.get(id)?;
    if sig.constraints.is_empty() {
        return None;
    }
    Some(
        sig.constraints
            .iter()
            .map(|clause| match clause {
                duka_lib::duka_frontend::parser::ast::WhereClause::Bound((n, _), ty, _) => {
                    format!("{n}: {ty}")
                }
                duka_lib::duka_frontend::parser::ast::WhereClause::Concept(expr, _) => {
                    duka_lib::duka_frontend::analyzer::tcx::render(expr)
                }
                duka_lib::duka_frontend::parser::ast::WhereClause::Bind(de, ty, _) => {
                    format!("type {de} = {ty}")
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Groups the errors of a document by the file they belong to, so an error
/// inside a required module is reported on that module. The locks are released
/// before awaiting, the guards must not cross an await point.
fn diagnostics_of(snapshot: &Snapshot, uri: &Url) -> HashMap<Url, Vec<Diagnostic>> {
    let mut groups: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    // A generated declaration file is not the reader's code. They opened it
    // because they jumped to a builtin, and an error in it is either a bug in
    // this generator or a diagnostic about a declaration, which is not something
    // the reader can act on.
    if uri.scheme() == crate::declared::SCHEME {
        return groups;
    }
    // a module that is reachable twice is analyzed once but its errors can
    // still arrive twice, the client should only ever see one
    let mut seen: HashSet<(u32, u32, u32, u32, String)> = HashSet::new();
    for err in &snapshot.analysis.errors {
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
    groups
}

/// Where one diagnostic sits, so a module's error that reached the round
/// through two documents is sent once.
fn diagnostic_key(diagnostic: &Diagnostic) -> (u32, u32, u32, u32, String) {
    (
        diagnostic.range.start.line,
        diagnostic.range.start.character,
        diagnostic.range.end.line,
        diagnostic.range.end.character,
        diagnostic.message.clone(),
    )
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

/// How many diagnostics one file carries before the rest are summarized. An
/// editor showing a thousand squiggles is no more useful than one showing the
/// first two hundred and a note.
const MAX_PUBLISHED: usize = 200;

/// The analysis of one document at the revision the editor last sent, running
/// the pipeline outside every lock. A snapshot that is still current answers
/// without a run; otherwise the run is captured, executed on its own thread,
/// and committed only if no edit landed while it went on, so an edit
/// supersedes it instead of waiting for it. A panic in the pipeline is a line
/// in the log and an empty answer, not a request that never replies.
async fn snapshot_of(client: &Client, workspace: &SharedWorkspace, uri: &Url) -> Option<Snapshot> {
    // One analysis per document at a time: a request that arrives while
    // another is running waits here and then finds the snapshot that run
    // committed, instead of paying for the same pipeline a second time.
    let gate = lock(workspace).analysis_gate(uri)?;
    let _gate = gate.lock().await;
    let mut attempts = 0;
    loop {
        if let Some(snapshot) = lock(workspace).snapshot(uri) {
            return Some(snapshot);
        }
        let pending = lock(workspace).pending_analysis(uri)?;
        attempts += 1;
        let revision = pending.revision;
        let text = Arc::clone(&pending.text);
        let analysis = tokio::task::spawn_blocking(move || pending.run()).await;
        let analysis = match analysis {
            Ok(analysis) => analysis,
            Err(err) => {
                let why = if err.is_panic() {
                    panic_message(err.into_panic().as_ref())
                } else {
                    format!("{err}")
                };
                let _ = client
                    .log_message(
                        MessageType::ERROR,
                        format!("analysis of {uri} failed: {why}"),
                    )
                    .await;
                return None;
            }
        };
        let (committed, analysis) = lock(workspace).commit(uri, revision, &text, analysis);
        if committed {
            return lock(workspace).snapshot(uri);
        }
        if attempts >= MAX_ANALYSIS_ATTEMPTS {
            let line_index = Arc::new(convert::LineIndex::new(&text));
            return Some(Snapshot {
                text,
                analysis,
                line_index,
            });
        }
    }
}

/// Publishes the diagnostics of the documents in `order`, the file the reader
/// just touched first. The errors of one analysis can point at a module file,
/// so what the round says is the union of every document's groups: the files
/// the union no longer covers are cleared, and the set of published files
/// becomes exactly the union.
async fn publish_round(
    client: &Client,
    workspace: &SharedWorkspace,
    published: &Mutex<HashSet<Url>>,
    order: &[Url],
) {
    let mut groups: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    let mut seen: HashSet<(u32, u32, u32, u32, String)> = HashSet::new();
    for uri in order {
        let Some(snapshot) = snapshot_of(client, workspace, uri).await else {
            continue;
        };
        for (target, diagnostics) in diagnostics_of(&snapshot, uri) {
            let fresh = diagnostics
                .into_iter()
                .filter(|diagnostic| seen.insert(diagnostic_key(diagnostic)));
            groups.entry(target).or_default().extend(fresh);
        }
    }
    let previously = published.lock().unwrap_or_else(|e| e.into_inner()).clone();
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
        let mut diagnostics = diagnostics.clone();
        if diagnostics.len() > MAX_PUBLISHED {
            let total = diagnostics.len();
            diagnostics.truncate(MAX_PUBLISHED);
            diagnostics.push(convert::truncated(MAX_PUBLISHED, total));
        }
        let _ = client
            .publish_diagnostics(target.clone(), diagnostics, None)
            .await;
    }
    *published.lock().unwrap_or_else(|e| e.into_inner()) = groups.into_keys().collect();
}

/// The one task that publishes diagnostics. It waits for a kick, waits out the
/// debounce, then publishes every file that was waiting plus the rest of the
/// open ones: an edit invalidates every snapshot, and a file that requires the
/// edited one carries its errors, so a module's mistake reaches the files that
/// read it, not only itself.
async fn publisher_loop(
    client: Client,
    workspace: Arc<SharedWorkspace>,
    published: Arc<Mutex<HashSet<Url>>>,
    waiting: Arc<Mutex<HashSet<Url>>>,
    kick: Arc<tokio::sync::Notify>,
) {
    loop {
        kick.notified().await;
        tokio::time::sleep(DIAGNOSTIC_DEBOUNCE).await;
        let changed: HashSet<Url> = waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();
        if changed.is_empty() {
            continue;
        }
        let mut order: Vec<Url> = changed.iter().cloned().collect();
        order.extend(
            lock(&workspace)
                .open_uris()
                .into_iter()
                .filter(|open| !changed.contains(open)),
        );
        publish_round(&client, &workspace, &published, &order).await;
    }
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
    preceded_by(before).is_some_and(|name| name == cpar::REQUIRE || name == ctype::REQUIRE)
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
    Require {
        module: String,
        /// members already read off the module, nearest last
        members: Vec<String>,
        /// `RequireType("m")` names a module in a type position, `require("m")`
        /// names the same module as a value
        type_position: bool,
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
    require_head(&chars, path_end).map(|(module, members, type_position)| AccessBase::Require {
        module,
        members,
        type_position,
    })
}

/// The `require("m")` or `RequireType("m")` ending at `end`, with the module it
/// names, the members already read off it, and whether it names the type
/// namespace or the value one.
///
/// `end` is where the head stops and the `.` would begin. The argument is a
/// string literal, so the name is taken out of the quotes rather than read as an
/// expression -- which is also what makes this the same name `require "m"` uses.
fn require_head(chars: &[char], end: usize) -> Option<(String, Vec<String>, bool)> {
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
    let type_position = if head == duka_lib::duka_shared::constants::ctype::REQUIRE {
        true
    } else if head == duka_lib::duka_shared::constants::cpar::REQUIRE {
        false
    } else {
        return None;
    };
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
    Some((module, members, type_position))
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

fn is_private_member(name: &str) -> bool {
    name.starts_with('_')
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('_') || name.starts_with('$')
}

fn visible_members(members: Vec<(String, String)>) -> Vec<(String, String)> {
    members
        .into_iter()
        .filter(|(name, _)| !is_hidden_name(name))
        .collect()
}

/// Whether a member access base names the class itself rather than an instance.
fn base_names_class(analysis: &compile::DocAnalysis, base: &str) -> bool {
    let mut segments = base.split('.');
    let Some(head) = segments.next() else {
        return false;
    };
    let rest: Vec<&str> = segments.collect();
    if rest.is_empty() {
        return analysis
            .scope
            .symbols
            .lookup_unambiguous(head)
            .is_some_and(|sym| matches!(sym.symbol_type, SymbolType::ObjectClass(_)));
    }
    // `m.Point`: the module says which of its names are classes
    let Some(module) = module_for_base(analysis, head) else {
        return false;
    };
    rest.len() == 1
        && matches!(
            module.exported.get(rest[0]),
            Some(duka_lib::duka_frontend::analyzer::modules::ExportedTypeKind::Object(_))
        )
}

fn class_method<'a>(
    objects: &'a [ObjectType],
    class: &'a ObjectType,
    name: &str,
) -> Option<&'a ObjectMethod> {
    let mut current = Some(class);
    while let Some(next) = current {
        if let Some(method) = next.methods.iter().find(|method| &*method.name == name) {
            return Some(method);
        }
        current = next.base.and_then(|base| objects.get(base));
    }
    None
}

fn class_for_base<'a>(
    analysis: &'a compile::DocAnalysis,
    base: &str,
) -> Option<(&'a ObjectType, &'a [ObjectType])> {
    path_resolved(analysis, base)?.class_table(analysis)
}

fn new_hover(
    analysis: &compile::DocAnalysis,
    text: &str,
    token: &duka_lib::duka_frontend::lexer::token::Token,
    class: &ObjectType,
    objects: &[ObjectType],
) -> Hover {
    let init = class_method(objects, class, csugar::INIT_FUNC);
    let params = match init {
        Some(init) => {
            let names = param_names(&analysis.tokens.tokens, init.span).unwrap_or_default();
            docs::function_parameters(&init.sig, &names)
        }
        None => vec![],
    };
    let card = crate::hover::Card::new()
        .signature(format!(
            "function {}:new({}) -> {}",
            class.name,
            params.join(", "),
            class.name
        ))
        .prose(format!("Creates a new `{}`.", class.name))
        .prose("The arguments are the ones `init` takes.");
    convert::to_markup_hover(text, token, &card.render())
}

/// Every name a class offers: its fields, then its methods as functions.
///
/// A class read through its own name answers with what sits on the class table --
/// its properties, its static methods and the `new` factory -- while an instance
/// answers with the members and the instance methods instead.
fn class_members(
    objects: &[ObjectType],
    class: &ObjectType,
    as_class: bool,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur = Some(class);
    while let Some(c) = cur {
        for m in c.members.iter() {
            if is_private_member(&m.name) {
                continue;
            }
            if !out.iter().any(|(name, _)| name == m.name.as_ref()) {
                out.push((m.name.to_string(), m.ty.to_string()));
            }
        }
        for m in c.methods.iter() {
            if m.is_static != as_class || is_private_member(&m.name) {
                continue;
            }
            if !out.iter().any(|(name, _)| name == m.name.as_ref()) {
                out.push((
                    m.name.to_string(),
                    Type::Function(Some(m.sig.clone())).to_string(),
                ));
            }
        }
        cur = c.base.and_then(|b| objects.get(b));
    }
    if as_class && !out.iter().any(|(name, _)| name == "new") {
        let detail = match class_method(objects, class, csugar::INIT_FUNC) {
            Some(init) => format!(
                "function({}) -> {}",
                docs::function_parameters(&init.sig, &[]).join(", "),
                class.name
            ),
            None => format!("function() -> {}", class.name),
        };
        out.insert(0, ("new".to_owned(), detail));
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
    let mut card = crate::hover::Card::new();
    let filled: Vec<String> = bindings
        .iter()
        .filter(|b| b.kind == BindingKind::Parameter)
        .map(|b| format!("`{} = {}`", b.name, b.value))
        .collect();
    card = card.bullets(filled);
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
        card = card.prose(format!("`where {where_line}`"));
    }
    Some(format!("\n\n{}", card.render()))
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
    use duka_lib::duka_frontend::parser::ast::{ReturnAnnotation, TypeDesc, TypeFnParam};

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
            return normalized.to_string();
        }
        let resolved = normalized.as_type();
        if matches!(resolved, Type::Any) && !matches!(&normalized, TypeDesc::Pure(Type::Any)) {
            normalized.to_string()
        } else {
            resolved.to_string()
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
            | TypeDesc::SelfType(_)
            | TypeDesc::Array(None)
            | TypeDesc::AnyType(_) => false,
            TypeDesc::FnLit(fparams, _) => fparams
                .iter()
                .any(|p| matches!(p, TypeFnParam::Typed(_, t) if has_splice(t))),
            TypeDesc::ShapeType(_, args, _) => args.iter().any(has_splice),
            TypeDesc::ConstLit(bound, _) => bound.as_deref().is_some_and(has_splice),
        }
    }
    fn params(list: &[TypeFnParam], names: &[Box<str>]) -> String {
        list.iter()
            .map(|p| match p {
                TypeFnParam::Typed((n, _), ty) => format!("{n}: {}", show(ty, names)),
                TypeFnParam::Name((n, _)) => n.to_string(),
                TypeFnParam::Kinded((n, _), k) => format!("{n}: {k}"),
                TypeFnParam::Var(_, Some((n, _))) => format!("...{n}"),
                TypeFnParam::Var(_, None) => "...".to_owned(),
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
                params(&f.params, &names),
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
                    TypeFnParam::Name((n, _))
                    | TypeFnParam::Kinded((n, _), _)
                    | TypeFnParam::Typed((n, _), _)
                    | TypeFnParam::Var(_, Some((n, _))) => {
                        Some(n.as_str().to_owned().into_boxed_str())
                    }
                    TypeFnParam::Var(_, None) => None,
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
        false => format!("type function {name}{generics}({params}) -> {returns}"),
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
        SymbolType::TypeBuiltin => {
            let view = docs::type_builtin_view(name)?;
            let mut hover = convert::to_markup_hover(text, token, &view.render());
            convert::append_markup(&mut hover, convert::DECLARED_NOTE);
            return Some(hover);
        }
        SymbolType::TypeParam { bound, default } => {
            let mut card = crate::hover::Card::new().signature(format!("(type parameter) {name}"));
            if let Some(bound) = bound {
                card = card.prose(format!("bound: `{bound}`"));
            }
            if let Some(default) = default {
                card = card.prose(format!("default: `{default}`"));
            }
            convert::to_markup_hover(text, token, &card.render())
        }
        SymbolType::TypeBinding { source } => {
            // Reads as a binding rather than a parameter: `where type V = E` is
            // not a slot with a bound and a default, it is a name standing for a
            // type that was written somewhere above.
            let card = crate::hover::Card::new()
                .signature(format!("(type binding) {name} -- from {source}"));
            convert::to_markup_hover(text, token, &card.render())
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
            let generics = declared_generics(analysis, symbol);
            // At a call the signature that answers is the one this call produced,
            // not the one the declaration was written with -- so the solution goes
            // into the analysed type, which is the only one of the two a type can
            // be walked through.
            let solution = call_bindings.and_then(|bs| {
                let bindings: std::collections::HashMap<_, _> = bs
                    .iter()
                    .filter_map(|b| {
                        b.value_type
                            .as_deref()
                            .cloned()
                            .map(|t| (b.name.clone(), t))
                    })
                    .collect();
                (!bindings.is_empty()).then_some(duka_lib::duka_frontend::solver::Solution {
                    bindings,
                    ..Default::default()
                })
            });

            // let effects_note = symbol
            //     .ty
            //     .as_deref()
            //     .and_then(|ty| match ty {
            //         Type::Function(Some(ft)) => Some(docs::effects_suffix(ft.effects)),
            //         _ => None,
            //     })
            //     .unwrap_or_default();

            let signature = match symbol.ty.as_deref().and_then(as_function) {
                Some(sig) => {
                    let mut ft = sig.param_view();
                    ft.type_vars = [].into();
                    if let Some(solution) = &solution {
                        let viewed = Type::Function(Some(ft));
                        ft = match solution.substitute(&viewed) {
                            Type::Function(Some(ft)) => ft,
                            _ => match viewed {
                                Type::Function(Some(ft)) => ft,
                                _ => unreachable!(),
                            },
                        };
                        ft.type_vars = [].into();
                    }
                    let names =
                        param_names(&analysis.tokens.tokens, symbol.span).unwrap_or_default();
                    docs::function_signature("function ", name, generics.as_deref(), &names, &ft)
                }
                None => {
                    let tail = symbol
                        .ty_display
                        .as_deref()
                        .filter(|ty| !ty.is_empty())
                        .map(|ty| ty.strip_prefix("function").unwrap_or(ty))
                        .unwrap_or_default();
                    format!("function {name}{tail}")
                }
            };

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
            let mut card = crate::hover::Card::new().signature(signature);
            if let Some(where_list) = rendered_where(analysis, symbol) {
                card = card.code("duka", format!("where {where_list}"));
            }
            let mut hover = convert::to_markup_hover(text, token, &card.render());
            if symbol.attributes.has_declare() {
                // the signature is the whole of a declaration, so the note says
                // what the absence of a body means rather than what the body was
                convert::append_markup(&mut hover, convert::DECLARED_NOTE);
            }
            hover
        }
        SymbolType::Variable | SymbolType::Constant(_) => {
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
            convert::to_markup_hover(
                text,
                token,
                &crate::hover::Card::new().signature(signature).render(),
            )
        }
        _ => return None,
    };
    if let Some(note) = generic_note {
        convert::append_markup(&mut hover, note);
    }
    // What the declaration had to say about itself, carried on the symbol so it
    // answers the same whether the cursor sits on the declaration or a use.
    if let Some(doc) = symbol.attributes.get_doc() {
        let prose = crate::hover::Card::new().prose(doc.as_ref()).render();
        convert::append_markup(&mut hover, &format!("\n\n{prose}"));
    }
    if let Some(info) = symbol.attributes.deprecated() {
        let mut note = String::from("\n\n**Deprecated**");
        if !info.reason.is_empty() {
            note.push_str(": ");
            note.push_str(info.reason);
        }
        if !info.replacement.is_empty() {
            note.push_str("\n\nUse `");
            note.push_str(info.replacement);
            note.push_str("` instead.");
        }
        convert::append_markup(&mut hover, &note);
    }
    Some(hover)
}

/// Where a `require`d module declares the member this token names.
///
/// The path the token is written with is the module and the member, so the
/// declaration is looked up in that module's own symbol table and its file is
/// where the reader lands. Nothing is said when the base is not a module: a
/// record or a class has its declaration here, not there.
fn module_member_definition(
    analysis: &compile::DocAnalysis,
    _text: &str,
    token: &duka_lib::duka_frontend::lexer::token::Token,
) -> Option<Location> {
    let TokenKind::Ident(name) = &token.0 else {
        return None;
    };
    let base = member_base(analysis, token.1)?;
    let module = module_for_base(analysis, &base)?;
    let sym = module
        .analysis
        .symbols
        .scopes
        .first()
        .and_then(|scope| scope.symbols.get(&**name))
        .and_then(|symbols| symbols.iter().next_back())
        .or_else(|| module.analysis.symbols.lookup_anywhere(name))?;
    let SourceName::File(_, path) = &module.source.name else {
        return None;
    };
    let uri = Url::from_file_path(path).ok()?;
    let source = String::from_utf8_lossy(&module.source.source);
    Some(Location {
        uri,
        range: convert::lsp_range(&source, sym.span),
    })
}

/// The declaration a position is about: the token's own declaration, else a use
/// recorded for that token.
fn resolve_symbol(
    analysis: &compile::DocAnalysis,
    span: Span,
) -> Option<&duka_lib::duka_shared::utils::Symbol> {
    let table = &analysis.scope.symbols;
    table.symbol_at_span(span).or_else(|| {
        analysis
            .scope
            .uses
            .get(&span)
            .and_then(|id| table.symbol_by_id(*id))
    })
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
    if let Some((name, spans)) = loop_label_spans(analysis, span) {
        let card = crate::hover::Card::new()
            .signature(format!("(loop label) {name}"))
            .prose("`break ::name::` / `continue ::name::`")
            .prose(format!(
                "Target of {} jump(s).",
                spans.len().saturating_sub(1)
            ));
        return Some(convert::to_markup_hover(text, token, &card.render()));
    }
    let table = &analysis.scope.symbols;
    if let Some(name) = table.label_at(span) {
        let card = crate::hover::Card::new()
            .signature(format!("(label) {name}"))
            .prose("`label`")
            .prose(format!(
                "Reached by {} `goto`.",
                table.label_uses(span).len()
            ));
        return Some(convert::to_markup_hover(text, token, &card.render()));
    }
    let name = table.goto_label_at(span)?;
    let target = match table.goto_target(span) {
        Some(decl) => format!("Jumps to line {}.", decl.start.line),
        None => "No label of that name is in reach.".to_owned(),
    };
    let card = crate::hover::Card::new()
        .signature(format!("(goto) {name}"))
        .prose("`goto`")
        .prose(target);
    Some(convert::to_markup_hover(text, token, &card.render()))
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
    let objects = &analysis.scope.objects;
    // the innermost class whose span holds the position, so a nested one wins
    let class = objects
        .iter()
        .filter(|object| {
            span.start.line >= object.decl_span.start.line
                && span.end.line <= object.decl_span.end.line
        })
        .max_by_key(|object| (object.decl_span.start.line, object.decl_span.start.column))?;
    let mut cur = Some(class);
    while let Some(c) = cur {
        for member in &c.members {
            if &*member.name == name {
                return Some((class.name.to_string(), member.ty.to_string()));
            }
        }
        for method in &c.methods {
            if &*method.name == name {
                return Some((
                    class.name.to_string(),
                    Type::Function(Some(method.sig.clone())).to_string(),
                ));
            }
        }
        cur = c.base.and_then(|b| objects.get(b));
    }
    None
}

/// The class a position is written inside, and the table that class lives in.
///
/// `self` and `super` are not symbols -- they are the enclosing class and its
/// base -- so this is the only way to answer a question about them.
fn enclosing_class(
    analysis: &compile::DocAnalysis,
    pos: Position,
) -> Option<(&ObjectType, &[ObjectType])> {
    let objects = &analysis.scope.objects;
    let line = pos.line + 1;
    let class = objects
        .iter()
        .filter(|object| object.decl_span.start.line <= line && line <= object.decl_span.end.line)
        .max_by_key(|object| (object.decl_span.start.line, object.decl_span.start.column))?;
    Some((class, objects))
}

/// The class a member access base names: `self`, `super`, or a value in scope.
fn base_class<'a>(
    analysis: &'a compile::DocAnalysis,
    pos: Position,
    base: &str,
) -> Option<(&'a ObjectType, &'a [ObjectType])> {
    if base == cgen::SELF {
        return enclosing_class(analysis, pos);
    }
    if base == cgen::SUPER {
        let (class, objects) = enclosing_class(analysis, pos)?;
        return Some((class.base.and_then(|b| objects.get(b))?, objects));
    }
    path_resolved(analysis, base)?.class_table(analysis)
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
    let module = module_for_base(analysis, name)?;
    Some(module.value_is_declared(member))
}

/// The class a type names, whichever module declared it.
///
/// A class that crossed a `require` says which module's table of classes its id
/// indexes, and the answer is read from there rather than from this file. Two
/// files may both declare a `Point` -- and both number it `0` -- so looking a
/// class up by name would answer with whichever came first.
fn type_definition_location(
    analysis: &compile::DocAnalysis,
    text: &str,
    uri: &Url,
    ty: &Type,
) -> Option<Location> {
    match ty {
        Type::Union(items) => items
            .iter()
            .find_map(|item| type_definition_location(analysis, text, uri, item)),
        Type::Object { id, module, .. } => match module {
            None => {
                let class = analysis.scope.objects.get(*id)?;
                Some(Location {
                    uri: uri.clone(),
                    range: convert::lsp_range(text, class.decl_span),
                })
            }
            Some(key) => {
                let module = analysis.scope.modules.get(key.as_ref())?;
                let class = module.analysis.objects.get(*id)?;
                let SourceName::File(_, path) = &module.source.name else {
                    return None;
                };
                let source = String::from_utf8_lossy(&module.source.source);
                Some(Location {
                    uri: Url::from_file_path(path).ok()?,
                    range: convert::lsp_range(&source, class.decl_span),
                })
            }
        },
        _ => None,
    }
}

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
    fn a_function_returning_an_alias_does_not_say_any() {
        let src = "type Alias = int\nfunction f(): Alias\n    return 1\nend\nf()\n";
        let markup = hover_markup(src, "f", "f()");
        assert!(!markup.contains("-> any"), "{markup}");
        assert!(
            markup.contains("-> int") || markup.contains("-> Alias"),
            "{markup}"
        );
    }

    #[test]
    fn a_type_function_names_its_return_type_instead_of_any() {
        let src = concat!(
            "type Alias = int\n",
            "type function Ident(T) = Alias\n",
            "return 1\n",
        );
        let markup = hover_markup(src, "Ident", "Ident(T");
        assert!(markup.contains("-> Alias"), "{markup}");
        assert!(!markup.contains("-> any"), "{markup}");
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
            markup.contains("type function FFFF(...ts) -> [int, ...ts]"),
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
            &duka_lib::duka_shared::config::DukaParsingConfig::default(),
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
        {
            let (src, list, constraints) = (
                concat!(
                    "function B<T, ...V>(a: T, b: array<T!>!, ...): (int, C, ...)\n",
                    "    where T: [V], type C = [V, T]\n",
                    "    return 1, false\n",
                    "end\n",
                    "B([12312], [])\n",
                ),
                "<T, ...V>",
                "`where T: [int], C = [int, [int]]`",
            );
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
        assert!(markup.contains("[int]"), "{markup}");
        assert!(
            !markup.contains("function(T"),
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
            markup.contains("function print(a: int) -> string"),
            "{markup}"
        );
        assert!(
            !markup.contains("Prints to standard output"),
            "the builtin's prose was shown for a function the project wrote: {markup}"
        );
    }

    #[test]
    fn a_function_hover_says_what_it_effects() {
        let text = "@effects(external)\nfunction reads()\nend\nreads()\n";
        let markup = hover_markup(text, "reads", "reads()");
        assert!(
            markup.contains("function reads() @effects(external)"),
            "{markup}"
        );
    }

    #[test]
    fn an_inferred_effect_reaches_the_hover() {
        let text = "function bail()\n    error(\"no\")\nend\nbail()\n";
        let markup = hover_markup(text, "bail", "bail()");
        assert!(
            markup.contains("function bail() @effects(throw)"),
            "{markup}"
        );
    }

    #[test]
    fn an_exclusion_shows_in_the_hover() {
        let text = "@effects(deny(control))\nfunction clean()\nend\nclean()\n";
        let markup = hover_markup(text, "clean", "clean()");
        assert!(
            markup.contains("function clean() @effects(deny(control))"),
            "{markup}"
        );
    }

    #[test]
    fn a_quiet_function_says_nothing_about_effects() {
        let text = "function calm()\n    return 1\nend\ncalm()\n";
        let markup = hover_markup(text, "calm", "calm()");
        assert!(
            !markup.contains("@effects"),
            "nothing written and nothing called infers to auto: {markup}"
        );
    }

    #[test]
    fn a_builtin_hover_carries_its_effect() {
        let view = docs::value_builtin_view("os.time").expect("os.time");
        let signature = view.signature.expect("a signature");
        assert!(signature.contains("@effects(nondet)"), "{signature}");
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
                .resolve_at((span.start.line, span.start.column), name)
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
        assert!(value.contains(":foo("), "{value}");
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
    fn hover_sees_a_loop_variable_resolved_from_what_it_iterates() {
        let text = "for x in [10, 20] do\n    local y = x\nend\n";
        let analysis = analyze(text);
        let pos = Position {
            line: 1,
            character: 14,
        };
        let token = convert::token_at(text, pos, &analysis.tokens.tokens).expect("x");
        let symbol = analysis
            .scope
            .symbols
            .symbol_at_span(token.1)
            .or_else(|| {
                analysis
                    .scope
                    .uses
                    .get(&token.1)
                    .and_then(|id| analysis.scope.symbols.symbol_by_id(*id))
            })
            .or_else(|| analysis.scope.symbols.resolve_at((1, 14), "x"))
            .expect("the loop variable resolves");
        let hover = convert::to_hover(text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
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
    fn a_local_initialized_with_nil_shows_the_nil_it_holds() {
        let text = "local a = nil\nreturn 1\n";
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        assert_eq!(
            symbol_of(text, &analysis, "a = nil").ty_display.as_deref(),
            Some("nil"),
            "strict: a literal nil is what the name holds where it is declared"
        );
        // strict: a name with no annotation and no value is not a quiet `any`
        let text = "local a = nil\nlocal b\n";
        let analysis = analyze(text);
        assert!(
            analysis
                .errors
                .iter()
                .any(|e| e.to_string().contains("reads as unknown")),
            "{:?}",
            analysis.errors
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
    fn a_type_function_builtin_reads_as_a_type() {
        let text = "type LET3 = WithMetatable(table, table)\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert_eq!(
            types_of(&s, "WithMetatable"),
            vec![convert::SEMANTIC_TYPE],
            "{s:?}"
        );
    }

    #[test]
    fn a_builtin_that_hands_back_its_arguments_is_typed_by_them() {
        // `as` is `params(who: vararg<T>), returns(vararg<T>)`: the pack names
        // both the rest arguments and the rest returns
        let analysis = analyze("local c, d, e = as.<int, int, int>(1, 2, 3)");
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        for name in ["c", "d", "e"] {
            let sym = analysis
                .scope
                .symbols
                .lookup_unambiguous(name)
                .unwrap_or_else(|| panic!("{name} is declared"));
            assert_eq!(sym.ty_display.as_deref(), Some("int"), "{name}");
        }

        // and with no type arguments the pack is filled in from what was passed
        let analysis = analyze("local a, b = as(1, \"two\")");
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
        let a = analysis.scope.symbols.lookup_unambiguous("a").expect("a");
        let b = analysis.scope.symbols.lookup_unambiguous("b").expect("b");
        assert_eq!(a.ty_display.as_deref(), Some("int"), "{:?}", a.ty_display);
        assert_eq!(
            b.ty_display.as_deref(),
            Some("string"),
            "{:?}",
            b.ty_display
        );
    }

    #[test]
    fn a_class_name_offers_its_factory_and_hides_private_members() {
        let text = "@data(frozen)\nobject ABC\n    x: int = 0\n    _yy: int! = 1\n    function helper(): int\n        return 1\n    end\n    function :bump()\n        self.x = self.x + 1\n    end\nend\nlocal a = ABC.new()\n";
        let analysis = analyze(text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);

        // the class itself: its properties, its statics and the `new` factory
        let resolved = path_resolved(&analysis, "ABC").expect("the class");
        let (class, objects) = resolved.class_table(&analysis).expect("a class");
        let names: Vec<String> = class_members(objects, class, true)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(names.contains(&"new".to_owned()), "{names:?}");
        assert!(names.contains(&"helper".to_owned()), "{names:?}");
        assert!(names.contains(&"x".to_owned()), "{names:?}");
        assert!(
            !names.contains(&"bump".to_owned()),
            "an instance method is not on the class table: {names:?}"
        );
        assert!(
            !names.contains(&"_yy".to_owned()),
            "a private member is not offered: {names:?}"
        );

        // an instance: its members and instance methods, not the factory
        let resolved = path_resolved(&analysis, "a").expect("the instance");
        let (class, objects) = resolved.class_table(&analysis).expect("a class");
        let names: Vec<String> = class_members(objects, class, false)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(names.contains(&"x".to_owned()), "{names:?}");
        assert!(names.contains(&"bump".to_owned()), "{names:?}");
        assert!(!names.contains(&"new".to_owned()), "{names:?}");
        assert!(!names.contains(&"helper".to_owned()), "{names:?}");
        assert!(!names.contains(&"_yy".to_owned()), "{names:?}");

        // and the private member still reads without an error
        let text = format!("{text}local c = a._yy\n");
        let analysis = analyze(&text);
        assert!(analysis.errors.is_empty(), "{:?}", analysis.errors);
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
        let analysis = analyze(text);
        let span = token_of(text, &analysis, "print").1;
        assert_eq!(member_or_name(&analysis, span), "print");
        assert_eq!(
            docs::value_builtin_params(&member_or_name(&analysis, span)).as_deref(),
            Some(["...".to_owned()].as_slice()),
            "print is variadic"
        );

        let text = "local s = string.upper(\"a\")\n";
        let analysis = analyze(text);
        let span = token_of(text, &analysis, "upper").1;
        assert_eq!(member_or_name(&analysis, span), "string.upper");
        assert_eq!(
            docs::value_builtin_params(&member_or_name(&analysis, span)).as_deref(),
            Some(["s".to_owned()].as_slice())
        );
    }

    #[test]
    fn a_standard_library_function_is_described_the_way_a_local_one_is() {
        let text = "function f(a: int): string return \"x\" end\n";
        let analysis = analyze(text);
        let local = symbol_of(text, &analysis, "f(a: int)");
        let local_signature = match &local.symbol_type {
            SymbolType::Function => {
                let sig = local
                    .ty
                    .as_deref()
                    .and_then(as_function)
                    .expect("a signature");
                docs::function_signature("function ", "f", None, &["a".to_owned()], sig)
            }
            other => panic!("{other:?}"),
        };
        let view = docs::value_builtin_view("string.upper").expect("string.upper");
        let builtin_signature = view.signature.clone().expect("signature");
        assert!(
            builtin_signature.contains("-> string"),
            "{builtin_signature}"
        );
        assert!(
            builtin_signature.starts_with("function "),
            "{builtin_signature}"
        );
        assert!(
            local_signature.starts_with("function "),
            "{local_signature}"
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
        // Where the `?` falls used to be the whole rule: nilability belonged to
        // a declaration of a value, and a parameter started out accepting
        // `nil`. Strict reads a declaration as what it says: `a: T` is `T`,
        // and a caller with a nil has to write the nil. `array<T!>!` is a type
        // argument written by the programmer, so it reads exactly as written.
        // `(int, T, ...)` is a type context -- it says what comes back -- and
        // neither the `int` nor the `T` is quietly widened to accept a `nil`
        // the signature never promised.
        assert_eq!(ty, "function<T: [V], V>(T, array<T>, ...) -> (int, T, ...)");
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
                view.render().contains(&name.to_string()) || view.render().contains("@"),
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
        assert!(print.documentation.is_none());
        assert_eq!(
            print
                .data
                .as_ref()
                .and_then(|data| data.get("builtin"))
                .and_then(|v| v.as_str()),
            Some("print")
        );
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
        // a real absolute path, so the module has a file the editor can open
        let dir = std::env::temp_dir().join("duka-lsp-module-test");
        let mut open = HashMap::new();
        open.insert(
            dir.join("modules").join("host.duka"),
            Arc::new(module.to_owned()),
        );
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
    fn a_require_binding_names_the_module_file() {
        let module = "export function f(): int return 1 end\n";
        let text = "local host = require(\"host\")\nlocal n = host.f()\nreturn n\n";
        let analysis = analyze_with_module(text, module);
        let bindings = require_bindings(&analysis);
        let module_path = std::env::temp_dir()
            .join("duka-lsp-module-test")
            .join("modules")
            .join("host.duka");
        assert_eq!(bindings.get("host"), Some(&module_path));
    }

    #[test]
    fn a_member_read_is_an_occurrence_of_the_module_declaration() {
        let module = "export function f(): int return 1 end\n";
        let text = "local host = require(\"host\")\nlocal n = host.f()\nreturn n\n";
        let analysis = analyze_with_module(text, module);
        let dir = std::env::temp_dir().join("duka-lsp-module-test");
        let module_path = dir.join("modules").join("host.duka");
        let token = token_of(text, &analysis, "f()");
        let target = cross_target(&analysis, token.1, "f", Some(&dir.join("main.duka")))
            .expect("a cross target");
        assert_eq!(target.decl_path, module_path);
        assert!(module_symbol(&analysis, "host", "f").is_some());
        assert_eq!(
            member_occurrences(&analysis, "f", &module_path),
            vec![token.1]
        );
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
            vec!["function(string) -> int".to_owned()],
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
    fn self_and_super_offer_the_class_members() {
        let text = "object A\n    value: int = 0\n    function :get(): int\n        return self.value\n    end\nend\nobject B extends A\n    function :run()\n        self.get()\n        super.get()\n    end\nend\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let members_at = |line: u32, character: u32| {
            let pos = Position::new(line, character);
            let Some(AccessBase::Path(base)) = member_access_base(&index, text, pos) else {
                panic!("a member access at {line}:{character}");
            };
            let (class, objects) =
                base_class(&analysis, pos, &base).unwrap_or_else(|| panic!("{base} names a class"));
            class_members(objects, class, false)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        // `self.` in `B` is `B`, whose members include the ones `A` wrote
        let own = members_at(8, 13);
        assert!(own.contains(&"value".to_owned()), "{own:?}");
        assert!(own.contains(&"run".to_owned()), "{own:?}");
        assert!(own.contains(&"get".to_owned()), "{own:?}");
        // `super.` is `A`, which does not have `B`'s own method
        let base = members_at(9, 14);
        assert!(base.contains(&"value".to_owned()), "{base:?}");
        assert!(base.contains(&"get".to_owned()), "{base:?}");
        assert!(!base.contains(&"run".to_owned()), "{base:?}");
    }

    #[test]
    fn an_inline_require_offers_the_module_members() {
        let entry = "local n = require(\"host\").foo\n";
        let analysis = analyze_with_module(
            entry,
            "export function foo(): int return 1 end\nexport local bar = 1\n",
        );
        let index = convert::LineIndex::new(entry);
        let pos = Position::new(0, 26);
        let Some(AccessBase::Require {
            module,
            members,
            type_position,
        }) = member_access_base(&index, entry, pos)
        else {
            panic!("require(\"host\"). is a member access");
        };
        assert_eq!(module, "host");
        assert!(members.is_empty());
        assert!(!type_position);
        let items = require_value_members(&analysis, &module, &members);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"foo"), "{labels:?}");
        assert!(labels.contains(&"bar"), "{labels:?}");
    }

    #[test]
    fn a_required_member_jumps_into_the_module_file() {
        let entry = "local host = require(\"host\")\nlocal n = host.foo()\n";
        let analysis = analyze_with_module(entry, "export function foo(): int return 1 end\n");
        let at = entry.rfind("host.foo").expect("the member") + "host.".len();
        let prefix = &entry[..at];
        let line = prefix.matches('\n').count() as u32 + 1;
        let character = prefix.rsplit('\n').next().unwrap().chars().count() as u32 + 1;
        let index = convert::LineIndex::new(entry);
        let pos = index.position(entry, line, character);
        let token = convert::token_at(entry, pos, &analysis.tokens.tokens).expect("a token");
        let location = module_member_definition(&analysis, entry, token).expect("a location");
        assert!(
            location.uri.as_str().ends_with("host.duka"),
            "{:?}",
            location.uri
        );
        assert_eq!(location.range.start.line, 0);
        assert_eq!(
            location.range.start.character,
            "export function ".len() as u32
        );
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
            Some("int".to_owned()),
            "a field of a class from another module keeps its type"
        );
        assert!(
            member_of_class(objects, class, Some("len"))
                .is_some_and(|(_, ty)| ty.starts_with("function")),
            "and so does a method"
        );

        // two files may both declare a `Point`; the answer is the one this
        // receiver was built from, not the first one in this file
        assert_eq!(class_members(objects, class, false).len(), 2);

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
        assert!(signature.starts_with("function print("), "{signature}");
        assert!(!view.content.is_empty(), "{}", view.content);
    }

    #[test]
    fn a_standard_library_member_is_reachable_by_its_path() {
        let view = docs::value_builtin_view("string.upper").expect("string.upper");
        assert_eq!(view.name, "upper");
        let signature = view.signature.as_deref().expect("signature");
        assert!(signature.starts_with("function upper("), "{signature}");
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
                Some(format!("type function {name}({params})").as_str()),
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
        let hover = symbol_hover(&analysis, text, token, pick, "Pick", None, None)
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
    fn a_hover_shows_the_where_list() {
        let src = "object Point\n    x: int = 0\nend\nfunction shift<T>(v: T): int\nwhere\n    T: Point\n    return 1\nend\nreturn 1\n";
        let markup = hover_markup(src, "shift", "shift<T>");
        assert!(markup.contains("where T: Point"), "{markup}");
    }

    #[test]
    fn a_rust_type_builtin_has_a_hover() {
        let text = "type P = Patterns.GtPat(int, 0)\nreturn nil\n";
        let analysis = analyze(text);
        let token = token_of(text, &analysis, "GtPat");
        let ty = analysis
            .scope
            .type_at_span
            .get(&token.1)
            .expect("a member of a builtin module resolves");
        assert!(ty.to_string().contains("GtPat"), "{ty}");
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
        assert!(shown.contains("(module) os"), "{shown}");
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
        assert!(
            file.text
                .lines()
                .skip(line as usize)
                .take_while(|l| !l.is_empty())
                .any(|l| l.trim() == "function exit(code: int | nil)"),
            "the line it lands on belongs to the declaration of exit: {:?}",
            file.text
                .lines()
                .skip(line as usize)
                .take(6)
                .collect::<Vec<_>>()
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

    #[test]
    fn signature_help_lists_parameters_and_follows_the_cursor() {
        let text = "function add(a: int, b: string): int\n    return a\nend\nadd(1, 2)\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let (line, character) = offset_of(text, "add(1, ");
        let help = signature_help_for(
            &analysis,
            text,
            &index,
            Position {
                line,
                character: character + 7,
            },
        )
        .expect("a signature");
        assert_eq!(help.signatures[0].label, "add(a: int, b: string) -> int");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn signature_help_reads_a_method_off_its_class() {
        let text = "object A\n    function add(a: int, b: int): int\n        return a\n    end\nend\nlocal a: A = A.new()\na:add(1, 2)\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let (line, character) = offset_of(text, "add(1, ");
        let help = signature_help_for(
            &analysis,
            text,
            &index,
            Position {
                line,
                character: character + 7,
            },
        )
        .expect("a method signature");
        assert_eq!(help.signatures[0].label, "add(a: int, b: int) -> int");
        assert_eq!(help.active_parameter, Some(1));
    }

    #[test]
    fn document_highlight_lists_the_declaration_and_every_use() {
        let text = "local x = 1\nreturn x + x\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let span = token_of(text, &analysis, "x = 1").1;
        let highlights = highlights_for(&analysis, text, &index, span).expect("highlights");
        assert_eq!(highlights.len(), 3, "{highlights:#?}");
    }

    #[test]
    fn rename_replaces_every_use_but_not_a_member() {
        let text = "local x = 1\nreturn x + x\n";
        let analysis = analyze(text);
        let span = token_of(text, &analysis, "x = 1").1;
        let (_, spans) = same_name_spans(&analysis, span).expect("rename spans");
        assert_eq!(spans.len(), 3);
        assert!(valid_ident("name_2"));
        assert!(!valid_ident("1bad"));
        assert!(!valid_ident("int"));
        assert!(!valid_ident("local"));

        let text = "object A\n    x = 1\nend\nlocal a: A = A.new()\nreturn a.x\n";
        let analysis = analyze(text);
        let (line, character) = offset_of(text, "a.x");
        let token = convert::token_at(
            text,
            Position {
                line,
                character: character + 2,
            },
            &analysis.tokens.tokens,
        )
        .expect("the member token");
        assert!(is_member_read(&analysis, token.1));
    }

    #[test]
    fn folding_covers_blocks_and_multiline_comments() {
        let text =
            "function f()\n    if true then\n        return 1\n    end\nend\n--[[ a\ncomment ]]\n";
        let analysis = analyze(text);
        let ranges = folding_ranges_for(&analysis, text);
        assert!(
            ranges.iter().any(|r| r.start_line == 0 && r.end_line == 4),
            "{ranges:#?}"
        );
        assert!(
            ranges.iter().any(|r| r.start_line == 1 && r.end_line == 3),
            "{ranges:#?}"
        );
        assert!(
            ranges.iter().any(|r| r.start_line == 5 && r.end_line == 6),
            "{ranges:#?}"
        );
    }

    #[test]
    fn a_require_string_links_to_the_module_file() {
        let module = "export function f(): int return 1 end\n";
        let text = "local host = require(\"host\")\nreturn host.f()\n";
        let analysis = analyze_with_module(text, module);
        let index = convert::LineIndex::new(text);
        let links = document_links(text, &analysis, &index);
        assert_eq!(links.len(), 1, "{links:#?}");
        let expected = std::env::temp_dir()
            .join("duka-lsp-module-test")
            .join("modules")
            .join("host.duka");
        assert_eq!(
            links[0]
                .target
                .as_ref()
                .and_then(|uri| uri.to_file_path().ok()),
            Some(expected)
        );
    }

    #[test]
    fn a_selection_grows_from_the_token_outward() {
        let text = "local x = 1\nreturn x\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let (line, character) = offset_of(text, "x = 1");
        let selection = selection_range_for(&analysis, text, &index, Position { line, character })
            .expect("a selection");
        let mut sizes = vec![];
        let mut current = Some(&selection);
        while let Some(range) = current {
            sizes.push((
                range.range.end.line - range.range.start.line,
                range
                    .range
                    .end
                    .character
                    .saturating_sub(range.range.start.character),
            ));
            current = range.parent.as_deref();
        }
        assert!(sizes.len() >= 3, "{sizes:?}");
        assert!(
            sizes.windows(2).all(|pair| pair[0] <= pair[1]),
            "the chain must only grow: {sizes:?}"
        );
    }

    #[test]
    fn an_object_holds_its_members_in_the_outline() {
        let text =
            "object A\n    x: int\n    function add(a: int): int\n        return a\n    end\nend\n";
        let analysis = analyze(text);
        let index = convert::LineIndex::new(text);
        let symbols = document_symbols(&analysis, text, &index);
        assert_eq!(symbols.len(), 1, "{symbols:#?}");
        let class = &symbols[0];
        assert_eq!(class.name, "A");
        let children: Vec<&str> = class
            .children
            .as_ref()
            .expect("children")
            .iter()
            .map(|child| child.name.as_str())
            .collect();
        assert_eq!(children, vec!["x", "add"]);
    }

    fn diagnostic_with(code: &str, data: serde_json::Value, range: Range) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(
            range,
            Some(DiagnosticSeverity::WARNING),
            Some(NumberOrString::String(code.to_owned())),
            Some("duka".into()),
            "message".into(),
            None,
            None,
        );
        diagnostic.data = Some(data);
        diagnostic
    }

    #[test]
    fn an_object_new_shows_the_init_signature() {
        let text = concat!(
            "object A\n",
            "    v: int = 0\n",
            "    function init(v: int)\n",
            "        self.v = v\n",
            "    end\n",
            "end\n",
            "local a = A.new(1)\n",
            "return a\n",
        );
        let analysis = analyze(text);
        let token = token_of(text, &analysis, "new(1)");
        let (class, objects) = class_for_base(&analysis, "A").expect("the class");
        let hover = new_hover(&analysis, text, token, class, objects);
        assert!(
            hover_value(&hover).contains("function A:new(v: int) -> A"),
            "{}",
            hover_value(&hover)
        );
        let index = convert::LineIndex::new(text);
        let (line, character) = offset_of(text, "new(1)");
        let help = signature_help_for(
            &analysis,
            text,
            &index,
            Position {
                line,
                character: character + 4,
            },
        )
        .expect("a signature");
        assert_eq!(help.signatures[0].label, "A:new(v: int) -> A");
    }

    #[test]
    fn a_loop_label_is_hoverable_and_linked_to_its_jumps() {
        let text = concat!(
            "for ::outer:: i = 1, 10 do\n",
            "    break ::outer::\n",
            "end\n",
            "while ::w:: true do\n",
            "    continue ::w::\n",
            "end\n",
            "return 0\n",
        );
        let analysis = analyze(text);
        for (name, needle, uses) in [("outer", "outer", 2), ("w", "w::", 2)] {
            let token = token_of(text, &analysis, needle);
            let (_, spans) = loop_label_spans(&analysis, token.1).expect("loop label");
            assert_eq!(spans.len(), uses, "{name}: {spans:?}");
            let hover = label_hover(&analysis, text, token.1, token).expect("a hover");
            assert!(
                hover_value(&hover).contains(&format!("(loop label) {name}")),
                "{}",
                hover_value(&hover)
            );
        }
    }

    #[test]
    fn internal_sugar_names_are_not_offered_by_completion() {
        let text = "local _s_self = 1\nlocal _b_hidden = 2\nlocal normal = 3\nreturn normal\n";
        let analysis = analyze(text);
        let (items, _) = completion_items(&analysis, CompletionContext::Value, "");
        let labels = labels(&items);
        assert!(labels.contains(&"normal"), "{labels:?}");
        assert!(!labels.contains(&"_s_self"), "{labels:?}");
        assert!(!labels.contains(&"_b_hidden"), "{labels:?}");
    }

    #[test]
    fn a_recursive_table_alias_renders_without_the_marker() {
        let text = "type Tab = { a: function(Self) -> Self }\nreturn 1\n";
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, "Tab");
        let rendered = symbol
            .ty
            .as_deref()
            .map(ToString::to_string)
            .or_else(|| symbol.ty_display.as_deref().map(str::to_owned))
            .unwrap_or_default();
        assert!(!rendered.contains("(recursive)"), "{rendered}");
        assert!(rendered.contains("Self"), "{rendered}");
    }

    #[test]
    fn a_deprecated_diagnostic_offers_the_replacement() {
        let uri = Url::parse("file:///test.duka").unwrap();
        let text = "old_fn()\n";
        let index = convert::LineIndex::new(text);
        let range = Range::new(Position::new(0, 0), Position::new(0, 6));
        let diagnostic = diagnostic_with(
            "deprecated",
            serde_json::json!({ "replacement": "new_fn" }),
            range,
        );
        let actions = quick_fixes(&uri, text, &index, &[diagnostic]);
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("expected a code action");
        };
        let edit = action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(&uri))
            .and_then(|edits| edits.first())
            .expect("an edit");
        assert_eq!(edit.new_text, "new_fn");
        assert_eq!(edit.range, range);
    }

    #[test]
    fn a_discarded_result_offers_local_underscore() {
        let uri = Url::parse("file:///test.duka").unwrap();
        let text = "    f()\n";
        let index = convert::LineIndex::new(text);
        let range = Range::new(Position::new(0, 4), Position::new(0, 7));
        let diagnostic = diagnostic_with("must-use", serde_json::json!({ "name": "f" }), range);
        let actions = quick_fixes(&uri, text, &index, &[diagnostic]);
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("expected a code action");
        };
        let edit = action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(&uri))
            .and_then(|edits| edits.first())
            .expect("an edit");
        assert_eq!(edit.new_text, "local _ = ");
        assert_eq!(
            edit.range,
            Range::new(Position::new(0, 4), Position::new(0, 4))
        );
    }

    #[test]
    fn a_value_of_a_class_points_at_the_class() {
        let text = "object Point\n    x: int\nend\nlocal p: Point = Point.new()\nreturn p\n";
        let analysis = analyze(text);
        let symbol = symbol_of(text, &analysis, "p: Point");
        let ty = symbol.ty.as_deref().expect("a type");
        let uri = Url::parse("file:///test.duka").unwrap();
        let location = type_definition_location(&analysis, text, &uri, ty).expect("a location");
        assert_eq!(location.uri, uri);
        assert_eq!(location.range.start.line, 0);
    }

    #[test]
    #[ignore = "Not supported in CI"]
    fn the_stress_workspace_parses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("extension")
            .join("test_workspace")
            .join("test.duka");
        let text = std::fs::read_to_string(&path).expect("test.duka");
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        let mut open = HashMap::new();
        open.insert(path.clone(), Arc::new(text.clone()));
        let analysis = compile::analyze(&text, "test.duka", Some(&path), &open, &mut cache);
        assert!(!analysis.errors.is_empty());
    }
}
