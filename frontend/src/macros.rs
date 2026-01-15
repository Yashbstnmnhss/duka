use std::sync::{LazyLock, RwLock};

use duka_shared::{
    builtin::{Builtins, GlobalBuiltins},
    constants::clex,
    error::Span,
    token::{Token, TokenKind},
    value::DukaInt,
};

#[derive(Debug)]
pub enum MacroToken {
    /// pure token
    Token(Token),
    /// index of parameter
    Replace(usize),
    /// separator, separator join type
    VarArg(Token, VarArgSeparatorType),
}

#[derive(Debug)]
pub enum VarArgSeparatorType {
    Left,
    Right,
    All,
    None,
}
pub type MacroName = String;
pub type MacroParam = Vec<Token>;
pub type MacroBody = (usize, Vec<MacroToken>);
pub type MacroExpanding = (MacroName, u16);
pub type MacroFunc = fn(Span, &[MacroExpanding], Vec<MacroParam>) -> Vec<Token>;

pub static MACRO_BUILTINS: GlobalBuiltins<MacroFunc> = LazyLock::new(|| {
    RwLock::new({
        Builtins::<MacroFunc>::new()
            .register(clex::NAMEOF, |_, _, tks| {
                tks.into_iter()
                    .next()
                    .map(|tks| {
                        tks.into_iter()
                            .next()
                            .map(|(tk, span)| vec![(TokenKind::String(tk.name().into()), span)])
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            })
            .register(clex::STRINGIFY, |_, _, tks| {
                tks.into_iter()
                    .next()
                    .map(|tks| {
                        tks.into_iter()
                            .next()
                            .map(|(tk, span)| {
                                vec![(TokenKind::String(tk.stringify().into_owned().into()), span)]
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            })
            .register(clex::CONCAT, |call_site, _, tks| {
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
                vec![(TokenKind::Ident(str), call_site)]
            })
            .register(clex::COUNTER, |call_site, expanding, _| {
                vec![(
                    TokenKind::Int(expanding.last().map(|i| i.1).unwrap_or_default() as DukaInt),
                    call_site,
                )]
            })
            .register(clex::WHEN, |_, _, params| {
                let mut params = params.into_iter();
                let cond = params
                    .next()
                    .unwrap_or_default()
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let body = params.next().unwrap_or_default();

                matches!(cond.0, TokenKind::True)
                    .then_some(body)
                    .unwrap_or_else(|| params.next().unwrap_or_default())
            })
            .register(clex::NONEMPTY, |call_site, _, params| {
                vec![(
                    params
                        .is_empty()
                        .then_some(TokenKind::False)
                        .unwrap_or(TokenKind::True),
                    call_site,
                )]
            })
            .register(clex::LENIS, |call_site, _, mut params| {
                if let Some(tks) = params.pop()
                    && let Some((TokenKind::Int(len), _)) = tks.first()
                {
                    vec![(
                        (params.len() == *len as usize)
                            .then_some(TokenKind::False)
                            .unwrap_or(TokenKind::True),
                        call_site,
                    )]
                } else {
                    vec![(TokenKind::False, call_site)]
                }
            })
    })
});
