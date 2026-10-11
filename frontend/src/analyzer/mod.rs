pub mod attributes;
pub mod builtin;
pub mod eval;
pub mod modules;
pub mod objects;
pub mod prelude;
pub mod tcx;
pub mod typechecker;
pub mod tyval;
pub mod visitors;

use duka_shared::{
    config::DukaAnalyzerConfig,
    constants::{catt, ccallish, cpar, ctype},
    dtype::{FunctionType, Type},
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{DukaAdapter, DukaAnalyzer, SourceInfo},
    utils::{ScopeType, SymbolTable},
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub use eval::TypeEval;
pub use modules::{
    DukaSourceProvider, ModuleBuildCache, ModuleMap, ModuleType, build_module_types,
    build_module_types_cached,
};
pub mod destructuring;
pub use typechecker::TypeChecker;
pub use tyval::TypeClosure;
pub use tyval::TypeValue;

/// A stage of the type system.
///
/// Separate from `DukaAnalyzer` because an analyzer is handed what the stage
/// before it produced, and a type checker also needs the tree.
pub trait DukaTypeChecker {
    type OutputData;

    fn check(
        &self,
        chunk: &crate::parser::ast::DukaChunk,
        data: AnalyzerData,
        provider: &dyn DukaSourceProvider,
    ) -> (Self::OutputData, Vec<DukaSpannedError>);
}

impl DukaTypeChecker for TypeChecker {
    type OutputData = AnalyzerData;

    fn check(
        &self,
        chunk: &crate::parser::ast::DukaChunk,
        data: AnalyzerData,
        provider: &dyn DukaSourceProvider,
    ) -> (AnalyzerData, Vec<DukaSpannedError>) {
        // The provider is what lets a `RequireType("m")` be resolved here rather
        // than only afterwards: a module name is relative, so a name that is not
        // yet a key in the map cannot be found without asking, and asking without
        // one made a chained `RequireType("m").A.B` an error rather than a type.
        let (data, errors) = TypeEval.analyze_with_provider(chunk, data, Some(provider));
        let errors: Vec<_> = errors.collect();
        let (data, errs) = self.analyze_with_modules(chunk, data, Some(provider));
        (data, errors.into_iter().chain(errs).collect())
    }
}

use crate::{
    analyzer::visitors::{
        CalcFoldTransformer, DesugarTransformer, ExportDesugarer, LabelChecker, LoopChecker,
        MeaninglessTransformer, VarArgChecker,
    },
    parser::ast::{
        Block, DukaChunk, Expr, FuncBody, IfClause, Match, MatchClause, Node, NodeKind,
        ObjectProperty, Param, Path, Stmt, TypeDesc, TypeParam, has_attr,
    },
};

pub use objects::{
    DeclaredTypeParam, MethodLink, ObjectMember, ObjectMethod, ObjectType, declared_type_params,
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
    fn visit_node(&mut self, _node: &Node) {}
    fn visit_stmt(&mut self, _stmt: &Stmt) {}
    fn visit_expr(&mut self, _expr: &Expr) {}
    /// A name written in a type position, reached while walking a type.
    ///
    /// The derived visitor skips whole fields rather than parts of one, and a
    /// `Named` is a name and a span -- neither of which is a node -- so the walk
    /// cannot step into it. This is the one place a type hands a name out, which
    /// is what lets the checker see every name in a type without walking the shape
    /// by hand.
    fn visit_type_name(&mut self, _name: &str, _span: &Span) {}
    fn before(&mut self) {}
    fn after(&mut self) {}
    fn visit_if_clause_block(&mut self, _block: &IfClause, _enter: bool) {}
    fn visit_match_else_block(&mut self, _block: &Match, _enter: bool) {}
    fn visit_match_clause_block(&mut self, _block: &MatchClause, _enter: bool) {}
    fn visit_func_block(&mut self, _block: &FuncBody, _enter: bool) {}
    fn visit_do_stmt_block(&mut self, _block: &NodeKind, _enter: bool) {}
    fn visit_do_expr_block(&mut self, _block: &NodeKind, _enter: bool) {}
    fn visit_loop_stmt_block(&mut self, _block: &NodeKind, _enter: bool) {}
    fn visit_block(&mut self, _enter: bool) {}

    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
}
pub trait VisitorMut {
    fn visit_node(&mut self, _node: &mut Node) {}
    fn visit_stmt(&mut self, _stmt: &mut Stmt) {}
    fn visit_expr(&mut self, _expr: &mut Expr) {}
    /// The mutable half of [`Visitor::visit_type_name`]. A name is not a node, so
    /// there is nothing to descend into -- the hook exists so that a rewrite can
    /// see one, and a name that should change changes here.
    fn visit_type_name(&mut self, _name: &mut Box<str>, _span: &mut Span) {}
    /// Notice, if you are going to override this function, you need to handle deeply visiting manually
    fn visit_block(&mut self, block: &mut Block)
    where
        Self: Sized,
    {
        for b in block.0.iter_mut() {
            b.visit_mut(self);
        }
    }
    fn before(&mut self) {}
    fn after(&mut self) {}

    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        std::iter::empty()
    }
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
pub type CallResults = Vec<(Box<str>, Box<[TypeValue]>, TypeValue)>;

#[derive(Debug, Clone)]
pub struct TypeFn {
    pub name: Box<str>,
    pub body: Box<FuncBody>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct InlineTypeFn {
    pub name: Box<str>,
    pub params: Box<[Param]>,
    pub ret_ty: TypeDesc,
    pub span: Span,
}

#[derive(Debug, Default)]
pub struct ScopeAnalysis {
    pub symbols: SymbolTable,
    pub objects: Vec<ObjectType>,
    pub type_fns: Vec<TypeFn>,
    pub inline_type_fns: Vec<InlineTypeFn>,
    pub aliases: Vec<(Box<str>, TypeDesc)>,
    pub type_results: CallResults,
    pub call_cache: Arc<Mutex<CallResults>>,
    /// type function bodies: Type::TypeFn.id indexes this
    /// A type function has to be able to sit inside a type -- a record field, an
    /// array element, a type argument -- and `Type` lives in `shared` while
    /// `FuncBody` lives here, so the type names the body by its position in this
    /// table. It is shaped like `call_cache` and for the same reason: both are
    /// per analysis, so an id read back through a cached analysis is only
    /// meaningful in the analysis that produced it.
    pub closures: Arc<Mutex<Vec<TypeClosure>>>,
    pub links: Vec<MethodLink>,
    pub uses: HashMap<Span, usize>,
    /// what each type parameter of a generic call was solved to, plus the
    /// bound it was declared with, so a language server can show `T = int`
    /// instead of only the substituted signature
    pub generic_bindings: Vec<(Span, Vec<GenericBinding>)>,
    /// The type parameter list each generic declaration was written with, keyed
    /// by the span of the name it was declared under.
    ///
    /// The symbol table keeps a parameter per name, which cannot answer `<T,
    /// ...V>`: order is part of what was written, and so is the `...`. Recording
    /// the list once is what lets the editor render a declaration as written
    /// instead of reading the text back and reading it apart again.
    pub declared_type_params: HashMap<Span, Box<[DeclaredTypeParam]>>,
    /// What a `---` documented, keyed by the span of what it documented.
    ///
    /// Kept beside the other recorded facts about a declaration rather than read
    /// off the source: the text is already matched to the thing it belongs to, and
    /// a reader asking about a name must not have to answer the question of which
    /// comment near it was meant for it.
    pub docs: HashMap<Span, String>,
    /// What each name written as a member of a type resolved to, keyed by where
    /// that name is written.
    ///
    /// A member read in a type position is a reference like any other -- it
    /// names something, here usually something in another file, and binds
    /// nothing -- and the pass that resolved it is the only one that knows what.
    /// So it is recorded while it is resolved rather than left for a reader to
    /// work out again: a language server cannot evaluate a type by itself, and
    /// `RequireType("m").Alias` is not something a symbol table answers.
    pub type_at_span: HashMap<Span, Arc<Type>>,
    pub modules: ModuleMap,
}

/// What a name in a generic call's answer is, which decides where it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingKind {
    /// a slot the call solved for
    Parameter,
    /// `where type C = E`
    TypeBinding,
    /// `where Sized(T)`
    Concept,
}

/// One solved type parameter of a generic call
#[derive(Debug, Clone)]
pub struct GenericBinding {
    pub name: Box<str>,
    pub value: Box<str>,
    pub bound: Option<Box<str>>,
    pub kind: BindingKind,
    /// `value` still structured, so a hover can substitute into a type.
    pub value_type: Option<std::sync::Arc<duka_shared::dtype::Type>>,
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

        /// A type-level expression can compute a record, so its fields are
        /// fields of this alias too. The visitor stores a `TypeDesc` by value
        /// because its callback cannot hand back a borrow of what it was given.
        struct CollectRecords(Vec<TypeDesc>);
        impl Visitor for CollectRecords {
            fn visit_node(&mut self, expr: &Expr) {
                if let NodeKind::TypeLit(td) = &expr.0 {
                    self.0.push(td.clone());
                }
            }
        }

        impl ScopeVisitor {
            #[inline]
            fn check_name(&mut self, name: &str, span: Span, type_context: bool) {
                let error = if type_context {
                    name == ctype::REQUIRE
                } else {
                    cpar::PRESERVED.contains(&name) || ccallish::CALLISHES.contains(&name)
                };
                if error {
                    self.2.push(DukaSpannedError {
                        kind: DukaSemanticError::PreservedName(name.into()).into(),
                        level: Default::default(),
                        span,
                        source_info: self.1.clone(),
                        related: [].into(),
                    });
                }
            }

            /// Keeps on the symbol what the attributes written on the declaration
            /// say about it.
            ///
            /// Whatever they mean for the type has already been acted on by the
            /// time a symbol exists, so what is kept here is only what a reader
            /// needs: a hover, a completion, or an importer on the other side of a
            /// `require` asking what it was told. Every form of declaration goes
            /// through here, so a name declared `@declare` reads the same way
            /// whether it is a function, a value or an object.
            fn record_attribute(&mut self, id: usize, attrs: &crate::parser::ast::Attrs) {
                if let Some(attribute) = attributes::first(attrs) {
                    self.0.symbols.set_attribute(id, attribute);
                }
            }

            /// Keeps the type parameter list a declaration was written with, so
            /// a reader can render `<T, ...V>` without reading the text back.
            fn record_type_param_list(&mut self, span: Span, body: &crate::parser::ast::FuncBody) {
                if body.1.is_empty() {
                    return;
                }
                self.0
                    .declared_type_params
                    .insert(span, declared_type_params(body));
            }

            /// Declares every record literal reachable from an annotation as a
            /// field of `owner`. A field is a member, so it takes no part in
            /// value resolution; the span it is written at is what lets a hover
            /// on the declaration read the field instead of guessing from the
            /// shape of the tokens around it.
            fn declare_record_fields(&mut self, ty: &TypeDesc, owner: usize) {
                match ty {
                    TypeDesc::TypeTable(fields) => {
                        for field in fields.iter() {
                            self.0.symbols.declare_field(
                                field.key.as_ref(),
                                field.span,
                                field.ty.as_type(),
                                owner,
                            );
                            self.declare_record_fields(&field.ty, owner);
                        }
                    }
                    TypeDesc::Array(inner) => {
                        if let Some(inner) = inner.as_deref() {
                            self.declare_record_fields(inner, owner);
                        }
                    }
                    TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                        self.declare_record_fields(inner, owner)
                    }
                    TypeDesc::Table(k, v) => {
                        for inner in [k, v].into_iter().flatten() {
                            self.declare_record_fields(inner, owner);
                        }
                    }
                    TypeDesc::Union(ts) | TypeDesc::TypeTuple(ts) => {
                        for inner in ts.iter() {
                            self.declare_record_fields(inner, owner);
                        }
                    }
                    TypeDesc::Generic(_, args, _) => {
                        for inner in args.iter() {
                            self.declare_record_fields(inner, owner);
                        }
                    }
                    TypeDesc::Function(ft) => {
                        let Some(ft) = ft.as_ref() else { return };
                        for inner in ft.params.iter().chain(ft.returns.iter()) {
                            self.declare_record_fields(inner, owner);
                        }
                    }
                    TypeDesc::FnLit(body) => {
                        for p in body.0.iter() {
                            if let Param::Typed(_, t) = p {
                                self.declare_record_fields(t, owner);
                            }
                        }
                        if let Some(ret) = body.2.as_ref() {
                            for t in ret.tys.iter() {
                                self.declare_record_fields(t, owner);
                            }
                        }
                    }
                    // a type-level expression can compute a record, so its
                    // fields are fields of this alias too
                    TypeDesc::Expr(expr) => {
                        let mut collect = CollectRecords(vec![]);
                        expr.visit(&mut collect);
                        for td in collect.0.iter() {
                            self.declare_record_fields(td, owner);
                        }
                    }
                    TypeDesc::Pure(_) | TypeDesc::Named(..) | TypeDesc::TypeOf(..) => (),
                }
            }
        }

        impl Visitor for ScopeVisitor {
            fn visit_node(&mut self, stmt: &Stmt) {
                match stmt.0 {
                    NodeKind::Label(ref lab) => {
                        if let Err(last_span) = self.0.symbols.declare_label(lab.0.as_str(), lab.1)
                        {
                            self.2.push(DukaSpannedError {
                                kind: DukaSemanticError::DuplicatedItem(
                                    "label".into(),
                                    lab.0.as_str().into(),
                                )
                                .into(),
                                level: Default::default(),
                                span: lab.1,
                                related: [("it was already declared here".into(), last_span)]
                                    .into(),
                                source_info: self.1.clone(),
                            });
                        }
                    }
                    NodeKind::Goto(ref lab) => {
                        // the label a goto names belongs to a function, so the
                        // goto is recorded in that function's scope beside the
                        // labels it may land on
                        self.0.symbols.declare_goto(lab.0.as_str(), lab.1);
                    }
                    NodeKind::Assign(ref names, ..) => {
                        for name in names {
                            let key = name.to_string().into_boxed_str();

                            if self.0.symbols.lookup(&key).is_some() {
                                continue;
                            }

                            let span = name.get_span();
                            self.check_name(&key, span, false);
                            self.0
                                .symbols
                                .declare_variable(key, span, !self.3.var_default_local);
                        }
                    }
                    NodeKind::FunctionDecl(ref name, ref attrs, ref body, global) => {
                        let key = name.to_string().into_boxed_str();
                        let span = name.get_span();
                        self.check_name(&key, span, false);
                        let id = self.0.symbols.declare_function(key, span, global);
                        self.record_attribute(id, attrs);
                        self.record_type_param_list(span, body);
                    }
                    NodeKind::Define(ref names, ref exprs, global, _) => {
                        for (idx, (((key, span), attrs, _ty), _)) in names.iter().enumerate() {
                            self.check_name(key, *span, false);
                            let id = if !global
                                && has_attr(attrs, catt::CONST)
                                && let Some(Node(NodeKind::Literal(cv), span)) = exprs.get(idx)
                            {
                                self.0
                                    .symbols
                                    .declare_constant(key.as_str(), *span, cv.clone())
                            } else {
                                self.0.symbols.declare_variable(key.as_str(), *span, global)
                            };
                            self.record_attribute(id, attrs);
                        }
                    }
                    NodeKind::Destructing(ref names, _, global) => {
                        // A destructuring is its own statement, and a name it binds
                        // is a binding like any other. Without this the names are in
                        // no scope at all until the desugarer has read the pattern
                        // apart, which is why a reader saw no `a` in `local {a, b}`
                        // at all rather than an `a` of the wrong type.
                        for name in crate::analyzer::destructuring::names(names) {
                            self.check_name(&name.0, name.1, global);
                            self.0
                                .symbols
                                .declare_variable(name.0.as_str(), name.1, global);
                        }
                    }
                    NodeKind::TypeAlias(ref name, ref ty) => {
                        let (key, span) = name;
                        self.check_name(key, *span, true);
                        let id = self.0.aliases.len();
                        self.0
                            .aliases
                            .push((key.clone().into_boxed_str(), (**ty).clone()));
                        let owner = self.0.symbols.declare_type_alias(key.as_str(), *span, id);
                        self.declare_record_fields(ty, owner);
                    }
                    NodeKind::TypeFunction(ref name, ref body) => {
                        let (key, span) = name;
                        self.check_name(key, *span, true);
                        self.record_type_param_list(*span, body);
                        let id = self.0.type_fns.len();
                        self.0.type_fns.push(TypeFn {
                            name: key.clone().into_boxed_str(),
                            body: body.clone(),
                            span: *span,
                        });
                        self.0
                            .symbols
                            .declare_type_function(key.as_str(), *span, id);
                    }
                    NodeKind::InlineTypeFunction(ref name, ref params, ref ret_ty) => {
                        let (key, span) = name;
                        self.check_name(key, *span, true);
                        let id = self.0.inline_type_fns.len();
                        self.0.inline_type_fns.push(InlineTypeFn {
                            name: key.clone().into_boxed_str(),
                            params: params.clone(),
                            ret_ty: ret_ty.as_ref().clone(),
                            span: *span,
                        });
                        self.0
                            .symbols
                            .declare_inline_type_function(key.as_str(), *span, id);
                    }
                    NodeKind::Object(ref od) => {
                        let id = self.0.objects.len();
                        let name = od.name.0.clone().into_boxed_str();
                        let decl_span = od.name.1;
                        self.check_name(&name, decl_span, false);
                        let owner = self.0.symbols.declare_object_class(
                            name.clone(),
                            decl_span,
                            od.global,
                            id,
                        );
                        self.record_attribute(owner, &od.attrs);
                        let mut members = vec![];
                        for p in od.properties.iter() {
                            let ObjectProperty::NameValue(n, _, ty) = p else {
                                continue;
                            };
                            let ty = ty.clone().unwrap_or(TypeDesc::Pure(Type::Any));
                            self.0
                                .symbols
                                .declare_field(n.0.as_str(), n.1, ty.as_type(), owner);
                            members.push(ObjectMember {
                                name: n.0.clone().into_boxed_str(),
                                ty,
                                span: n.1,
                            });
                        }
                        let mut methods = vec![];
                        for (name, _, body) in od.static_methods.iter() {
                            self.check_name(&name.0, name.1, false);
                            methods.push(ObjectMethod {
                                name: name.0.clone().into_boxed_str(),
                                sig: method_sig(body),
                                span: name.1,
                                is_static: true,
                            });
                        }
                        for (name, _, body) in od.methods.iter() {
                            self.check_name(&name.0, name.1, false);
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
                            type_params: od
                                .type_params
                                .iter()
                                .map(|TypeParam((n, _), ..)| n.clone().into_boxed_str())
                                .collect(),
                            members: members.into(),
                            methods: methods.into(),
                            decl_span: stmt.1,
                        });
                    }
                    NodeKind::Access(ref path) => {
                        if let Some((name, span)) = path_deref_name(path.as_ref())
                            && let Some(symbol) = self.0.symbols.lookup(name)
                        {
                            self.0.uses.insert(span, symbol.id);
                        }
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
                    // `<T: num>` declares a type slot of this function. The body
                    // can read the name, but only in a type position, so it is
                    // declared as a type parameter rather than as a local.
                    for TypeParam((name, span), bound, default, _) in block.1.iter() {
                        self.0.symbols.declare_type_param(
                            name.as_str(),
                            *span,
                            bound.as_ref().map(|b| b.to_string().into_boxed_str()),
                            default.as_ref().map(|d| d.to_string().into_boxed_str()),
                        );
                    }
                    // `type V = E` introduces a type, so it is declared here.
                    // `T: B` and `Concept(T)` name types that are declared
                    // elsewhere -- a type parameter, a local, a parameter -- so
                    // they are references to those and are left alone. A binding
                    // declared by a clause is recorded before the signature is
                    // read, which is why `where type C = T` can be used in the
                    // return annotation above it.
                    for clause in block.3.iter() {
                        if let crate::parser::ast::WhereClause::Bind(names, value, _) = clause {
                            // A destructuring binding introduces a type per name,
                            // each standing for the same source, so a hover on
                            // any of them answers the same way.
                            let source = value.to_string().into_boxed_str();
                            for name in crate::analyzer::destructuring::names(names) {
                                self.0.symbols.declare_type_binding(
                                    name.0.as_str(),
                                    name.1,
                                    source.clone(),
                                );
                            }
                        }
                    }
                    for param in block.0.iter() {
                        match param {
                            Param::Typed((name, span), ty) => {
                                self.0.symbols.declare_parameter(
                                    name.as_str(),
                                    *span,
                                    ty.as_type(),
                                );
                            }
                            Param::Name((name, span)) => {
                                self.0
                                    .symbols
                                    .declare_parameter(name.as_str(), *span, Type::Any);
                            }
                            Param::Var(_) => {}
                        }
                    }
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_do_stmt_block(&mut self, _block: &NodeKind, enter: bool) {
                if enter {
                    self.0.symbols.enter(ScopeType::Normal);
                } else {
                    self.0.symbols.exit();
                }
            }
            fn visit_do_expr_block(&mut self, _block: &NodeKind, enter: bool) {
                self.visit_do_stmt_block(_block, enter);
            }
            fn visit_loop_stmt_block(&mut self, _block: &NodeKind, enter: bool) {
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
        // What a `---` documented, read here because the parser is the only thing
        // that saw the comment and the only thing that could say which declaration
        // it was about. Nothing downstream reads the text off the token stream.
        analysis.docs = chunk.docs.clone();
        resolve_bases(
            &mut analysis,
            chunk.source_info.clone().into(),
            &mut visitors.2,
        );
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
        if let Some((name, span)) = obj.base_ref.clone() {
            match by_name.get(&name) {
                Some(kind) if *kind != i => obj.base = Some(*kind),
                _ => errors.push(DukaSpannedError {
                    kind: DukaSemanticError::UnknownBase(name).into(),
                    span,
                    level: Default::default(),
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
                level: Default::default(),
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

fn path_deref_name(path: &Path) -> Option<(&str, Span)> {
    match path {
        Path::Chain(base, _) => path_deref_name(base),
        Path::Expr(_) => None,
        Path::Base((name, span)) => Some((name.as_str(), *span)),
    }
}

fn method_sig(body: &FuncBody) -> FunctionType {
    FunctionType {
        params: body
            .named_params()
            .map(|(_, ty)| ty.map(TypeDesc::as_type).unwrap_or(Type::Any))
            .collect(),
        var_arg: body.has_var_arg(),
        returns: body
            .2
            .as_ref()
            .map(|r| {
                r.tys
                    .iter()
                    .filter_map(|t| t.base_type().cloned())
                    .collect()
            })
            .unwrap_or_default(),
        return_var_arg: body.2.as_ref().is_some_and(|r| r.var_arg),
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

    fn adapt(&self, _: &mut Self::InputType) -> Vec<DukaSpannedError> {
        //
        vec![]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Adapter;
impl DukaAdapter for Adapter {
    type InputType = DukaChunk;

    fn adapt(&self, chunk: &mut Self::InputType) -> Vec<DukaSpannedError> {
        let mut a = transform(
            &mut MeaninglessTransformer::new(chunk.source_info.clone()),
            chunk,
        );
        let b = transform(
            &mut DesugarTransformer::new(chunk.source_info.clone()),
            chunk,
        );
        ExportDesugarer::new().run(chunk);
        let c = transform(
            &mut CalcFoldTransformer::new(chunk.source_info.clone()),
            chunk,
        );
        a.extend(b);
        a.extend(c);
        a
    }
}

/// Immutable check
pub fn check<V: Visitor>(visitor: &mut V, input: &DukaChunk) -> Vec<DukaSpannedError> {
    input.visit(visitor);
    visitor.report().collect()
}
/// Mutable transform
pub fn transform<V: VisitorMut>(
    visitor_mut: &mut V,
    input: &mut DukaChunk,
) -> Vec<DukaSpannedError> {
    input.visit_mut(visitor_mut);
    visitor_mut.report().collect()
}
