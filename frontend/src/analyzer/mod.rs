pub mod visitors;

use duka_shared::{
    ast::Block,
    error::DukaError,
    types::{DukaAdapter, DukaAnalyzer, Visit, VisitMut, Visitor, VisitorMut},
};

use crate::analyzer::visitors::{LabelChecker, LoopChecker, VarArgChecker};

pub struct Analyzer;
impl DukaAnalyzer for Analyzer {
    type InputType = Block;

    fn analyze(mut self, chunk: &Block) -> Vec<DukaError> {
        vec![]
    }
}

pub struct Adapter;
impl DukaAdapter for Adapter {
    type InputType = Block;

    fn adapt(mut self, chunk: &mut Block) {}
}

pub fn check<V: Visitor>(visitor: &mut V, chunk: &Block) -> Vec<DukaError> {
    chunk.visit(visitor);
    visitor.report()
}
pub fn transform<V: VisitorMut>(visitor_mut: &mut V, chunk: &mut Block) {
    chunk.visit_mut(visitor_mut);
}
