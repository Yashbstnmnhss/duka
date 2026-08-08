pub mod objects;
pub mod typechecker;
pub mod visitors;

use duka_shared::{
    config::DukaAnalyzerConfig,
    constants::catt,
    dtype::{FunctionType, Type},
    errors::{DukaSemanticError, DukaSpannedError},
    types::{DukaAdapter, DukaAnalyzer, SourceInfo},
    utils::{ScopeType, SymbolTable},
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use typechecker::TypeChecker;

use crate::{
    analyzer::visitors::{
        ConstFoldTransformer, DesugarTransformer, ExportDesugarer, LabelChecker, LoopChecker,
        MeaninglessTransformer, VarArgChecker,
    },
    parser::ast::{
        DukaChunk, Expr, ExprKind, FuncBody, IfClause, Match, MatchClause, ObjectProperty, Param,
        Stmt, StmtKind, has_attr,
    },
};

pub use objects::{MethodLink, ObjectMember, ObjectMethod, ObjectType};

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
    fn visit_block(&mut self, _enter: bool) {}

    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
}
pub trait VisitorMut {
    fn visit_stmt(&mut self, _stmt: &mut Stmt) {}
    fn visit_expr(&mut self, _expr: &mut Expr) {}

    fn visit_block(&mut self, _enter: bool) {}
    fn before(&mut self) {}
    fn after(&mut self) {}
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmptyAnalyzer;
impl DukaAnalyzer for EmptyAnalyzer {
    type InputType = DukaChunk;
    type InputData = DukaAnalyzerConfig;
    type OutputData = ();
    fn analyze(
        &self,
        _: &Self::InputType,
        _: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        ((), std::iter::empty())
    }
}

pub type AnalyzerData = (DukaAnalyzerConfig, ScopeAnalysis);

#[derive(Debug, Default)]
pub struct ScopeAnalysis {
    pub symbols: SymbolTable,
    pub objects: Vec<ObjectType>, // 由objectid访问 这个仅是编译期的
    pub links: Vec<MethodLink>,   //用于LSP提示
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeAnalyzer;
impl DukaAnalyzer for ScopeAnalyzer {
    type InputType = DukaChunk;
    type InputData = DukaAnalyzerConfig;
    type OutputData = AnalyzerData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        config: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        struct ScopeVisitor(
            ScopeAnalysis,
            Arc<SourceInfo>,
            Vec<DukaSpannedError>,
            DukaAnalyzerConfig,
        );
        impl Visitor for ScopeVisitor {
            fn visit_stmt(&mut self, stmt: &Stmt) {
                match stmt.0 {
                    StmtKind::Label(ref lab) => {
                        if let Err(last_span) = self.0.symbols.declare_label(lab.as_str(), stmt.1) {
                            self.2.push(DukaSpannedError {
                                kind: DukaSemanticError::DuplicatedItem(
                                    "label".into(),
                                    lab.as_str().into(),
                                )
                                .into(),
                                span: stmt.1,
                                related: [("it was already declared here".into(), last_span)]
                                    .into(),
                                source_info: self.1.clone(),
                            });
                        }
                    }
                    StmtKind::Assign(ref names, ..) => {
                        for name in names {
                            let key = name.to_string().into_boxed_str();

                            if self.0.symbols.lookup(&key).is_some() {
                                continue;
                            }

                            let span = name.get_span();
                            self.0
                                .symbols
                                .declare_variable(key, span, !self.3.var_default_local);
                        }
                    }
                    StmtKind::Function(ref name, .., global) => {
                        let key = name.to_string().into_boxed_str();
                        let span = name.get_span();
                        self.0.symbols.declare_function(key, span, global);
                    }
                    StmtKind::Define(ref names, ref exprs, global) => {
                        for (idx, (((key, span), attrs, _ty), _)) in names.iter().enumerate() {
                            if !global
                                && has_attr(attrs, catt::CONST)
                                && let Some(Expr(ExprKind::Literal(cv), span)) = exprs.get(idx)
                            {
                                self.0
                                    .symbols
                                    .declare_constant(key.as_str(), *span, cv.clone());
                                continue;
                            }
                            self.0.symbols.declare_variable(key.as_str(), *span, global);
                        }
                    }
                    StmtKind::Object(ref od) => {
                        let id = self.0.objects.len();
                        let name = od.name.0.clone().into_boxed_str();
                        let decl_span = od.name.1;
                        self.0
                            .symbols
                            .declare_object_class(name.clone(), decl_span, od.global, id);
                        let members = od
                            .properties
                            .iter()
                            .filter_map(|p| match p {
                                ObjectProperty::NameValue(n, _) => Some(ObjectMember {
                                    name: n.0.clone().into_boxed_str(),
                                    ty: Type::Any,
                                    span: n.1,
                                }),
                                ObjectProperty::KeyValue(..) => None,
                            })
                            .collect();
                        let mut methods = Vec::new();
                        for (name, _, body) in od.static_methods.iter() {
                            methods.push(ObjectMethod {
                                name: name.0.clone().into_boxed_str(),
                                sig: method_sig(body),
                                span: name.1,
                                is_static: true,
                            });
                        }
                        for (name, _, body) in od.methods.iter() {
                            methods.push(ObjectMethod {
                                name: name.0.clone().into_boxed_str(),
                                sig: method_sig(body),
                                span: name.1,
                                is_static: false,
                            });
                        }
                        self.0.objects.push(ObjectType {
                            name,
                            global: od.global,
                            base: None,
                            base_ref: od
                                .base
                                .as_ref()
                                .map(|(b, sp)| (b.clone().into_boxed_str(), *sp)),
                            members,
                            methods: methods.into(),
                            decl_span: stmt.1,
                        });
                    }
                    _ => (),
                }
            }
            fn visit_if_clause_block(&mut self, _block: &IfClause, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_match_else_block(&mut self, _block: &Match, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_match_clause_block(&mut self, _block: &MatchClause, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_func_block(&mut self, block: &FuncBody, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Function);
                    for param in block.0.iter() {
                        match param {
                            Param::Typed((name, span), _) | Param::Name((name, span)) => {
                                self.0.symbols.declare_variable(name.as_str(), *span, false);
                            }
                            Param::Var(_) => {}
                        }
                    }
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_do_stmt_block(&mut self, _block: &StmtKind, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_do_expr_block(&mut self, _block: &ExprKind, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_loop_stmt_block(&mut self, _block: &StmtKind, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Loop);
                } else {
                    self.0.symbols.exit();
                }
            }
        }

        let mut visitors = ScopeVisitor(
            ScopeAnalysis::default(),
            chunk.source_info.clone().into(),
            vec![],
            config.clone(),
        );
        chunk.visit(&mut visitors);
        let mut analysis = visitors.0;
        resolve_bases(&mut analysis, chunk.source_info.clone().into(), &mut visitors.2);
        ((config, analysis), visitors.2.into_iter())
    }
}

fn resolve_bases(
    analysis: &mut ScopeAnalysis,
    source: Arc<SourceInfo>,
    errors: &mut Vec<DukaSpannedError>,
) {
    let by_name: HashMap<Box<str>, usize> = analysis
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name.clone(), i))
        .collect();
    for (i, obj) in analysis.objects.iter_mut().enumerate() {
        let obj = obj;
        if let Some((name, span)) = obj.base_ref.clone() {
            match by_name.get(&name) {
                Some(kind) if *kind != i => obj.base = Some(*kind),
                _ => errors.push(DukaSpannedError {
                    kind: DukaSemanticError::UnknownBase(name).into(),
                    span,
                    related: [].into(),
                    source_info: source.clone(),
                }),
            }
        }
    }
    for i in 0..analysis.objects.len() {
        if has_cycle(&analysis.objects, i) {
            let class = &analysis.objects[i];
            errors.push(DukaSpannedError {
                kind: DukaSemanticError::CircularExtends(class.name.clone()).into(),
                span: class
                    .base_ref
                    .as_ref()
                    .map(|(_, sp)| *sp)
                    .unwrap_or(class.decl_span),
                related: [].into(),
                source_info: source.clone(),
            });
        }
    }
}

fn has_cycle(objects: &[ObjectType], start: usize) -> bool {
    let mut seen = HashSet::new();
    let mut cur = objects[start].base;
    while let Some(id) = cur {
        if !seen.insert(id) {
            return true;
        }
        cur = match objects.get(id) {
            Some(o) => o.base,
            None => break,
        };
    }
    false
}

fn method_sig(body: &FuncBody) -> FunctionType {
    FunctionType {
        params: body
            .0
            .iter()
            .map(|p| match p {
                Param::Typed(_, t) => t.clone(),
                _ => Type::Any,
            })
            .collect(),
        var_arg: body.has_var_arg(),
        returns: body.1.clone().into_iter().collect(),
        return_var_arg: false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasicAnalyzer;
impl DukaAnalyzer for BasicAnalyzer {
    type InputType = DukaChunk;
    type InputData = AnalyzerData;
    type OutputData = AnalyzerData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        let errors = check(
            &mut LabelChecker::new(chunk.source_info.clone(), &data),
            chunk,
        )
        .into_iter()
        .chain(check(
            &mut LoopChecker::new(chunk.source_info.clone(), &data),
            chunk,
        ))
        .chain(check(
            &mut VarArgChecker::new(chunk.source_info.clone(), &data),
            chunk,
        ));
        (data, errors)
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
        ExportDesugarer::new().run(chunk);
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
