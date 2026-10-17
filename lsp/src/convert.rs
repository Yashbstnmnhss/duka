//! Conversions from Duka compiler types to LSP types.

use std::collections::{HashMap, HashSet};

use duka_lib::duka_frontend::{
    analyzer::objects::{ObjectMethod, ObjectType},
    lexer::token::{Token, TokenKind},
};
use duka_lib::duka_shared::{
    dtype::Type,
    errors::{DukaDiagnose, Severity, Span},
    utils::{Symbol, SymbolTable, SymbolType},
    value::ConstValue,
};
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Hover, HoverContents, Location,
    MarkupContent, MarkupKind, Position, Range, SemanticToken, Url,
};

use crate::roles::{Role, is_metamethod};

pub const SEMANTIC_FUNCTION: u32 = 0;
pub const SEMANTIC_VARIABLE: u32 = 1;
pub const SEMANTIC_KEYWORD: u32 = 2;
pub const SEMANTIC_MACRO: u32 = 3;
pub const SEMANTIC_TYPE: u32 = 4;
pub const SEMANTIC_ATTRIBUTE: u32 = 5;
pub const SEMANTIC_PROPERTY: u32 = 6;
pub const SEMANTIC_METAMETHOD: u32 = 7;
/// `...`, both the rest parameter and the spread in a call
pub const SEMANTIC_VARARG: u32 = 8;

/// Byte offset of the first character of every line. `Span` columns are
/// counted in characters while LSP wants UTF-16 code units, so every span
/// still walks its own line, but finding the line has to stop being a scan
/// from the start of the file: the semantic token pass asks once per token and
/// the naive version made that quadratic in file size.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    pub fn line_text<'a>(&self, text: &'a str, line_idx: usize) -> &'a str {
        let Some(&start) = self.starts.get(line_idx) else {
            return "";
        };
        let end = self
            .starts
            .get(line_idx + 1)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(text.len());
        let slice = text.get(start..end).unwrap_or("");
        slice.strip_suffix('\r').unwrap_or(slice)
    }

    pub fn position(&self, text: &str, line: u32, column: u32) -> Position {
        let line_idx = line.saturating_sub(1) as usize;
        let line_text = self.line_text(text, line_idx);
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

    pub fn range(&self, text: &str, span: Span) -> Range {
        Range::new(
            self.position(text, span.start.line, span.start.column),
            self.position(text, span.end.line, span.end.column),
        )
    }

    /// The source text a range covers, used to echo a literal back verbatim so
    /// the editor can highlight it
    pub fn slice(&self, text: &str, range: Range) -> String {
        if range.start.line == range.end.line {
            let line = self.line_text(text, range.start.line as usize);
            let chars: Vec<char> = line.chars().collect();
            let from = (range.start.character as usize).min(chars.len());
            let to = (range.end.character as usize).min(chars.len());
            if from <= to {
                return chars[from..to].iter().collect();
            }
            return String::new();
        }
        let mut out = String::new();
        for line in range.start.line..=range.end.line {
            let line_text = self.line_text(text, line as usize);
            let chars: Vec<char> = line_text.chars().collect();
            let from = if line == range.start.line {
                (range.start.character as usize).min(chars.len())
            } else {
                0
            };
            let to = if line == range.end.line {
                (range.end.character as usize).min(chars.len())
            } else {
                chars.len()
            };
            if from <= to {
                out.extend(chars[from.min(chars.len())..to.min(chars.len())].iter());
            }
            out.push('\n');
        }
        out
    }
}

pub fn lsp_range(text: &str, span: Span) -> Range {
    LineIndex::new(text).range(text, span)
}

pub fn token_at<'a>(text: &str, pos: Position, tokens: &'a [Token]) -> Option<&'a Token> {
    let index = LineIndex::new(text);
    token_at_indexed(&index, text, pos, tokens)
}

