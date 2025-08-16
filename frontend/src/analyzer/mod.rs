pub mod visitors;

use duka_shared::{
    ast::Block,
    error::DukaError,
    types::{DukaAdapter, DukaAnalyzer, Visit, VisitMut, Visitor, VisitorMut},
};

use crate::analyzer::visitors::{
    ConstFoldTransformer, DesugarTransformer, LabelChecker, LoopChecker, MeaninglessTransformer,
    VarArgChecker,
};

pub struct Analyzer;
impl DukaAnalyzer for Analyzer {
    type InputType = Block;

    fn analyze(self, chunk: &Block) -> Vec<DukaError> {
        let mut res = vec![];
        res.extend(check(&mut LabelChecker::new(), chunk));
        res.extend(check(&mut LoopChecker::new(), chunk));
        res.extend(check(&mut VarArgChecker::new(), chunk));
        res
    }
}

pub struct Adapter;
impl DukaAdapter for Adapter {
    type InputType = Block;

    fn adapt(self, chunk: &mut Block) {
        transform(&mut ConstFoldTransformer::new(), chunk);
        transform(&mut MeaninglessTransformer::new(), chunk);
        transform(&mut DesugarTransformer::new(), chunk);
    }
}

pub fn check<V: Visitor>(visitor: &mut V, chunk: &Block) -> Vec<DukaError> {
    chunk.visit(visitor);
    visitor.report()
}
pub fn transform<V: VisitorMut>(visitor_mut: &mut V, chunk: &mut Block) {
    chunk.visit_mut(visitor_mut);
}
