pub mod visitors;

use duka_shared::{
    error::DukaSpannedError,
    types::{DukaAdapter, DukaAnalyzer, DukaChunk, Visit, VisitMut, Visitor, VisitorMut},
};

use crate::analyzer::visitors::{
    ConstFoldTransformer, DesugarTransformer, LabelChecker, LoopChecker, MeaninglessTransformer,
    VarArgChecker,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EmptyAnalyzer;
impl DukaAnalyzer for EmptyAnalyzer {
    type InputType = DukaChunk;
    fn analyze(&self, _: &Self::InputType) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analyzer;
impl DukaAnalyzer for Analyzer {
    type InputType = DukaChunk;

    fn analyze(&self, chunk: &Self::InputType) -> impl Iterator<Item = DukaSpannedError> {
        check(&mut LabelChecker::new(), chunk)
            .into_iter()
            .chain(check(&mut LoopChecker::new(), chunk))
            .chain(check(&mut VarArgChecker::new(), chunk))
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
        transform(&mut ConstFoldTransformer::new(), chunk);
        transform(&mut MeaninglessTransformer::new(), chunk);
        transform(&mut DesugarTransformer::new(), chunk);
    }
}

/// Immutable check
pub fn check<V: Visitor>(visitor: &mut V, input: &DukaChunk) -> Vec<DukaSpannedError> {
    input.chunk.visit(visitor);
    visitor.report().collect()
}
/// Mutable transform
pub fn transform<V: VisitorMut>(visitor_mut: &mut V, input: &mut DukaChunk) {
    input.chunk.visit_mut(visitor_mut);
}