/// Resolves every candidate range through a caller-built index. Building the
/// index per token is what made hovering a large file quadratic, and the
/// token stream is not guaranteed sorted (macro expansion can interleave
/// spans), so this stays a scan rather than a binary search.
pub fn token_at_indexed<'a>(
    index: &LineIndex,
    text: &str,
    pos: Position,
    tokens: &'a [Token],
) -> Option<&'a Token> {
    tokens.iter().find(|(_, span)| {
        let range = index.range(text, *span);
        pos >= range.start && pos < range.end
    })
}

/// Hover for a name that resolves to a symbol.
///
/// `@declare` says what exists rather than what to build, and a reader asking
/// "what is this" is asking a different question of one than of the other: a
/// declared name has a signature and no body, and whether there is anything to
/// call is not a question the signature answers. The flag is read off the symbol
/// rather than passed in, so it cannot get out of step with what was resolved.
pub fn to_hover(text: &str, token: &Token, symbol: Option<&Symbol>) -> Hover {
    let (kind, span) = token;
    let name = match kind {
        TokenKind::Ident(name) => name.as_str(),
        t if t.is_keyword() => t.name(),
        _ => "<symbol>",
    };
    let ty = symbol.and_then(|i| i.ty_display.as_deref());
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
                    _ => {
                        let scope = match symbol {
                            Some(s) if s.is_global => "global",
                            Some(_) => "local",
                            None => "unresolved",
                        };
                        match ty {
                            Some(ty) if !ty.is_empty() => {
                                format!("{scope} {name}: {ty}")
                            }
                            _ => format!("{scope} {name}"),
                        }
                    }
                }
            ),
        },
        _ => MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("`{}`", kind),
        },
    };
    let mut hover = Hover {
        contents: HoverContents::Markup(contents),
        range: Some(lsp_range(text, *span)),
    };
    if symbol.is_some_and(|s| s.attributes.has_declare()) {
        append_markup(&mut hover, DECLARED_NOTE);
    }
    hover
}

/// What a `@declare`d name is, said under the hover rather than inside the
/// signature, so the signature stays the signature.
pub const DECLARED_NOTE: &str = "\n`@declare` -- this is **only** a declaration.";

/// Hover for `container.member`. The plain identifier hover resolves to the
/// symbol the member was read from, which is the whole record or module, so
/// field and method reads get their own rendering.
pub fn to_member_hover(text: &str, token: &Token, container: &str, detail: &str) -> Hover {
    member_hover(text, token, container, detail, false)
}

/// As [`to_member_hover`], for a member the other module only declared.
///
/// A member read across a `require` is the one case where "is this a value?"
/// has no answer in the value layer: nothing was built, so the only thing the
/// importer was given is the shape. Saying so is the difference between a member
/// that can be read and one that was promised.
pub fn member_hover(
    text: &str,
    token: &Token,
    container: &str,
    detail: &str,
    declared: bool,
) -> Hover {
    let (kind, span) = token;
    let name = match kind {
        TokenKind::Ident(name) => name.as_str(),
        _ => "",
    };
    let tag = if detail.starts_with("function") {
        "method"
    } else {
        "field"
    };
    let mut value = format!("```duka\n({tag}) {container}.{name}: {detail}\n```");
    if declared {
        value.push_str(&format!("\n\n{}", DECLARED_NOTE));
    }
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lsp_range(text, *span)),
    }
}

/// `{ a = 1 }` - the name on the left of an `=` inside braces is a table key,
/// not a variable, so it must not borrow the hover of a same-named local
pub fn to_table_key_hover(index: &LineIndex, text: &str, token: &Token) -> Option<Hover> {
    let (kind, span) = token;
    let name = ident_text(kind)?;
    let source = index.slice(text, index.range(text, *span));
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "```duka\n{source}\n```\n`(table key) {name}`\n\nField of the table literal."
            ),
        }),
        range: Some(lsp_range(text, *span)),
    })
}

/// Adds a block below an existing hover, so a generic call keeps its resolved
/// signature and gains the type parameter bindings instead of losing them
pub fn append_markup(hover: &mut Hover, extra: &str) {
    if let HoverContents::Markup(MarkupContent { value, .. }) = &mut hover.contents {
        value.push_str(extra);
    }
}

