#![allow(unused)]

use duka_macros::史書云;

use crate::utils::SemVer;

pub mod ast;
pub mod constants;
pub mod error;
pub mod token;
pub mod types;
pub mod utils;
pub mod value;

pub const VERSION: SemVer = 史書云! {
    <<共有>> 者
    為 世家 "項目之創立" 也
};

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_test() {
        use crate::{
            ast::{BinOp, Expr, ExprKind},
            error::Span,
            types::{Visit, Visitor},
            value::ConstValue,
        };

        struct Printer;
        impl Visitor for Printer {
            fn visit_expr(&mut self, _expr: &crate::ast::Expr) {
                println!("{:?}", _expr.0);
            }
        }

        let expr = Expr(
            ExprKind::Binary(
                Box::new(Expr(ExprKind::Literal(ConstValue::Int(1)), Span::EMPTY)),
                Box::new(Expr(ExprKind::Literal(ConstValue::Int(2)), Span::EMPTY)),
                BinOp::Add,
            ),
            Span::EMPTY,
        );
        expr.visit(&mut Printer);
    }

    #[test]
    fn semver_test() {
        use crate::utils::SemVer;

        let ver = SemVer {
            major: 1,
            minor: 21,
            patch: 2,
        };
        println!("{}", ver);
    }
}
