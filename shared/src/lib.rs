#![allow(unused)]

pub mod ast;
pub mod error;
pub mod gc;
pub mod token;
pub mod types;
pub mod utils;
pub mod value;

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_test() {
        use crate::{
            ast::{BinOp, Expr, ExprKind},
            error::Span,
            types::{Visit, Visitor},
            value::Value,
        };

        struct Printer;
        impl Visitor for Printer {
            fn visit_expr(&mut self, _expr: &crate::ast::Expr) {
                println!("{:?}", _expr.0);
            }
        }

        let expr = Expr(
            ExprKind::Binary(
                Box::new(Expr(ExprKind::Literal(Value::Int(1)), Span::EMPTY)),
                Box::new(Expr(ExprKind::Literal(Value::Int(2)), Span::EMPTY)),
                BinOp::Add,
            ),
            Span::EMPTY,
        );
        expr.visit(&mut Printer);
    }

    #[test]
    fn gc_test() {
        use crate::gc::*;
    }
}