/// A hover whose body is already markdown, for the cases that assemble their
/// own text rather than deriving it from a symbol
pub fn to_markup_hover(text: &str, token: &Token, value: &str) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: value.to_string(),
        }),
        range: Some(lsp_range(text, token.1)),
    }
}

/// A parameter is a local, but calling it one is what the reader needs to see
pub fn to_param_hover(text: &str, token: &Token, ty: Option<&str>) -> Hover {
    let (kind, span) = token;
    let name = ident_text(kind).unwrap_or_default();
    let value = match ty.filter(|t| !t.is_empty()) {
        Some(ty) => format!("```duka\n(param) {name}: {ty}\n```"),
        None => format!("```duka\n(param) {name}\n```"),
    };
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lsp_range(text, *span)),
    }
}

pub fn ident_text(kind: &TokenKind) -> Option<&str> {
    match kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    }
}

/// `...` is neither a keyword nor a symbol, so it needs its own hover
pub fn to_vararg_hover(text: &str, token: &Token) -> Option<Hover> {
    let (kind, span) = token;
    if !matches!(kind, TokenKind::Dots) {
        return None;
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```duka\n...\n```\n`vararg`\n\nCollects every remaining argument into a list.\nIn a parameter list it declares the rest parameter.".to_string(),
        }),
        range: Some(lsp_range(text, *span)),
    })
}

/// Int, float, string and bool literals carry no symbol, so they used to fall
/// through every lookup and hover as nothing. The literal is echoed verbatim in
/// a fenced block so the editor highlights it, followed by the details that are
/// worth knowing for that kind.
pub fn to_literal_hover(index: &LineIndex, text: &str, token: &Token) -> Option<Hover> {
    let (kind, span) = token;
    let range = index.range(text, *span);
    let mut source = index.slice(text, range);
    // a negative literal is a unary minus in front of the number token
    let mut negative = false;
    if matches!(kind, TokenKind::Int(_)) {
        let line = index.line_text(text, range.start.line as usize);
        let chars: Vec<char> = line.chars().collect();
        let before = chars.get(range.start.character.wrapping_sub(1) as usize);
        if before.is_some_and(|c| *c == '-') {
            source.insert(0, '-');
            negative = true;
        }
    }
    let (type_name, details): (&str, Vec<String>) = match kind {
        TokenKind::Int(v) => {
            let v = if negative { -*v } else { *v };
            let mut details = vec![format!("decimal: `{v}`")];
            if v >= 0 {
                details.push(format!("hex: `0x{v:x}`"));
                details.push(format!("octal: `0o{v:o}`"));
                details.push(format!("binary: `0b{v:b}`"));
            }
            ("int", details)
        }
        TokenKind::Float(v) => {
            let mut details = vec![format!("value: `{v}`")];
            if v.is_finite() {
                details.push(format!("scientific: `{v:e}`"));
            }
            ("float", details)
        }
        TokenKind::String(bytes) => {
            let decoded = String::from_utf8_lossy(bytes);
            let mut hex: Vec<String> = bytes.iter().take(24).map(|b| format!("{b:02x}")).collect();
            if bytes.len() > 24 {
                hex.push("...".to_string());
            }
            (
                "string",
                vec![
                    format!("characters: {}", decoded.chars().count()),
                    format!("bytes: {}", bytes.len()),
                    format!("byte values: `{}`", hex.join(" ")),
                ],
            )
        }
        TokenKind::True => ("bool", vec!["value: `true`".to_string()]),
        TokenKind::False => ("bool", vec!["value: `false`".to_string()]),
        _ => return None,
    };

    let mut value = format!("```duka\n{source}\n```\n\n`{type_name}`\n");
    for detail in details {
        value.push_str(&format!("\n- {detail}"));
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(lsp_range(text, *span)),
    })
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

