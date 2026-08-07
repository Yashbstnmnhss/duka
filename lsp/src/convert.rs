//! Conversions from Duka compiler types to LSP types.

use duka_frontend::lexer::token::{Token, TokenKind};
use duka_shared::errors::{DukaSpannedError, Span};
use duka_shared::utils::{SymbolTable, SymbolType};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Hover, HoverContents, Location,
    MarkupContent, MarkupKind, Position, Range, SemanticToken, Url,
};

pub const SEMANTIC_METHOD: u32 = 0;
pub const SEMANTIC_VARIABLE: u32 = 1;
pub const SEMANTIC_CONSTANT: u32 = 2;
pub const SEMANTIC_MACRO: u32 = 3;

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

pub fn token_at<'a>(text: &str, pos: Position, tokens: &'a [Token]) -> Option<&'a Token> {
    tokens.iter().find(|(_, span)| {
        let range = lsp_range(text, *span);
        pos >= range.start && pos < range.end
    })
}

/// 生成 hover 内容
pub fn to_hover(text: &str, symbol: &Token, ty: Option<&str>) -> Hover {
    let (kind, span) = symbol;
    let name = match kind {
        TokenKind::Ident(name) => name.as_str(),
        _ => "symbol",
    };
    let ty = ty.unwrap_or("unknown");
    let contents = match kind {
        TokenKind::Ident(_) => MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**{}**\n\n```duka\n{ty}\n```", name),
        },
        _ => MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("`{}`", kind),
        },
    };
    Hover {
        contents: HoverContents::Markup(contents),
        range: Some(lsp_range(text, *span)),
    }
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

fn ident_semantic_type(
    table: &SymbolTable,
    kind: &TokenKind,
    next: Option<&TokenKind>,
) -> Option<u32> {
    let TokenKind::Ident(name) = kind else {
        return None;
    };
    if matches!(next, Some(TokenKind::Bang)) {
        return Some(SEMANTIC_MACRO);
    }
    match table.lookup_named(name.as_str()).map(|s| &s.symbol_type) {
        Some(SymbolType::Function) => Some(SEMANTIC_METHOD),
        Some(SymbolType::Constant(_)) => Some(SEMANTIC_CONSTANT),
        _ => Some(SEMANTIC_VARIABLE),
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
    table: &SymbolTable,
) -> Vec<SemanticToken> {
    let mut data = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;
    for i in 0..tokens.len() {
        let (kind, span) = &tokens[i];
        if kind.is_terminator() {
            continue;
        }
        let next = tokens.get(i + 1).map(|(k, _)| k);
        let Some(token_type) = ident_semantic_type(table, kind, next) else {
            continue;
        };
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
            token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = start.line;
        prev_char = start.character;
    }
    data
}
