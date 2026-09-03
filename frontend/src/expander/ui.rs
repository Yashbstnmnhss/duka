use crate::lexer::token::{Token, TokenKind};
use crate::parser::ast::{BangMacroNode, Expr, ExprKind, Field, Path};
use duka_shared::errors::Span;
use duka_shared::value::ConstValue;

use super::{BangExpander, BangExpanderError};

pub struct UIExpanderAdapter;

impl BangExpander for UIExpanderAdapter {
    fn expand(&self, node: &BangMacroNode) -> Result<ExprKind, BangExpanderError> {
        UIExpander::expand(node)
    }
}

struct UIExpander;

impl UIExpander {
    fn expand(node: &BangMacroNode) -> Result<ExprKind, BangExpanderError> {
        let mut parser = UIParser::new(&node.tokens);
        let expr = parser
            .parse_element()
            .map_err(BangExpanderError::ParseError)?;
        if parser.pos < parser.tokens.len() {
            return Err(BangExpanderError::ParseError(format!(
                "unexpected token after UI element at position {}",
                parser.pos
            )));
        }
        Ok(expr)
    }
}

struct UIParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> UIParser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.0)
    }

    fn next(&mut self) -> Option<&TokenKind> {
        let kind = self.tokens.get(self.pos).map(|t| &t.0);
        if kind.is_some() {
            self.pos += 1;
        }
        kind
    }

    fn expect(&mut self, expected: &TokenKind) -> Result<(), String> {
        match self.next() {
            Some(tok) if tok == expected => Ok(()),
            Some(tok) => Err(format!("expected {:?}, got {:?}", expected, tok)),
            None => Err(format!("expected {:?}, got EOF", expected)),
        }
    }

    fn peek_is_props(&self) -> bool {
        if self.peek() != Some(&TokenKind::LParen) {
            return false;
        }
        if let Some(TokenKind::Ident(_)) = self.tokens.get(self.pos + 1).map(|t| &t.0) {
            if let Some(token) = self.tokens.get(self.pos + 2) {
                return matches!(token.0, TokenKind::Assign | TokenKind::RParen);
            }
        }
        false
    }

    fn lparen_followed_by_brace(&self) -> bool {
        self.peek() == Some(&TokenKind::LParen) && self.peek_n(2) == Some(&TokenKind::LBrace)
    }

    fn peek_n(&self, n: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + n).map(|t| &t.0)
    }

    fn parse_element(&mut self) -> Result<ExprKind, String> {
        let tag = self.parse_tag_name()?;
        let props = if self.peek() == Some(&TokenKind::LParen) {
            self.parse_props()?
        } else {
            None
        };
        let children = if self.peek() == Some(&TokenKind::LBrace) {
            self.parse_children()?
        } else {
            vec![]
        };
        Ok(self.make_element(tag, props, children))
    }

    fn make_element(&self, tag: String, props: Option<ExprKind>, children: Vec<Expr>) -> ExprKind {
        let mut args = vec![string_expr(&tag)];
        match props {
            Some(props_expr) => {
                args.push(Expr(props_expr, Span::EMPTY));
            }
            None => {
                args.push(Expr(ExprKind::Table(Box::default()), Span::EMPTY));
            }
        }
        for child in children {
            args.push(child);
        }
        ExprKind::Call(
            Box::new(Expr(
                ExprKind::Access(Box::new(Path::Base(("h".to_string(), Span::EMPTY)))),
                Span::EMPTY,
            )),
            args.into_boxed_slice(),
        )
    }

    fn parse_tag_name(&mut self) -> Result<String, String> {
        match self.next() {
            Some(TokenKind::Ident(name)) => Ok(name.clone()),
            Some(TokenKind::Local) => Ok("local".to_string()),
            Some(TokenKind::Function) => Ok("function".to_string()),
            Some(TokenKind::If) => Ok("if".to_string()),
            Some(TokenKind::Else) => Ok("else".to_string()),
            Some(TokenKind::For) => Ok("for".to_string()),
            Some(TokenKind::While) => Ok("while".to_string()),
            Some(TokenKind::Return) => Ok("return".to_string()),
            Some(tok) => Err(format!("expected tag name, got {:?}", tok)),
            None => Err("expected tag name, got EOF".to_string()),
        }
    }

    fn parse_props(&mut self) -> Result<Option<ExprKind>, String> {
        self.expect(&TokenKind::LParen)?;

        // Empty props: ()
        if self.peek() == Some(&TokenKind::RParen) {
            self.expect(&TokenKind::RParen)?;
            return Ok(None);
        }

        // Variable-as-props: (var) — a single identifier not followed by `=`
        // Handles patterns like: path(path_attrs) or div(dynamic_props)
        let is_key_value = match (self.peek(), self.tokens.get(self.pos + 1).map(|t| &t.0)) {
            (
                Some(TokenKind::Ident(_)) | Some(TokenKind::Local) | Some(TokenKind::Function),
                Some(TokenKind::Assign),
            ) => true,
            _ => false,
        };

        if !is_key_value {
            let expr = self.parse_prop_value()?;
            self.expect(&TokenKind::RParen)?;
            return Ok(Some(expr.0));
        }

        let mut fields = vec![];

        while self.peek() != Some(&TokenKind::RParen) {
            if self.peek().is_none() {
                return Err("unexpected EOF in props".to_string());
            }
            let key = match self.next() {
                Some(TokenKind::Ident(k)) => k.clone(),
                Some(TokenKind::Local) => "local".to_string(),
                Some(TokenKind::Function) => "function".to_string(),
                Some(tok) => return Err(format!("expected prop name, got {:?}", tok)),
                None => return Err("expected prop name, got EOF".to_string()),
            };
            self.expect(&TokenKind::Assign)?;
            let value = self.parse_prop_value()?;
            fields.push(Field::KeyValue(
                Expr(
                    ExprKind::Literal(ConstValue::String(key.into_bytes().into())),
                    Span::EMPTY,
                ),
                value,
            ));

            if self.peek() == Some(&TokenKind::Comma) {
                self.next();
            }
        }
        self.expect(&TokenKind::RParen)?;

        if fields.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ExprKind::Table(fields.into_boxed_slice())))
        }
    }

    fn parse_prop_value(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(TokenKind::String(s)) => {
                let s = s.clone();
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::String(s)), Span::EMPTY))
            }
            Some(TokenKind::Int(n)) => {
                let n = *n;
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Int(n)), Span::EMPTY))
            }
            Some(TokenKind::Float(f)) => {
                let f = *f;
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Float(f)), Span::EMPTY))
            }
            Some(TokenKind::True) => {
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Bool(true)), Span::EMPTY))
            }
            Some(TokenKind::False) => {
                self.next();
                Ok(Expr(
                    ExprKind::Literal(ConstValue::Bool(false)),
                    Span::EMPTY,
                ))
            }
            Some(TokenKind::Nil) => {
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Nil), Span::EMPTY))
            }
            Some(TokenKind::Ident(name)) => {
                let name = name.clone();
                self.next();
                Ok(Expr(
                    ExprKind::Access(Box::new(Path::Base((name, Span::EMPTY)))),
                    Span::EMPTY,
                ))
            }
            Some(TokenKind::LBrace) => self.parse_element().map(|e| Expr(e, Span::EMPTY)),
            Some(tok) => Err(format!("unexpected token in prop value: {:?}", tok)),
            None => Err("unexpected EOF in prop value".to_string()),
        }
    }

    fn parse_children(&mut self) -> Result<Vec<Expr>, String> {
        self.expect(&TokenKind::LBrace)?;
        let mut children = vec![];

        while self.peek() != Some(&TokenKind::RBrace) {
            if self.peek().is_none() {
                return Err("unexpected EOF in children".to_string());
            }
            let child = self.parse_child()?;
            children.push(child);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(children)
    }

    fn parse_child(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(TokenKind::LBrace) => self.parse_element().map(|e| Expr(e, Span::EMPTY)),
            Some(TokenKind::String(s)) => {
                let s = s.clone();
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::String(s)), Span::EMPTY))
            }
            Some(TokenKind::Ident(name)) => {
                let name = name.clone();
                self.next();
                match self.peek() {
                    Some(TokenKind::LParen) if self.peek_is_props() => {
                        let props = self.parse_props()?;
                        let children = if self.peek() == Some(&TokenKind::LBrace) {
                            self.parse_children()?
                        } else {
                            vec![]
                        };
                        Ok(Expr(self.make_element(name, props, children), Span::EMPTY))
                    }
                    Some(TokenKind::LParen) if self.lparen_followed_by_brace() => {
                        // Empty props `tag()` followed by children block: treat as element
                        self.expect(&TokenKind::LParen)?;
                        self.expect(&TokenKind::RParen)?;
                        let children = self.parse_children()?;
                        Ok(Expr(self.make_element(name, None, children), Span::EMPTY))
                    }
                    Some(TokenKind::LParen) => self.parse_call(name),
                    Some(TokenKind::LBrace) => {
                        let children = self.parse_children()?;
                        Ok(Expr(self.make_element(name, None, children), Span::EMPTY))
                    }
                    _ => Ok(Expr(
                        ExprKind::Access(Box::new(Path::Base((name, Span::EMPTY)))),
                        Span::EMPTY,
                    )),
                }
            }
            Some(TokenKind::Int(n)) => {
                let n = *n;
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Int(n)), Span::EMPTY))
            }
            Some(TokenKind::Float(f)) => {
                let f = *f;
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Float(f)), Span::EMPTY))
            }
            Some(TokenKind::True) => {
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Bool(true)), Span::EMPTY))
            }
            Some(TokenKind::False) => {
                self.next();
                Ok(Expr(
                    ExprKind::Literal(ConstValue::Bool(false)),
                    Span::EMPTY,
                ))
            }
            Some(TokenKind::Nil) => {
                self.next();
                Ok(Expr(ExprKind::Literal(ConstValue::Nil), Span::EMPTY))
            }
            Some(TokenKind::LParen) => {
                self.next();
                let expr = self.parse_child()?;
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
            }
            Some(tok) => Err(format!("unexpected token in child: {:?}", tok)),
            None => Err("unexpected EOF in child".to_string()),
        }
    }

    fn parse_call(&mut self, name: String) -> Result<Expr, String> {
        self.expect(&TokenKind::LParen)?;
        let mut args = vec![];

        while self.peek() != Some(&TokenKind::RParen) {
            if self.peek().is_none() {
                return Err("unexpected EOF in call args".to_string());
            }
            let arg = self.parse_child()?;
            args.push(arg);
            if self.peek() == Some(&TokenKind::Comma) {
                self.next();
            }
        }
        self.expect(&TokenKind::RParen)?;

        Ok(Expr(
            ExprKind::Call(
                Box::new(Expr(
                    ExprKind::Access(Box::new(Path::Base((name, Span::EMPTY)))),
                    Span::EMPTY,
                )),
                args.into_boxed_slice(),
            ),
            Span::EMPTY,
        ))
    }
}

fn string_expr(s: &str) -> Expr {
    Expr(
        ExprKind::Literal(ConstValue::String(s.as_bytes().to_vec().into())),
        Span::EMPTY,
    )
}
