//! The Duka language server backend.

use std::collections::HashMap;
use std::sync::Mutex;

use duka_frontend::lexer::token::TokenKind;
use duka_shared::{
    docs::{attr_doc, keyword_doc, type_doc},
    dtype::Type,
};
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
                                    SemanticTokenType::KEYWORD,
                                    SemanticTokenType::MACRO,
                                    SemanticTokenType::TYPE,
                                    SemanticTokenType::KEYWORD,
                                    SemanticTokenType::PROPERTY,
                                    SemanticTokenType::EVENT,
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

        let Some(idx) = analysis.tokens.tokens.iter().position(|t| {
            let range = convert::lsp_range(&text, t.1);
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
            return Ok(None);
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
        let table = &analysis.scope.symbols;
        let sym = table.symbol_at_span(token.1).or_else(|| {
            analysis
                .scope
                .uses
                .get(&token.1)
                .and_then(|id| table.symbol_by_id(*id))
        });
        if let Some(sym) = sym {
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: convert::lsp_range(&text, sym.span),
            })));
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
        let hover = convert::to_method_hover(&text, token, object, method);
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
        let hover = convert::to_method_hover(&text, token, object, method);
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
        let hover = convert::to_hover(&text, token, Some(symbol));
        let value = match hover.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!("expected markup"),
        };
        assert!(value.contains("local"), "{value}");
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
        let hover = convert::to_hover(&text, token, Some(symbol));
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
        let mut out = Vec::new();
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
        assert!(types_of(&s, "A")
            .iter()
            .all(|t| *t == convert::SEMANTIC_TYPE));
        assert!(types_of(&s, "a")
            .iter()
            .all(|t| *t == convert::SEMANTIC_VARIABLE));
        assert!(types_of(&s, "b")
            .iter()
            .all(|t| *t == convert::SEMANTIC_VARIABLE));
        assert!(types_of(&s, "int")
            .iter()
            .all(|t| *t == convert::SEMANTIC_TYPE));
        assert!(types_of(&s, "new")
            .iter()
            .all(|t| *t == convert::SEMANTIC_FUNCTION));
    }

    #[test]
    fn semantic_keyword_constant_tokens() {
        let text = "local c: bool = true\nif false then\n    print(nil)\nend\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(types_of(&s, "true")
            .iter()
            .all(|t| *t == convert::SEMANTIC_CONSTANT));
        assert!(types_of(&s, "false")
            .iter()
            .all(|t| *t == convert::SEMANTIC_CONSTANT));
        assert!(types_of(&s, "nil")
            .iter()
            .all(|t| *t == convert::SEMANTIC_CONSTANT));
        for kw in ["local", "if", "then", "end"] {
            assert!(
                types_of(&s, kw)
                    .iter()
                    .all(|t| *t == convert::SEMANTIC_KEYWORD),
                "{kw} should be keyword"
            );
        }
        assert!(types_of(&s, "bool")
            .iter()
            .all(|t| *t == convert::SEMANTIC_TYPE));
    }

    #[test]
    fn semantic_metamethod_tokens() {
        let text = "local mt = { __index = function(k) return k * 2 end }\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(types_of(&s, "__index")
            .iter()
            .all(|t| *t == convert::SEMANTIC_METAMETHOD));
        assert!(types_of(&s, "function")
            .iter()
            .all(|t| *t == convert::SEMANTIC_KEYWORD));
    }

    #[test]
    fn semantic_property_tokens() {
        let text = "print(a.b)\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(types_of(&s, "b")
            .iter()
            .all(|t| *t == convert::SEMANTIC_PROPERTY));
    }

    #[test]
    fn semantic_method_chain_tokens() {
        let text = "a.b():c()\n";
        let analysis = analyze(text);
        let s = semantics(text, &analysis);
        assert!(types_of(&s, "b")
            .iter()
            .all(|t| *t == convert::SEMANTIC_FUNCTION));
        assert!(types_of(&s, "c")
            .iter()
            .all(|t| *t == convert::SEMANTIC_FUNCTION));
    }

    #[test]
    fn keyword_doc_hover_is_available() {
        let doc = keyword_doc("if").expect("if doc");
        assert_eq!(doc.title, "If");
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
        assert!(value.contains("If"), "{value}");
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
