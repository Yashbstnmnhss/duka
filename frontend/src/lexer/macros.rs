use std::{
    path::Path,
    sync::{LazyLock, RwLock},
};

use duka_shared::{
    builtin::{Builtins, GlobalBuiltins},
    constants::clex,
    errors::Span,
    types::{SourceName, Spanned},
    value::DukaInt,
};

use crate::lexer::token::{Token, TokenKind};

#[derive(Debug)]
pub enum MacroToken {
    /// pure token
    Token(Token),
    /// index of parameter
    Replace(usize),
    /// separator, separator join type
    VarArg(Token, VarArgSeparator),
}

#[derive(Debug)]
pub enum VarArgSeparator {
    Left,
    Right,
    All,
    None,
}
pub type MacroName = String;
pub type MacroError = Spanned<String>;
pub type MacroParam = Vec<Token>;
pub type MacroBody = (usize, Vec<MacroToken>);
pub type MacroExpanding = (MacroName, u16);
pub type MacroFunc =
    fn(SourceName, Span, &[MacroExpanding], Vec<MacroParam>) -> Result<Vec<Token>, MacroError>;

pub static MACRO_BUILTINS: GlobalBuiltins<MacroFunc> = LazyLock::new(|| {
    RwLock::new({
        Builtins::<MacroFunc>::new()
            .register(clex::NAMEOF, |_, _, _, tks| {
                Ok(tks
                    .into_iter()
                    .next()
                    .map(|tks| {
                        tks.into_iter()
                            .next()
                            .map(|(tk, span)| {
                                vec![(TokenKind::String(tk.name().as_bytes().into()), span)]
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default())
            })
            .register(clex::STRINGIFY, |_, _, _, tks| {
                Ok(tks
                    .into_iter()
                    .next()
                    .map(|tks| {
                        tks.into_iter()
                            .next()
                            .map(|(tk, span)| {
                                vec![(
                                    TokenKind::String(
                                        tk.stringify().into_owned().as_bytes().into(),
                                    ),
                                    span,
                                )]
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default())
            })
            .register(clex::CONCAT, |_, call_site, _, tks| {
                let str: String = tks
                    .into_iter()
                    .filter_map(|tks| tks.into_iter().next())
                    .filter_map(|tk| {
                        Some(match tk.0 {
                            TokenKind::Ident(id) => id,
                            TokenKind::Int(i) => i.to_string(),
                            TokenKind::Float(f) => f.to_string(),
                            t if t.is_keyword() => t.name().to_owned(),
                            _ => return None,
                        })
                    })
                    .collect();
                Ok(vec![(TokenKind::Ident(str), call_site)])
            })
            .register(clex::COUNTER, |_, call_site, expanding, _| {
                Ok(vec![(
                    TokenKind::Int(expanding.last().map(|i| i.1).unwrap_or_default() as DukaInt),
                    call_site,
                )])
            })
            .register(clex::WHEN, |_, _, _, params| {
                let mut params = params.into_iter();
                let cond = params
                    .next()
                    .unwrap_or_default()
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let body = params.next().unwrap_or_default();

                Ok(matches!(cond.0, TokenKind::True)
                    .then_some(body)
                    .unwrap_or_else(|| params.next().unwrap_or_default()))
            })
            .register(clex::NONEMPTY, |_, call_site, _, params| {
                Ok(vec![(
                    if params.is_empty() {
                        TokenKind::False
                    } else {
                        TokenKind::True
                    },
                    call_site,
                )])
            })
            .register(clex::LENIS, |_, call_site, _, mut params| {
                Ok(vec![(
                    if let Some(tks) = params.pop()
                        && let Some((TokenKind::Int(len), _)) = tks.first()
                    {
                        if params.len() == *len as usize {
                            TokenKind::False
                        } else {
                            TokenKind::True
                        }
                    } else {
                        TokenKind::False
                    },
                    call_site,
                )])
            })
            .register(clex::INCLUDE, |source_name, call_site, _, params| {
                if let Some(tks) = params.into_iter().next()
                    && let Some((TokenKind::String(path), span)) = tks.into_iter().next()
                {
                    let SourceName::File(_, source_path) = source_name else {
                        return Err(("\"include\" isn't supported here".to_owned(), call_site));
                    };

                    let path = Path::new(str::from_utf8(&path).map_err(|e| (e.to_string(), span))?);
                    let final_path = if path.is_absolute() {
                        path
                    } else {
                        &source_path.join(path)
                    };
                    let content = std::fs::read(final_path).map_err(|e| (e.to_string(), span))?;
                    Ok(vec![(TokenKind::String(content.into_boxed_slice()), span)])
                } else {
                    Err(("Expected file path constant string".to_owned(), call_site))
                }
            })
    })
});
