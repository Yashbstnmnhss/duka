//! The Duka language server backend.

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::{compile, convert};

pub struct Backend {
    client: Client,
    docs: Mutex<HashMap<Url, String>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            docs: Mutex::new(HashMap::new()),
        }
    }

    fn doc(&self, uri: &Url) -> Option<String> {
        self.docs.lock().ok().and_then(|d| d.get(uri).cloned())
    }

    async fn publish(&self, uri: &Url) {
        let Some(text) = self.doc(uri) else {
            return;
        };
        let analysis = compile::analyze(&text, uri.as_str());
        let diagnostics: Vec<Diagnostic> = analysis
            .errors
            .iter()
            .map(|e| convert::to_diagnostic(&text, uri, e))
            .collect();
        let _ = self
            .client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
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
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::KEYWORD,
                                    SemanticTokenType::STRING,
                                    SemanticTokenType::NUMBER,
                                    SemanticTokenType::OPERATOR,
                                    SemanticTokenType::VARIABLE,
                                    SemanticTokenType::new("punctuation"),
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

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if let Some(mut docs) = self.docs.lock().ok() {
            docs.insert(params.text_document.uri.clone(), params.text_document.text);
        }
        self.publish(&params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.last() {
            if let Some(mut docs) = self.docs.lock().ok() {
                docs.insert(uri.clone(), change.text.clone());
            }
        }
        self.publish(&uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.publish(&params.text_document.uri).await;
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some(text) = self.doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let analysis = compile::analyze(&text, params.text_document.uri.as_str());
        let data = convert::semantic_tokens(&text, &analysis.tokens.tokens);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
