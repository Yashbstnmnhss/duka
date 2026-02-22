pub mod visitors;

use duka_shared::{
    errors::{DukaSpannedError, Span},
    types::{DukaAdapter, DukaAnalyzer},
    utils::Scopes,
};

use crate::{
    analyzer::visitors::{
        ConstFoldTransformer, DesugarTransformer, LabelChecker, LoopChecker,
        MeaninglessTransformer, VarArgChecker,
    },
    parser::ast::{
        DukaChunk, Expr, ExprKind, FuncBody, IfClause, Match, MatchClause, Stmt, StmtKind,
    },
};

pub trait Visit {
    fn visit<V: Visitor>(&self, visitor: &mut V);
}
pub trait VisitMut {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V);
}

impl<T: Visit> Visit for Option<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        if let Some(self_) = self {
            self_.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Option<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        if let Some(self_) = self {
            self_.visit_mut(visitor);
        }
    }
}
impl<T: Visit> Visit for Box<[T]> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        for el in self {
            el.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Box<[T]> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        for el in self {
            el.visit_mut(visitor);
        }
    }
}

impl<T: Visit> Visit for Box<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        (**self).visit(visitor);
    }
}
impl<T: VisitMut> VisitMut for Box<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        (**self).visit_mut(visitor);
    }
}

impl<T: Visit> Visit for Vec<T> {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        for self_ in self {
            self_.visit(visitor);
        }
    }
}
impl<T: VisitMut> VisitMut for Vec<T> {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        for self_ in self {
            self_.visit_mut(visitor);
        }
    }
}

impl<A: Visit, B: Visit> Visit for (A, B) {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        self.0.visit(visitor);
        self.1.visit(visitor);
    }
}
impl<A: VisitMut, B: VisitMut> VisitMut for (A, B) {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        self.0.visit_mut(visitor);
        self.1.visit_mut(visitor);
    }
}

impl<A, B, C: Visit> Visit for (A, B, C) {
    fn visit<V: Visitor>(&self, visitor: &mut V) {
        self.2.visit(visitor);
    }
}
impl<A, B, C: VisitMut> VisitMut for (A, B, C) {
    fn visit_mut<V: VisitorMut>(&mut self, visitor: &mut V) {
        self.2.visit_mut(visitor);
    }
}

pub trait Visitor {
    fn visit_stmt(&mut self, _stmt: &Stmt) {}
    fn visit_expr(&mut self, _expr: &Expr) {}
    fn before(&mut self) {}
    fn after(&mut self) {}
    fn visit_if_clause_block(&mut self, _block: &IfClause, _enter: bool) {}
    fn visit_match_else_block(&mut self, _block: &Match, _enter: bool) {}
    fn visit_match_clause_block(&mut self, _block: &MatchClause, _enter: bool) {}
    fn visit_func_block(&mut self, _block: &FuncBody, _enter: bool) {}
    fn visit_do_stmt_block(&mut self, _block: &StmtKind, _enter: bool) {}
    fn visit_do_expr_block(&mut self, _block: &ExprKind, _enter: bool) {}
    fn visit_loop_stmt_block(&mut self, _block: &StmtKind, _enter: bool) {}

    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
}
pub trait VisitorMut {
    fn visit_stmt(&mut self, _stmt: &mut Stmt) {}
    fn visit_expr(&mut self, _expr: &mut Expr) {}

    fn visit_block(&mut self, _enter: bool) {}
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmptyAnalyzer;
impl DukaAnalyzer for EmptyAnalyzer {
    type InputType = DukaChunk;
    type InputData = ();
    type OutputData = ();
    fn analyze(
        &self,
        _: &Self::InputType,
        _: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        ((), std::iter::empty())
    }
}

#[derive(Debug)]
pub struct ScopeAnalysis {
    pub labels: Scopes<Box<str>, Span>,
    pub functions: Scopes<Box<str>, Span>,
    pub variabls: Scopes<Box<str>, Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeAnalyzer;
impl DukaAnalyzer for ScopeAnalyzer {
    type InputType = DukaChunk;
    type InputData = ();
    type OutputData = ScopeAnalysis;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        _: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        struct ScopeVisitor;
        impl Visitor for ScopeVisitor {}

        chunk.visit(&mut ScopeVisitor);
        (todo!(), std::iter::empty())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analyzer;
impl DukaAnalyzer for Analyzer {
    type InputType = DukaChunk;
    type InputData = ();
    type OutputData = ();

    fn analyze(
        &self,
        chunk: &Self::InputType,
        _: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        (
            (),
            check(&mut LabelChecker::new(chunk.source_info.clone()), chunk)
                .into_iter()
                .chain(check(
                    &mut LoopChecker::new(chunk.source_info.clone()),
                    chunk,
                ))
                .chain(check(
                    &mut VarArgChecker::new(chunk.source_info.clone()),
                    chunk,
                )),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmptyAdapter;
impl DukaAdapter for EmptyAdapter {
    type InputType = DukaChunk;

    fn adapt(&self, _: &mut Self::InputType) {
        //
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Adapter;
impl DukaAdapter for Adapter {
    type InputType = DukaChunk;

    fn adapt(&self, chunk: &mut Self::InputType) {
        transform(&mut MeaninglessTransformer::new(), chunk);
        transform(&mut DesugarTransformer::new(), chunk);
        transform(&mut ConstFoldTransformer::new(), chunk);
    }
}

/// Immutable check
pub fn check<V: Visitor>(visitor: &mut V, input: &DukaChunk) -> Vec<DukaSpannedError> {
    input.visit(visitor);
    visitor.report().collect()
}
/// Mutable transform
pub fn transform<V: VisitorMut>(visitor_mut: &mut V, input: &mut DukaChunk) {
    input.visit_mut(visitor_mut);
}
