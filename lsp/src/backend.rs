//! The Duka language server backend.

use std::collections::HashMap;
use std::sync::Mutex;

use duka_frontend::lexer::token::TokenKind;
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
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::FUNCTION,
                                    SemanticTokenType::VARIABLE,
                                    SemanticTokenType::new("constant"),
                                    SemanticTokenType::MACRO,
                                    SemanticTokenType::TYPE,
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
        let data =
            convert::semantic_tokens(&text, &analysis.tokens.tokens, &analysis.scope.symbols);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let text = match self.doc(uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let analysis = compile::analyze(&text, uri.as_str());
        let table = &analysis.scope.symbols;

        let Some(token) = convert::token_at(&text, pos, &analysis.tokens.tokens) else {
            return Ok(None);
        };

        let (kind, span) = token;
        if !matches!(kind, TokenKind::Ident(_)) {
            return Ok(None);
        }

        let ty = table.symbol_at_span(*span);
        Ok(Some(convert::to_hover(&text, token, ty)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let p = params.text_document_position_params;
        let uri = &p.text_document.uri;
        let pos = p.position;

        let Some(text) = self.doc(uri) else {
            return Ok(None);
        };

        let analysis = compile::analyze(&text, uri.as_str());
        let Some(token) = convert::token_at(&text, pos, &analysis.tokens.tokens) else {
            return Ok(None);
        };
        if !matches!(token.0, TokenKind::Ident(_)) {
            return Ok(None);
        }
        for link in &analysis.scope.links {
            if link.name_span == token.1 {
                return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                    uri: uri.clone(),
                    range: convert::lsp_range(&text, link.decl_span),
                })));
            }
        }
        Ok(None)
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(text: &str) -> compile::DocAnalysis {
        compile::analyze(text, "test.duka")
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
}
