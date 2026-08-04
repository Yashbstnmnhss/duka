//! Conversions from Duka compiler types to LSP types.

use duka_frontend::lexer::token::TokenKind;
use duka_shared::errors::{DukaSpannedError, Span};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range,
    SemanticToken, Url,
};

pub const SEMANTIC_KEYWORD: u32 = 0;
pub const SEMANTIC_STRING: u32 = 1;
pub const SEMANTIC_NUMBER: u32 = 2;
pub const SEMANTIC_OPERATOR: u32 = 3;
pub const SEMANTIC_VARIABLE: u32 = 4;
pub const SEMANTIC_PUNCTUATION: u32 = 5;

pub fn lsp_position(text: &str, line: u32, column: u32) -> Position {
    let line_idx = line.saturating_sub(1) as usize;
    let line_text = text.lines().nth(line_idx).unwrap_or("");
    let col_chars = column.saturating_sub(1) as usize;
    let mut character = 0u32;
    for c in line_text.chars().take(col_chars) {
        character += c.len_utf16() as u32;
    }
    Position {
        line: line_idx as u32,
        character,
    }
}

pub fn lsp_range(text: &str, span: Span) -> Range {
    Range::new(
        lsp_position(text, span.start.line, span.start.column),
        lsp_position(text, span.end.line, span.end.column),
    )
}

pub fn to_diagnostic(text: &str, uri: &Url, err: &DukaSpannedError) -> Diagnostic {
    let related = if err.related.is_empty() {
        None
    } else {
        Some(
            err.related
                .iter()
                .map(|(label, span)| DiagnosticRelatedInformation {
                    location: Location {
                        uri: uri.clone(),
                        range: lsp_range(text, *span),
                    },
                    message: label.to_string(),
                })
                .collect(),
        )
    };

    Diagnostic::new(
        lsp_range(text, err.span),
        Some(DiagnosticSeverity::ERROR),
        None,
        Some("duka".into()),
        err.kind.to_string(),
        related,
        None,
    )
}

pub fn semantic_token_type(kind: &TokenKind) -> u32 {
    if kind.is_keyword() {
        SEMANTIC_KEYWORD
    } else if kind.is_binop() || kind.is_unop() || kind.is_logic_binop() || kind.is_compare() {
        SEMANTIC_OPERATOR
    } else {
        match kind {
            TokenKind::Ident(_) => SEMANTIC_VARIABLE,
            TokenKind::String(_) => SEMANTIC_STRING,
            TokenKind::Int(_) | TokenKind::Float(_) => SEMANTIC_NUMBER,
            _ => SEMANTIC_PUNCTUATION,
        }
    }
}

fn utf16_len(text: &str, span: Span) -> u32 {
    let line_idx = span.start.line.saturating_sub(1) as usize;
    let line_text = text.lines().nth(line_idx).unwrap_or("");
    let from = span.start.column.saturating_sub(1) as usize;
    let to = span.end.column.saturating_sub(1) as usize;
    line_text
        .chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .map(|c| c.len_utf16() as u32)
        .sum()
}

pub fn semantic_tokens(
    text: &str,
    tokens: &[duka_frontend::lexer::token::Token],
) -> Vec<SemanticToken> {
    let mut data = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;
    for (kind, span) in tokens {
        if kind.is_terminator() {
            continue;
        }
        let start = lsp_position(text, span.start.line, span.start.column);
        let delta_line = start.line - prev_line;
        let delta_start = if delta_line == 0 {
            start.character - prev_char
        } else {
            start.character
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: utf16_len(text, *span),
            token_type: semantic_token_type(kind),
            token_modifiers_bitset: 0,
        });
        prev_line = start.line;
        prev_char = start.character;
    }
    data
}