/// A diagnostic belongs to the file the error actually came from, a required
/// module reports against its own uri and its own text
pub fn error_uri(err: &DukaDiagnose, fallback: &Url) -> Url {
    match &err.source_info().name {
        duka_lib::duka_shared::types::SourceName::File(_, path) => {
            Url::from_file_path(path.as_ref()).unwrap_or_else(|_| fallback.clone())
        }
        duka_lib::duka_shared::types::SourceName::Virtual(_) => fallback.clone(),
        _ => fallback.clone(),
    }
}

/// The text an error's span refers to
pub fn error_text(err: &DukaDiagnose) -> String {
    String::from_utf8_lossy(&err.source_info().source).into_owned()
}

/// The advice the compiler attached to a semantic error
pub fn error_help(err: &DukaDiagnose) -> Option<String> {
    match err.kind() {
        duka_lib::duka_shared::errors::DukaErrorKind::Semantic(e) => Some(e.get_help()),
        _ => None,
    }
}

/// A diagnostic's own level, in the editor's vocabulary.
///
/// The severity travels with the diagnostic rather than being decided here, so
/// a warning is shown as a warning whether it came from the analyser, the type
/// checker or anywhere else.
fn severity(level: Severity) -> DiagnosticSeverity {
    match level {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

pub fn to_diagnostic(text: &str, uri: &Url, err: &DukaDiagnose) -> Diagnostic {
    let related = if err.related().is_empty() {
        None
    } else {
        Some(
            err.related()
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
    let message = match error_help(err) {
        Some(help) => format!("{}\n\n{}", err.kind(), help),
        None => err.kind().to_string(),
    };

    Diagnostic::new(
        lsp_range(text, err.span()),
        Some(severity(err.severity())),
        None,
        Some("duka".into()),
        message,
        related,
        None,
    )
}

fn ident_semantic_type(
    names: &HashMap<&str, SymbolType>,
    keywordish: &HashSet<&str>,
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
    // a declaration written `@keywordish` reads as a keyword rather than as a
    // name, wherever it is mentioned
    if keywordish.contains(name.as_str()) {
        return Some(SEMANTIC_KEYWORD);
    }
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
    match names.get(name.as_str()) {
        Some(SymbolType::Function) => Some(SEMANTIC_FUNCTION),
        Some(SymbolType::Constant(_)) => Some(SEMANTIC_KEYWORD),
        Some(SymbolType::ObjectClass(_)) => Some(SEMANTIC_TYPE),
        Some(SymbolType::TypeAlias(_))
        | Some(SymbolType::TypeFunction(_))
        | Some(SymbolType::InlineTypeFunction(_)) => Some(SEMANTIC_TYPE),
        _ => Some(SEMANTIC_VARIABLE),
    }
}

fn utf16_len(index: &LineIndex, text: &str, span: Span) -> u32 {
    let line_idx = span.start.line.saturating_sub(1) as usize;
    let line_text = index.line_text(text, line_idx);
    let from = span.start.column.saturating_sub(1) as usize;
    let to = span.end.column.saturating_sub(1) as usize;
    line_text
        .chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// Pulls `name -> type` pairs out of a record type.
///
/// Read off the type, which is where the field names are right now, rather than
/// off the rendered text: a record renders one field per line, and taking that
/// back apart is how a member ends up wrong about a field whose type happens to
/// contain a brace.
pub fn record_members(ty: &Type) -> Vec<(String, String)> {
    let Type::TypeTable(fields) = ty else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(|(key, value)| {
            let ConstValue::String(name) = key else {
                return None;
            };
            Some((
                String::from_utf8_lossy(name).into_owned(),
                value.to_string(),
            ))
        })
        .collect()
}

/// Pulls `name -> type` pairs out of a rendered record type like
/// `{ x: int, y: int }`. The symbol table keeps types as display text, so
/// member completion has to read them back; nesting is tracked so a comma

/// Every token that belongs to an attribute: the sigil, the name, and whatever
/// the name is given to. `@name(...)` is one syntactic form, so a word inside
/// the parentheses belongs to the attribute rather than to whatever it happens
/// to name. The rule is about the form, not about any particular attribute.
fn attribute_tokens(
    tokens: &[duka_lib::duka_frontend::lexer::token::Token],
) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    for (i, (kind, _)) in tokens.iter().enumerate() {
        if !matches!(kind, TokenKind::At) {
            continue;
        }
        // The next thing that is not a comment. A `---` line can stand between
        // the sigil and the name it documents, and looking only at `i + 1` would
        // decide that `@` introduces nothing.
        let mut next = i + 1;
        while tokens.get(next).is_some_and(|(k, _)| k.is_comment()) {
            next += 1;
        }
        if !matches!(tokens.get(next).map(|(k, _)| k), Some(TokenKind::Ident(_))) {
            continue;
        }
        out.insert(i);
        out.insert(next);
        let mut after = next + 1;
        while tokens.get(after).is_some_and(|(k, _)| k.is_comment()) {
            after += 1;
        }
        if !matches!(tokens.get(after).map(|(k, _)| k), Some(TokenKind::LParen)) {
            continue;
        }
        let mut depth = 0usize;
        for (j, (k, _)) in tokens.iter().enumerate().skip(after) {
            match k {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        out.insert(j);
                        break;
                    }
                }
                _ => {}
            }
            out.insert(j);
        }
    }
    out
}

pub fn semantic_tokens(
    text: &str,
    tokens: &[duka_lib::duka_frontend::lexer::token::Token],
    table: &SymbolTable,
    roles: &HashMap<Span, Role>,
) -> Vec<SemanticToken> {
    let mut data = Vec::with_capacity(tokens.len());
    let index = LineIndex::new(text);
    // Same resolution order as `SymbolTable::lookup_named`, but paid once for
    // the whole file instead of walking every scope for every identifier.
    let mut names: HashMap<&str, SymbolType> = HashMap::new();
    let mut keywordish: HashSet<&str> = HashSet::new();
    for scope in table.scopes().iter().rev() {
        for (name, syms) in &scope.symbols {
            if let Some(sym) = syms.iter().rev().find(|s| s.is_value()) {
                if sym.attributes.has_keywordish() {
                    keywordish.insert(name.as_ref());
                }
                names
                    .entry(name.as_ref())
                    .or_insert(sym.symbol_type.clone());
            }
        }
    }
    let attributes = attribute_tokens(tokens);
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;
    for i in 0..tokens.len() {
        let (kind, span) = &tokens[i];
        if kind.is_terminator() {
            continue;
        }
        // The neighbouring tokens that are not comments. A `---` line can stand
        // between `x` and the `:` that types it, and looking only at `i - 1` would
        // decide that `x` declares nothing.
        let mut before = i;
        while before > 0 && tokens.get(before - 1).is_some_and(|(k, _)| k.is_comment()) {
            before -= 1;
        }
        let mut after = i + 1;
        while tokens.get(after).is_some_and(|(k, _)| k.is_comment()) {
            after += 1;
        }
        let next = tokens.get(after).map(|(k, _)| k);
        let prev = if before == 0 {
            None
        } else {
            tokens.get(before - 1).map(|(k, _)| k)
        };
        let token_type = if attributes.contains(&i) {
            Some(SEMANTIC_ATTRIBUTE)
        } else {
            match kind {
                TokenKind::Dots => Some(SEMANTIC_VARARG),
                TokenKind::Ident(name) => {
                    if is_metamethod(name) {
                        Some(SEMANTIC_METAMETHOD)
                    } else {
                        match roles.get(span) {
                            Some(Role::MethodCall) => Some(SEMANTIC_FUNCTION),
                            Some(Role::FieldAccess) => Some(SEMANTIC_PROPERTY),
                            None => ident_semantic_type(&names, &keywordish, kind, prev, next),
                        }
                    }
                }
                _ => None,
            }
        };
        let Some(token_type) = token_type else {
            continue;
        };
        let start = index.position(text, span.start.line, span.start.column);
        let delta_line = start.line - prev_line;
        let delta_start = if delta_line == 0 {
            start.character - prev_char
        } else {
            start.character
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: utf16_len(&index, text, *span),
            token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = start.line;
        prev_char = start.character;
    }
    data
}
