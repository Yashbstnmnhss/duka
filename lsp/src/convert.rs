//! Conversions from Duka compiler types to LSP types.

use std::collections::HashMap;

use duka_frontend::{
    analyzer::objects::{ObjectMethod, ObjectType},
    lexer::token::{Token, TokenKind},
};
use duka_shared::{
    docs::Doc,
    dtype::Type,
    errors::{DukaSpannedError, Span},
    utils::{Symbol, SymbolTable, SymbolType},
};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Hover, HoverContents, Location,
    MarkupContent, MarkupKind, Position, Range, SemanticToken, Url,
};

use crate::roles::{is_metamethod, Role};

pub const SEMANTIC_FUNCTION: u32 = 0;
pub const SEMANTIC_VARIABLE: u32 = 1;
pub const SEMANTIC_KEYWORD: u32 = 2;
pub const SEMANTIC_MACRO: u32 = 3;
pub const SEMANTIC_TYPE: u32 = 4;
pub const SEMANTIC_ATTRIBUTE: u32 = 5;
pub const SEMANTIC_PROPERTY: u32 = 6;
pub const SEMANTIC_METAMETHOD: u32 = 7;

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

pub fn to_doc_hover(text: &str, token: &Token, doc: &Doc) -> Hover {
    let (_, span) = token;
    let mut value = format!("```duka\n{}\n```\n", doc.title);
    if !doc.content.is_empty() {
        value.push_str(doc.content);
        value.push('\n');
    }
    if let Some(example) = doc.example {
        value.push_str("\n```duka\n");
        value.push_str(example);
        value.push_str("\n```\n");
    }
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lsp_range(text, *span)),
    }
}

pub fn to_hover(text: &str, token: &Token, symbol: Option<&Symbol>) -> Hover {
    let (kind, span) = token;
    let name = match kind {
        TokenKind::Ident(name) => name.as_str(),
        t if t.is_keyword() => t.name(),
        _ => "<symbol>",
    };
    let ty = symbol.map(|i| i.ty.as_deref()).flatten();
    let is_global = symbol.map(|i| i.is_global).unwrap_or(false);
    let contents = match kind {
        TokenKind::Ident(_) => MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "```duka\n{}\n```",
                match &symbol.map(|i| &i.symbol_type) {
                    Some(SymbolType::Function) => match ty {
                        Some(ty) if !ty.is_empty() => format!("function {}: {}", name, ty),
                        _ => format!("function {}", name),
                    },
                    Some(SymbolType::TypeAlias(_)) => match ty {
                        Some(ty) if !ty.is_empty() && ty != "any" =>
                            format!("type {} = {}", name, ty),
                        _ => format!("type {}", name),
                    },
                    Some(SymbolType::TypeFunction(_)) => match ty {
                        Some(ty) if !ty.is_empty() => format!("type function {}: {}", name, ty),
                        _ => format!("type function {}", name),
                    },
                    Some(SymbolType::InlineTypeFunction(_)) => format!("type function {}", name),
                    Some(SymbolType::Constant(cv)) => format!("const {} = {}", name, cv),
                    Some(SymbolType::ObjectClass(_)) => format!("object {}", name),
                    _ => match ty {
                        Some(ty) if !ty.is_empty() => format!(
                            "{} {}: {}",
                            if is_global { "global" } else { "local" },
                            name,
                            ty
                        ),
                        _ => format!("{} {}", if is_global { "global" } else { "local" }, name),
                    },
                }
            ),
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

pub fn to_method_hover(
    text: &str,
    token: &Token,
    object: &ObjectType,
    method: &ObjectMethod,
) -> Hover {
    let (_, span) = token;
    let type_name = object.name.clone();
    let is_static = method.is_static;
    let name = method.name.clone();
    let sig = Type::Function(Some(method.sig.clone())).to_string();
    let contents = MarkupContent {
        kind: MarkupKind::Markdown,
        value: format!(
            "```duka\n(method) {}{}{} {}\n```",
            type_name,
            if is_static { "." } else { ":" },
            name,
            sig
        ),
    };
    Hover {
        range: Some(lsp_range(text, *span)),
        contents: HoverContents::Markup(contents),
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
    prev: Option<&TokenKind>,
    next: Option<&TokenKind>,
) -> Option<u32> {
    if let TokenKind::Dots = kind {
        return Some(SEMANTIC_MACRO);
    }

    let TokenKind::Ident(name) = kind else {
        return None;
    };
    if Type::from_keyword(name).is_some() {
        return Some(SEMANTIC_TYPE);
    }
    if matches!(prev, Some(TokenKind::At)) {
        return Some(SEMANTIC_ATTRIBUTE);
    }
    if matches!(next, Some(TokenKind::Bang)) {
        return Some(SEMANTIC_MACRO);
    }
    if matches!(prev, Some(TokenKind::Colon | TokenKind::Arrow))
        && Type::from_keyword(name).is_none()
    {
        return Some(SEMANTIC_TYPE);
    }
    match table.lookup_named(name.as_str()).map(|s| &s.symbol_type) {
        Some(SymbolType::Function) => Some(SEMANTIC_FUNCTION),
        Some(SymbolType::Constant(_)) => Some(SEMANTIC_KEYWORD),
        Some(SymbolType::ObjectClass(_)) => Some(SEMANTIC_TYPE),
        Some(SymbolType::TypeAlias(_))
        | Some(SymbolType::TypeFunction(_))
        | Some(SymbolType::InlineTypeFunction(_)) => Some(SEMANTIC_TYPE),
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
    roles: &HashMap<Span, Role>,
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
        let prev = if i == 0 {
            None
        } else {
            tokens.get(i - 1).map(|(k, _)| k)
        };
        let token_type = match kind {
            TokenKind::Ident(name) => {
                if is_metamethod(name) {
                    Some(SEMANTIC_METAMETHOD)
                } else {
                    match roles.get(span) {
                        Some(Role::MethodCall) => Some(SEMANTIC_FUNCTION),
                        Some(Role::FieldAccess) => Some(SEMANTIC_PROPERTY),
                        None => ident_semantic_type(table, kind, prev, next),
                    }
                }
            }
            _ => None,
        };
        let Some(token_type) = token_type else {
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
