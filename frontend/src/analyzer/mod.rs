pub mod visitors;

use duka_shared::{
    error::DukaSpannedError,
    types::{DukaAdapter, DukaAnalyzer, DukaChunk, Visit, VisitMut, Visitor, VisitorMut},
};

use crate::analyzer::visitors::{
    ConstFoldTransformer, DesugarTransformer, LabelChecker, LoopChecker, MeaninglessTransformer,
    VarArgChecker,
};

pub struct Analyzer;
impl DukaAnalyzer for Analyzer {
    type InputType = DukaChunk;

    fn analyze(self, chunk: &Self::InputType) -> Vec<DukaSpannedError> {
        let mut res = vec![];
        res.extend(check(&mut LabelChecker::new(), chunk));
        res.extend(check(&mut LoopChecker::new(), chunk));
        res.extend(check(&mut VarArgChecker::new(), chunk));
        res
    }
}

pub struct Adapter;
impl DukaAdapter for Adapter {
    type InputType = DukaChunk;

    fn adapt(self, chunk: &mut Self::InputType) {
        transform(&mut ConstFoldTransformer::new(), chunk);
        transform(&mut MeaninglessTransformer::new(), chunk);
        transform(&mut DesugarTransformer::new(), chunk);
    }
}

/// Immutable check
pub fn check<V: Visitor>(visitor: &mut V, input: &DukaChunk) -> Vec<DukaSpannedError> {
    input.chunk.visit(visitor);
    visitor.report()
}
/// Mutable transform
pub fn transform<V: VisitorMut>(visitor_mut: &mut V, input: &mut DukaChunk) {
    input.chunk.visit_mut(visitor_mut);
}
