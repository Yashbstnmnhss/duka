use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use duka_shared::constants::cpar;
use duka_shared::{
    constants::csugar,
    docs::Returns,
    dtype::{FunctionType, ObjectId, Type},
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::analyzer::{GenericBinding, ModuleType, attributes};
use crate::solver;
use crate::{
    analyzer::{
        AnalyzerData, CallResults, InlineTypeFn, TypeFn, Visit, Visitor,
        eval::{EvalCtx, EvalCtxInit},
        modules::DukaSourceProvider,
        objects::{MethodLink, ObjectMethod, ObjectType},
        tyval::{TypeClosure, TypeValue},
    },
    parser::ast::{
        Attrs, DestructingTerm, Destructuring, DukaChunk, Expr, ExprKind, Field, FuncBody, Name,
        Param, Path, PathSuffix, Stmt, StmtKind, TypeDesc, TypeFnValue, TypeParam, WhereClause,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub struct TypeChecker;

impl DukaAnalyzer for TypeChecker {
    type InputType = DukaChunk;
    type InputData = AnalyzerData;
    type OutputData = AnalyzerData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        self.analyze_with_modules(chunk, data, None)
    }
}

impl TypeChecker {
    /// Infers the value level shape of a module: the type of every `export`ed
    /// symbol plus the type `require` gives back to the importer
    pub fn analyze_module<'a>(
        &self,
        chunk: &'a DukaChunk,
        mut data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> (AnalyzerData, Vec<DukaSpannedError>, ModuleValue) {
        let source = Arc::new(chunk.source_info.clone());
        let inferred = Self::infer_returns(chunk, &data, provider);
        let mut ctx = TypeCheckerCtx::new(source, &data, provider);
        ctx.inferred_returns = inferred;
        chunk.visit(&mut ctx);
        let tail = chunk
            .block
            .1
            .as_deref()
            .and_then(|tail| ctx.infer_tail(tail));
        let mut links = std::mem::take(&mut ctx.links);
        let generic_bindings = std::mem::take(&mut ctx.generic_bindings);
        links.sort_by_key(|l| (l.call_span, l.name_span, l.decl_span, l.owner));
        links.dedup();
        let value = ctx.module_value(tail);

        let (errors, backfills) = ctx.finish();
        for (span, ty, value) in backfills {
            data.1.symbols.set_type_at_span(span, ty);
            if let Some(value) = value {
                data.1.symbols.set_type_value_at_span(span, Arc::new(value));
            }
        }
        data.1.links = links;
        data.1.generic_bindings = generic_bindings;
        (data, errors.collect(), value)
    }

    fn infer_returns<'a>(
        chunk: &'a DukaChunk,
        data: &AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> HashMap<Box<str>, InferredReturns> {
        let source = Arc::new(chunk.source_info.clone());
        let mut inferred: HashMap<Box<str>, InferredReturns> = HashMap::new();
        for _ in 0..MAX_INFER_ROUNDS {
            let mut collect = TypeCheckerCtx::new(source.clone(), data, provider);
            collect.collect_mode = true;
            collect.inferred_returns = inferred.clone();
            chunk.visit(&mut collect);
            if collect.collected_returns == inferred {
                break;
            }
            inferred = collect.collected_returns;
        }
        inferred
    }

    pub fn analyze_with_modules<'a>(
        &self,
        chunk: &'a DukaChunk,
        mut data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> (AnalyzerData, impl Iterator<Item = DukaSpannedError>) {
        let source = Arc::new(chunk.source_info.clone());
        let inferred = Self::infer_returns(chunk, &data, provider);
        let mut ctx = TypeCheckerCtx::new(source, &data, provider);
        ctx.inferred_returns = inferred;
        chunk.visit(&mut ctx);
        let mut links = std::mem::take(&mut ctx.links);
        let generic_bindings = std::mem::take(&mut ctx.generic_bindings);
        links.sort_by_key(|l| (l.call_span, l.name_span, l.decl_span, l.owner));
        links.dedup();

        let (errors, backfills) = ctx.finish();
        for (span, ty, value) in backfills {
            data.1.symbols.set_type_at_span(span, ty);
            if let Some(value) = value {
                data.1.symbols.set_type_value_at_span(span, Arc::new(value));
            }
        }
        data.1.links = links;
        data.1.generic_bindings = generic_bindings;

        (data, errors)
    }
}

type Multi = (Vec<Type>, bool);

const MAX_INFER_ROUNDS: usize = 64;

/// The value level shape of a module, what an importer gets from `require`
#[derive(Debug, Clone)]
pub struct ModuleValue {
    /// `require` hands this type back to the importer
    pub ty: Type,
    /// types of the symbols the module `export`s
    pub members: HashMap<Box<str>, Type>,
}
impl Default for ModuleValue {
    fn default() -> Self {
        Self {
            ty: Type::Table(None, None),
            members: HashMap::new(),
        }
    }
}

struct TypeCheckerCtx<'a> {
    source: Arc<SourceInfo>,
    ret_stack: Vec<Option<Multi>>,
    types: Vec<HashMap<Box<str>, Type>>,
    viewer: SymbolTableViewer<'a>,
    objects: &'a [ObjectType],
    aliases: &'a [(Box<str>, TypeDesc)],
    type_fns: &'a [TypeFn],
    inline_type_fns: &'a [InlineTypeFn],
    call_cache: Arc<Mutex<CallResults>>,
    closures: Arc<Mutex<Vec<TypeClosure>>>,
    modules: &'a HashMap<Box<str>, ModuleType>,
    provider: Option<&'a dyn DukaSourceProvider>,
    generic_fns: HashMap<Box<str>, GenericDecl>,
    links: Vec<MethodLink>,
    generic_bindings: Vec<(Span, Vec<GenericBinding>)>,
    errors: Vec<DukaSpannedError>,
    backfills: Vec<(Span, Box<str>, Option<Type>)>,
    collect_mode: bool,
    ret_collect: Vec<InferredReturns>,
    finished_returns: Vec<InferredReturns>,
    collected_returns: HashMap<Box<str>, InferredReturns>,
    inferred_returns: HashMap<Box<str>, InferredReturns>,
    final_args: bool,
    export_members: HashMap<Box<str>, Type>,
    /// The unknown type names already reported, so that a name reached twice --
    /// once building the signature and once checking the body -- is one mistake
    /// and not two.
    reported_unknown_types: Vec<(Box<str>, Span)>,
    /// `U: Point` from a `where`, kept next to the type parameters because it is
    /// What the current `where` list asked for that the declaration could not
    /// decide. Drained by the caller once the signature is built.
    obligations: Vec<Obligation>,
    /// The concepts that were deferred, kept so that the declaration pass does
    /// not read one a second time.
    concepts_deferred: Vec<Expr>,
}

/// Finds the type names hiding in a type-level expression, so they get the same
/// check as the ones written in the plain type shapes. The derived visitor
/// already walks the whole expression; all it stops short of is the `TypeDesc`
/// inside a `TypeLit`, which is the one place a type can be written there.
struct TypeNameWalk<'ctx, 'a>(&'ctx mut TypeCheckerCtx<'a>);

impl Visitor for TypeNameWalk<'_, '_> {
    fn visit_expr(&mut self, expr: &Expr) {
        if let ExprKind::TypeLit(td) = &expr.0 {
            self.0.check_type_names(td);
        }
    }
}

/// What the returns of a function without a return annotation work out to.
/// They come from several `return` statements, which line up position by
/// position rather than one after another: a function that returns `a` on one
/// path and `a, 1` on another returns `(int,)`, not `(int, int)`.
#[derive(Debug, Clone, PartialEq)]
struct InferredReturns {
    types: Box<[Type]>,
    /// The fewest values any single `return` handed back, against the most any
    /// of them did. A gap between the two means the tail is not always filled.
    shortest: usize,
    longest: usize,
    var_arg: bool,
}

impl Default for InferredReturns {
    fn default() -> Self {
        Self {
            types: Box::default(),
            shortest: usize::MAX,
            longest: 0,
            var_arg: false,
        }
    }
}

impl InferredReturns {
    /// Folds one `return` into what has been collected so far. A position this
    /// one leaves out keeps whatever the others say, and a position the others
    /// leave out stays unconstrained.
    fn merge(&mut self, incoming: &[Type]) {
        self.longest = self.longest.max(incoming.len());
        self.shortest = self.shortest.min(incoming.len());
        self.var_arg = self.shortest < self.longest;
        let mut types = self.types.to_vec();
        types.resize(self.longest, Type::Any);
        for (slot, ty) in types.iter_mut().zip(incoming) {
            if *slot == Type::Any {
                *slot = ty.clone();
            } else if *slot != *ty {
                *slot = slot.clone() | ty.clone();
            }
        }
        self.types = types.into_boxed_slice();
    }
}

impl<'a> TypeCheckerCtx<'a> {
    fn new(
        source: Arc<SourceInfo>,
        data: &'a AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> Self {
        Self {
            source,
            ret_stack: vec![None],
            types: vec![HashMap::new()],
            viewer: SymbolTableViewer::new(&data.1.symbols),
            objects: &data.1.objects,
            aliases: &data.1.aliases,
            type_fns: &data.1.type_fns,
            inline_type_fns: &data.1.inline_type_fns,
            call_cache: data.1.call_cache.clone(),
            closures: data.1.closures.clone(),
            modules: &data.1.modules,
            provider,
            generic_fns: HashMap::new(),
            generic_bindings: vec![],
            links: vec![],
            errors: vec![],
            backfills: vec![],
            collect_mode: false,
            ret_collect: vec![],
            finished_returns: vec![],
            collected_returns: HashMap::new(),
            inferred_returns: HashMap::new(),
            final_args: true,
            export_members: HashMap::new(),
            reported_unknown_types: Vec::new(),
            obligations: Vec::new(),
            concepts_deferred: Vec::new(),
        }
    }

    fn finish(
        self,
    ) -> (
        impl Iterator<Item = DukaSpannedError> + use<>,
        Vec<(Span, Box<str>, Option<Type>)>,
    ) {
        (self.errors.into_iter(), self.backfills)
    }

    fn err(&mut self, v: DukaSemanticError, span: Span) {
        if self.collect_mode {
            return;
        }
        self.errors.push(DukaSpannedError {
            level: Default::default(),
            kind: v.into(),
            span,
            related: [].into(),
            source_info: self.source.clone(),
        });
    }

    fn lookup_type(&self, name: &str) -> Option<Type> {
        for frame in self.types.iter().rev() {
            if let Some(t) = frame.get(name) {
                return Some(t.clone());
            }
        }
        let symbol = self.viewer.lookup(name)?;
        // whatever declared the name, the analysed type that went with it is
        // the answer: a constant's value, and a symbol that carries one, which
        // is what a builtin does
        if let SymbolType::Constant(cv) = &symbol.symbol_type {
            return Some(cv.type_of());
        }
        symbol.ty_value.as_deref().cloned()
    }

    fn declare(&mut self, name: &str, span: Span, ty: Type) {
        let st = (!self.collect_mode).then(|| ty.to_string().into_boxed_str());
        self.types
            .last_mut()
            .expect("there must be a type frame")
            .insert(name.into(), ty.clone());
        if let Some(st) = st {
            self.backfills.push((span, st, Some(ty)));
        }
    }

    /// Declare parameters' type in function body
    fn declare_params(&mut self, body: &FuncBody) {
        // A parameter's annotation may name a type parameter of the function
        // itself, and that name means `Type::Param` rather than a lookup that
        // happens to fail. Without this the annotation of `x: T` in
        // `function f<T>(x: T)` resolved to nothing and quietly became `any`.
        let names: Vec<&str> = body
            .1
            .iter()
            .map(|TypeParam((n, _), _, _, _)| n.as_str())
            .collect();
        for param in body.0.iter() {
            match param {
                Param::Typed((name, span), ty) => {
                    let ty = self.resolve_type(&normalize_generic_names(ty, &names));
                    self.declare(name, *span, ty)
                }
                Param::Name((name, span)) => self.declare(name, *span, Type::Any),
                Param::Var(_) => {
                    // VarArg is `...`
                }
            }
        }
    }

    fn resolve_type(&mut self, ty: &TypeDesc) -> Type {
        match ty {
            TypeDesc::Pure(t) => t.clone(),
            other => {
                let init = EvalCtxInit {
                    source: self.source.clone(),
                    viewer: self.viewer.clone(),
                    type_fns: self.type_fns,
                    inline_type_fns: self.inline_type_fns,
                    objects: self.objects,
                    aliases: self.aliases,
                    results: self.call_cache.clone(),
                    closures: self.closures.clone(),
                    modules: Some(self.modules),
                    provider: self.provider,
                    report_errors: true,
                };
                let mut hook = |t: &TypeDesc| match t {
                    TypeDesc::TypeOf { expr, .. } => Some(TypeValue::Type(self.infer_expr(expr))),
                    TypeDesc::Named(name, _) => self.lookup_type(name).map(TypeValue::Type),
                    _ => None,
                };
                let mut ev = EvalCtx::new(init).with_hook(Some(&mut hook));
                let r = ev.eval_type(other).to_type();
                let errs = std::mem::take(&mut ev.errors);
                self.errors.extend(errs);
                r
            }
        }
    }

    /// Reports a type name that resolves to nothing.
    ///
    /// A name like this used to answer `any`, which is worse than useless: every
    /// bound and every obligation then passes, and the mistake only surfaces
    /// much later as a mismatch against something unrelated.
    ///
    /// This is asked of signatures and nowhere else. A signature is where the
    /// language commits to a type, so a name in one that resolves to nothing is
    /// a mistake; the annotation on a `local` is not a commitment, and the
    /// language has always let a name in one of those stand for whatever it
    /// turns out to be (see `object_unknown_annotation`). The evaluator cannot
    /// make the call at all, because it is also reached from speculative places
    /// with partial environments, where a name it cannot see is a parameter that
    /// has not been bound yet rather than a mistake.
    fn check_type_names(&mut self, td: &TypeDesc) {
        if let TypeDesc::Named(name, span) = td
            && self.lookup_type(name).is_none()
            && self.viewer.lookup(name).is_none()
            && !self.reported_unknown_types.contains(&(name.clone(), *span))
        {
            self.reported_unknown_types.push((name.clone(), *span));
            self.err(DukaSemanticError::UnknownType(name.clone()), *span);
        }
        for child in td.type_children() {
            self.check_type_names(child);
        }
        // A type written as an expression holds its names in `TypeLit`, which
        // the derived visitor stops short of, so the walk is started from here.
        for expr in td.expressions() {
            let mut walk = TypeNameWalk(self);
            expr.visit(&mut walk);
        }
    }
    /// Runs a `where` list, top to bottom, so a clause can see the one above it.
    ///
    /// This is the declaration pass. The type parameters are still
    /// `Type::Param` here, so a bound or a concept that mentions one cannot be
    /// decided: that is not a failure, it is an obligation, and it is recorded
    /// for the call site to discharge once the arguments are known. A concept
    /// that answers `false` about types nothing is still waiting on *is* a
    /// failure and is reported here.
    ///
    /// A `type V = E` is put in the type frame, which is what makes `: V` in the
    /// return annotation and `V` in the body resolve to the same thing. It is put
    /// there rather than rewritten into a `Type::Param` because the value is
    /// already the answer: `V` is `T`, not a new variable that happens to equal
    /// one.
    fn run_where(&mut self, clauses: &[WhereClause], params: &[Box<str>]) {
        for clause in clauses {
            match clause {
                WhereClause::Bound((name, span), bound, _) => {
                    let ty = self.resolve_type(bound);
                    // The bound is not kept here. It is read twice, from two
                    // places that both have the clause in hand: `visit_func_block`
                    // files it into the type frame for the body, and
                    // `discharge_where` checks the obligation against the
                    // solution. A copy on the side would only be a third reading
                    // to keep in step with those two.
                    if mentions_param(&ty) {
                        self.obligations.push(Obligation::Bound {
                            name: name.clone().into_boxed_str(),
                            bound: ty,
                            span: *span,
                        });
                    }
                }
                WhereClause::Concept(cond, span) => {
                    if !self.concept_truth(cond, *span, params) {
                        self.err(DukaSemanticError::WhereConceptFailed, *span);
                    }
                }
                WhereClause::Bind(names, value, _span) => {
                    let ty = self.resolve_type(value);
                    for (name, span) in where_bind_names(names) {
                        self.declare(&name, span, ty.clone());
                    }
                }
            }
        }
    }

    /// A concept read for its truth. Duka's own reading: `nil` and `false` fail
    /// and everything else answers, because a concept is written over types and
    /// almost no type is a boolean.
    ///
    /// A concept that mentions one of this function's own type parameters cannot
    /// be read here at all -- nothing is known yet -- so it is recorded as an
    /// obligation and answered as if it held, rather than being called false. The
    /// mention is looked for in the concept itself and not in its answer, because
    /// `T == int` answers `false` rather than saying anything about `T`.
    fn concept_truth(&mut self, cond: &Expr, span: Span, params: &[Box<str>]) -> bool {
        if type_expr_names_type_param(cond, params) {
            self.concepts_deferred.push(cond.clone());
            self.obligations.push(Obligation::Concept { span });
            return true;
        }
        let ty = self.resolve_type(&TypeDesc::Expr(Box::new(cond.clone())));
        // `false` is the only answer that fails, and so are the types that stand
        // for "nothing": anything else -- including a type that is not a boolean
        // at all -- answers, because that is what a requirement over types means.
        !matches!(
            ty,
            Type::Literal(ConstValue::Bool(false))
                | Type::Literal(ConstValue::Nil)
                | Type::Nil
                | Type::Never
        )
    }

    fn resolve_module_type(&self, name: &str) -> Option<&'a ModuleType> {
        crate::analyzer::modules::resolve_module_type(
            self.modules,
            name,
            self.source.name.path(),
            self.provider?,
        )
    }

    fn resolve_display(&mut self, td: &TypeDesc) -> Option<String> {
        let init = EvalCtxInit {
            source: self.source.clone(),
            viewer: self.viewer.clone(),
            type_fns: self.type_fns,
            inline_type_fns: self.inline_type_fns,
            objects: self.objects,
            aliases: self.aliases,
            results: self.call_cache.clone(),
            closures: self.closures.clone(),
            modules: Some(self.modules),
            provider: self.provider,
            report_errors: false,
        };
        let mut hook = |t: &TypeDesc| match t {
            TypeDesc::TypeOf { expr, .. } => Some(TypeValue::Type(self.infer_expr(expr))),
            TypeDesc::Named(name, _) => self.lookup_type(name).map(TypeValue::Type),
            _ => None,
        };
        let mut ev = EvalCtx::new(init).with_hook(Some(&mut hook));
        let val = ev.eval_type(td);
        match &val {
            TypeValue::Closure(c) => {
                let params = c
                    .params
                    .iter()
                    .map(|p| match p {
                        Param::Typed(_, t) => t.to_string(),
                        _ => "any".to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = c
                    .body
                    .2
                    .as_ref()
                    .map(|r| {
                        r.tys
                            .iter()
                            .map(|t| t.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let arrow = if ret.is_empty() { "" } else { " -> " };
                Some(format!("type fn({params}){arrow}{ret}"))
            }
            _ => None,
        }
    }
}

impl TypeCheckerCtx<'_> {
    #[inline]
    fn fn_type(&mut self, body: &FuncBody) -> Type {
        self.fn_type_ret(body, &Attrs::default(), None)
    }

    fn fn_type_ret(
        &mut self,
        body: &FuncBody,
        attrs: &Attrs,
        inferred: Option<&InferredReturns>,
    ) -> Type {
        let FuncBody(_, type_params, ret, _, _) = body;
        let names: Vec<&str> = type_params
            .iter()
            .map(|TypeParam((n, _), _, _, _)| n.as_str())
            .collect();
        let declared: (Box<[Type]>, bool) = match ret {
            Some(r) => (
                r.tys
                    .iter()
                    .map(|t| {
                        let normalized = normalize_generic_names(t, &names);
                        self.check_type_names(&normalized);
                        self.resolve_type(&normalized)
                    })
                    .collect(),
                r.var_arg,
            ),
            None => match inferred {
                // a return that does not always fill the last position is only
                // allowed to vary because the function declares `...`
                Some(r) => (r.types.clone(), r.var_arg && body.has_var_arg()),
                None => ([].into(), false),
            },
        };
        // a return protocol says what the slots are, so it settles the question
        // an annotation or an inference would otherwise leave open
        let (returns, return_var_arg) = match attributes::returns(attrs) {
            Some(Returns::Result) => ([Type::Bool].into(), true),
            Some(Returns::Exit) => ([].into(), false),
            None => declared,
        };
        Type::Function(Some(FunctionType {
            params: body
                .named_params()
                .map(|(_, ty)| match ty {
                    Some(t) => {
                        let normalized = normalize_generic_names(t, &names);
                        self.check_type_names(&normalized);
                        self.resolve_type(&normalized)
                    }
                    None => Type::Any,
                })
                .collect(),
            var_arg: body.has_var_arg(),
            returns,
            return_var_arg,
        }))
    }
}

/// Rewrites every mention of a declared type parameter into `Type::Param`, the
/// match is exhaustive on purpose: a missing arm would silently turn the
/// parameter into `Any` instead of failing to compile
fn normalize_generic_names(tv: &TypeDesc, names: &[&str]) -> TypeDesc {
    match tv {
        TypeDesc::Pure(_) | TypeDesc::TypeOf { .. } => tv.clone(),
        // a name in a type-level expression is a parameter too, and leaving it
        // alone would resolve it to `any` at every use
        TypeDesc::Expr(expr) => {
            TypeDesc::Expr(Box::new(normalize_generic_names_in_expr(expr, names)))
        }
        TypeDesc::Named(name, _) if names.contains(&name.as_ref()) => {
            TypeDesc::Pure(Type::Param(name.clone()))
        }
        TypeDesc::Named(..) => tv.clone(),
        TypeDesc::Generic { name, args, span } => TypeDesc::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            span: *span,
        },
        TypeDesc::TypeCall { name, args, span } => TypeDesc::TypeCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            span: *span,
        },
        TypeDesc::Access {
            base,
            member,
            args,
            span,
        } => TypeDesc::Access {
            base: Box::new(normalize_generic_names(base, names)),
            member: Box::new(normalize_generic_names(member, names)),
            args: args.as_ref().map(|a| {
                a.iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect()
            }),
            span: *span,
        },
        TypeDesc::Array(e) => TypeDesc::Array(
            e.as_deref()
                .map(|e| Box::new(normalize_generic_names(e, names))),
        ),
        TypeDesc::Table(k, v) => TypeDesc::Table(
            k.as_deref()
                .map(|k| Box::new(normalize_generic_names(k, names))),
            v.as_deref()
                .map(|v| Box::new(normalize_generic_names(v, names))),
        ),
        TypeDesc::Union(ts) => TypeDesc::Union(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
        ),
        TypeDesc::TypeTuple(ts) => TypeDesc::TypeTuple(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
        ),
        TypeDesc::TypeTable(ts) => TypeDesc::TypeTable(
            ts.iter()
                .map(|(k, span, v)| (k.clone(), *span, normalize_generic_names(v, names)))
                .collect(),
        ),
        TypeDesc::Function(ft) => TypeDesc::Function(ft.as_ref().map(|ft| {
            TypeFnValue {
                params: ft
                    .params
                    .iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect(),
                var_arg: ft.var_arg,
                returns: ft
                    .returns
                    .iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect(),
                return_var_arg: ft.return_var_arg,
            }
        })),
        TypeDesc::FnLit(body) => {
            let mut cloned = body.as_ref().clone();
            let FuncBody(params, _, ret, _, _) = &mut cloned;
            for p in params.iter_mut() {
                if let Param::Typed(_, t) = p {
                    *t = normalize_generic_names(t, names);
                }
            }
            if let Some(r) = ret.as_mut() {
                for t in r.tys.iter_mut() {
                    *t = normalize_generic_names(t, names);
                }
            }
            TypeDesc::FnLit(Box::new(cloned))
        }
        TypeDesc::NonNil(inner) => {
            TypeDesc::NonNil(Box::new(normalize_generic_names(inner, names)))
        }
        TypeDesc::Nilable(inner) => {
            TypeDesc::Nilable(Box::new(normalize_generic_names(inner, names)))
        }
        TypeDesc::Rec(inner) => TypeDesc::Rec(Box::new(normalize_generic_names(inner, names))),
    }
}

/// The same rewrite inside a type-level expression. A bare name can only appear
/// as a `TypeLit`, so that is the only place a parameter can hide; the walk is
/// over the whole tree because an expression can nest records in calls.
fn normalize_generic_names_in_expr(expr: &Expr, names: &[&str]) -> Expr {
    let mut cloned = expr.clone();
    let mut rewrite = NormalizeGenericNames { names };
    rewrite.walk(&mut cloned);
    cloned
}

struct NormalizeGenericNames<'a> {
    names: &'a [&'a str],
}

impl NormalizeGenericNames<'_> {
    /// Rewrites every `TypeLit` in the expression tree. `Expr` has no
    /// `VisitorMut` derive, so the walk is spelled out over the node kinds a
    /// type-level expression may hold; a kind the gate refuses is not here.
    fn walk(&mut self, expr: &mut Expr) {
        match &mut expr.0 {
            ExprKind::TypeLit(td) => *td = normalize_generic_names(td, self.names),
            ExprKind::Call(callee, args) => {
                self.walk(callee);
                for arg in args.iter_mut() {
                    self.walk(arg);
                }
            }
            ExprKind::Unary(inner, _) => self.walk(inner),
            ExprKind::Binary(l, r, _) => {
                self.walk(l);
                self.walk(r);
            }
            _ => (),
        }
    }
}

impl<'a> Visitor for TypeCheckerCtx<'a> {
    fn visit_stmt(&mut self, stmt: &crate::parser::ast::Stmt) {
        match &stmt.0 {
            StmtKind::Define(names, exprs, _, _) => {
                let (tys, _var_arg) = self.infer_expr_list(exprs);
                for (idx, (((name, span), _attrs, ty), _)) in names.iter().enumerate() {
                    let actual = if let Some(ty) = tys.get(idx) {
                        let cv = match exprs.get(idx) {
                            Some(Expr(ExprKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        (ty.clone(), cv)
                    } else {
                        (Type::Nil, None)
                    };
                    let declared = ty.as_ref().map(|t| self.resolve_type(t));
                    if let Some(declared) = &declared
                        && !declared.accepts_value(&actual.0, actual.1.as_ref())
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                declared.to_string(),
                                actual.0.to_string(),
                            ),
                            exprs.get(idx).map(|e| e.1).unwrap_or(*span),
                        );
                    }
                    self.declare(
                        name,
                        *span,
                        declared.unwrap_or_else(|| inferred_init(actual.0)),
                    );
                }
            }
            StmtKind::Assign(targets, exprs) => {
                let (tys, _var_arg) = self.infer_expr_list(exprs);
                for (idx, target) in targets.iter().enumerate() {
                    let actual = if let Some(ty) = tys.get(idx) {
                        let cv = match exprs.get(idx) {
                            Some(Expr(ExprKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        (ty.clone(), cv)
                    } else {
                        (Type::Nil, None)
                    };
                    if let Path::Base((name, span)) = target
                        && self.lookup_type(name).is_none()
                    {
                        self.declare(name, *span, inferred_init(actual.0));
                        continue;
                    }
                    if let Some((owner, key, key_span)) = assign_target(target) {
                        self.assign_into(
                            owner,
                            key,
                            &actual.0,
                            exprs.get(idx).map(|e| e.1).unwrap_or(key_span),
                        );
                        continue;
                    }

                    if let Path::Base((name, sp)) = target
                        && let Some(declared) = self.lookup_type(name)
                        && !declared.accepts_value(&actual.0, actual.1.as_ref())
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                declared.to_string(),
                                actual.0.to_string(),
                            ),
                            exprs.get(idx).map(|e| e.1).unwrap_or(*sp),
                        );
                    }
                }
            }
            StmtKind::Return(items, ..) => {
                if self.collect_mode {
                    let (collected, _) = self.infer_expr_list(items);
                    if let Some(buf) = self.ret_collect.last_mut() {
                        buf.merge(&collected);
                    }
                }
                let ret = self.ret_stack.last().cloned().flatten();

                // if ret.is_none() -> void

                // the values are inferred either way, an unknown return type
                // only means there is nothing to check them against
                let (tys, var_arg_get) = self.infer_expr_list(items);
                if let Some((fixeds, var_arg)) = ret {
                    if tys.len() < fixeds.len() && !var_arg_get {
                        self.err(
                            DukaSemanticError::TypeMismatchReturn(
                                fixeds
                                    .iter()
                                    .map(|f| f.to_string())
                                    .chain(var_arg.then_some("...".to_owned()))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                format!("{} values", tys.len()),
                            ),
                            items.first().map(|e| e.1).unwrap_or_default(),
                        );
                    }
                    for (idx, ty) in tys.into_iter().enumerate() {
                        let cv = match items.get(idx) {
                            Some(Expr(ExprKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        let Some(expected) = fixeds.get(idx).cloned() else {
                            break; // whether var_arg or not, drop them
                        };
                        // a generic return type is only known at the call site
                        if mentions_param(&expected) {
                            continue;
                        }
                        if !expected.accepts_value(&ty, cv.as_ref()) {
                            self.err(
                                DukaSemanticError::TypeMismatchReturn(
                                    expected.to_string(),
                                    ty.to_string(),
                                ),
                                items.get(idx).map(|e| e.1).unwrap_or_default(),
                            );
                        }
                    }
                }
            }
            StmtKind::Function(path, attrs, body, _) => {
                if let Path::Base((name, span)) = path {
                    // the `where` list runs before the signature is built,
                    // because `type V = T` in it is what `: V` names, and before
                    // anything is inferred, because a bound written there is what
                    // the body is checked against
                    self.run_where(
                        &body.3,
                        &body
                            .1
                            .iter()
                            .map(|TypeParam((n, _), _, _, _)| n.clone().into_boxed_str())
                            .collect::<Vec<_>>(),
                    );
                    if !body.1.is_empty() {
                        self.generic_fns.insert(
                            name.clone().into_boxed_str(),
                            GenericDecl {
                                params: body.1.clone(),
                                constraints: body.3.clone(),
                            },
                        );
                    }
                    let ty = match &body.2 {
                        Some(_) => self.fn_type_ret(body, attrs, None),
                        None => {
                            let inferred = self.inferred_returns.get(name.as_str()).cloned();
                            match inferred {
                                Some(r) => self.fn_type_ret(body, attrs, Some(&r)),
                                None => self.fn_type(body),
                            }
                        }
                    };
                    self.declare(name, *span, ty);
                }
                if self.collect_mode
                    && let Some(returns) = self.finished_returns.pop()
                    && let Path::Base((name, _)) = path
                    && body.2.is_none()
                    && !returns.types.is_empty()
                {
                    self.collected_returns
                        .insert(name.clone().into_boxed_str(), returns);
                }
            }
            StmtKind::Expr(expr) => {
                self.infer_expr(expr);
            }
            StmtKind::TypeAlias((_, span), ty) => {
                let resolved = self.resolve_type(ty);
                let display = if resolved == Type::Any {
                    self.resolve_display(ty)
                        .unwrap_or_else(|| resolved.to_string())
                } else {
                    resolved.to_string()
                };
                if !self.collect_mode {
                    self.backfills
                        .push((*span, display.into_boxed_str(), Some(resolved)));
                }
            }
            StmtKind::TypeFunction((_, span), body) => {
                if !self.collect_mode {
                    self.backfills.push((
                        *span,
                        format!(
                            "function({})",
                            std::iter::repeat_n("type", body.0.len())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                        .into_boxed_str(),
                        None,
                    ));
                }
            }
            StmtKind::Call(callee, args) => {
                if let Expr(ExprKind::Access(path), span) = &**callee {
                    self.infer_call(path, *span, args);
                }
            }
            StmtKind::Export(inner) => {
                // statements are visited post order, so the exported symbols
                // already carry their types here
                for (name, _) in exported_names(&inner.0) {
                    if let Some(ty) = self.lookup_type(&name) {
                        self.export_members.insert(name, ty);
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        let outer = std::mem::replace(&mut self.final_args, false);
        self.infer_expr(expr);
        self.final_args = outer;
        match &expr.0 {
            ExprKind::Unary(e, op) => {
                if let UnOp::BitNot = op
                    && let Expr(ExprKind::Literal(ConstValue::Float(_)), span) = &**e
                {
                    self.err(
                        DukaSemanticError::TypeMismatchEqual(
                            Type::Int.to_string(),
                            Type::Float.to_string(),
                        ),
                        *span,
                    );
                }
            }
            ExprKind::Binary(a, b, op) => {
                if matches!(
                    op,
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::ShiftL | BinOp::ShiftR
                ) {
                    let both_literal =
                        matches!(a.0, ExprKind::Literal(_)) && matches!(b.0, ExprKind::Literal(_));
                    if both_literal {
                        for operand in [a.as_ref(), b.as_ref()] {
                            if matches!(operand.0, ExprKind::Literal(ConstValue::Float(_))) {
                                self.err(
                                    DukaSemanticError::TypeMismatchEqual(
                                        Type::Int.to_string(),
                                        Type::Float.to_string(),
                                    ),
                                    operand.1,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn after(&mut self) {}

    fn visit_func_block(&mut self, block: &FuncBody, enter: bool) {
        if enter {
            for TypeParam((name, span), _, _, _) in block.1.iter() {
                self.declare(name, *span, Type::Param(name.clone().into_boxed_str()));
                let name_str = name.as_str();
                // A `where U: Point` bound goes into the same frame as the
                // parameter, under a name nothing can collide with. The frame is
                // what scopes it: it leaves when the body does, so a parameter of
                // the same name in a nested function cannot read this one, and
                // nothing has to be popped by hand.
                //
                // Read out of `block` rather than off the checker's field. The
                // field is filled in by `run_where` on the way in, so a body
                // reached by any other route would be checked against whatever
                // the last function visited happened to leave behind -- and a
                // nested function would find an outer bound of the same name. The
                // clauses are right here; there is no reason to go looking.
                let bound = block.3.iter().find_map(|clause| match clause {
                    WhereClause::Bound((name, _), bound, _) if name.as_str() == name_str => {
                        Some(bound.as_ref())
                    }
                    _ => None,
                });
                if let Some(bound) = bound {
                    let bound = self.resolve_type(bound);
                    self.declare(&bound_key(name), *span, bound);
                }
            }
            let ret = match &block.2 {
                Some(r) => {
                    let tys: Vec<Type> = r.tys.iter().map(|t| self.resolve_type(t)).collect();

                    if tys.is_empty() && !r.var_arg {
                        None
                    } else {
                        Some((tys, r.var_arg))
                    }
                }
                None => None,
            };
            self.ret_stack.push(ret);
            self.declare_params(block);
            if self.collect_mode {
                self.ret_collect.push(InferredReturns::default());
            }
        } else {
            if self.collect_mode {
                let collected = self.ret_collect.pop().unwrap_or_default();
                self.finished_returns.push(collected);
            }
            self.ret_stack.pop();
        }
    }

    fn visit_block(&mut self, enter: bool) {
        if enter {
            self.types.push(HashMap::new());
            self.viewer.enter();
        } else {
            self.viewer.exit();
            self.types.pop();
        }
    }
}

impl TypeCheckerCtx<'_> {
    #[inline(always)]
    fn infer_expr(&mut self, Expr(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_list(&mut self, list: &[Expr]) -> Multi {
        let mut res = vec![];
        let mut va = false;
        if let Some((last, rest)) = list.split_last() {
            for e in rest {
                res.push(self.infer_expr_kind(&e.0));
            }
            let (mut ts, var_arg) = self.infer_expr_kind_multi(&last.0);
            res.append(&mut ts);
            va = var_arg;
        }
        (res, va)
    }

    fn infer_expr_kind_multi(&mut self, kind: &ExprKind) -> Multi {
        match kind {
            ExprKind::VarArg => (vec![], true),
            ExprKind::Call(callee, args) => {
                if let Expr(ExprKind::Access(path), span) = &**callee
                    && let Some(a) = self.infer_call(path, *span, args)
                {
                    return a;
                }
                (vec![Type::Any], true)
            }
            ek => (vec![self.infer_expr_kind(ek)], false),
        }
    }

    fn infer_expr_kind(&mut self, kind: &ExprKind) -> Type {
        match kind {
            ExprKind::Literal(lit) => lit.type_of(),
            ExprKind::Table(fields) => {
                if fields.is_empty() {
                    return Type::Table(None, None);
                }
                if !fields.iter().all(|f| matches!(f, Field::Value(_))) {
                    let mut vec = vec![];
                    for f in fields {
                        if let Field::NameValue(n, v) = f {
                            let e = &self.infer_expr_kind(&v.0);
                            vec.push((
                                ConstValue::String(n.0.as_bytes().to_vec().into_boxed_slice()),
                                Box::new(e.clone()),
                            ))
                        } else {
                            return Type::Table(None, None);
                        }
                    }
                    return Type::TypeTable(vec);
                }
                let last = fields.len().saturating_sub(1);
                let mut elems = vec![];
                for (idx, f) in fields.iter().enumerate() {
                    let Field::Value(e) = f else { continue };
                    if idx == last {
                        let (mut ts, _) = self.infer_expr_kind_multi(&e.0);
                        elems.append(&mut ts);
                    } else {
                        elems.push(self.infer_expr(e));
                    }
                }
                Type::TypeTuple(elems)
            }
            ExprKind::Array(items) => {
                let (elems, _) = self.infer_expr_list(items);
                Type::TypeTuple(elems)
            }
            ExprKind::Function(body) => {
                let ft @ Type::Function(Some(_)) = self.fn_type(body) else {
                    return Type::Any;
                };
                ft
            }
            ExprKind::Access(path) => self.infer_access(path),
            ExprKind::Call(callee, args) => match &**callee {
                Expr(ExprKind::Access(path), span) => match self.infer_call(path, *span, args) {
                    Some((a, true)) if a.is_empty() => Type::Any,
                    Some((v, _)) => v.into_iter().next().unwrap_or(Type::Nil),
                    None => Type::Never,
                },
                _ => Type::Any,
            },
            ExprKind::Unary(e, op) => match op {
                UnOp::Minus => self.infer_expr(e),
                UnOp::Length => match self.infer_expr(e) {
                    Type::String => Type::Int,
                    _ => Type::Any,
                },
                UnOp::Not => match self.infer_expr(e) {
                    Type::Bool => Type::Bool,
                    _ => Type::Any,
                },
                UnOp::BitNot => match self.infer_expr(e) {
                    Type::Int => Type::Int,
                    Type::Bool => Type::Bool,
                    _ => Type::Any,
                },
            },
            ExprKind::Binary(a, b, op) => match op {
                BinOp::Concat => Type::String,
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::ShiftL | BinOp::ShiftR => {
                    let lt = self.infer_expr(a);
                    let rt = self.infer_expr(b);
                    match (lt, rt) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Bool, Type::Bool) => Type::Bool,
                        _ => Type::Any,
                    }
                }
                BinOp::Add
                | BinOp::Sub
                | BinOp::Multiply
                | BinOp::Divide
                | BinOp::IDivide
                | BinOp::Mod
                | BinOp::Pow => {
                    let lt = self.infer_expr(a);
                    let rt = self.infer_expr(b);
                    match (lt, rt) {
                        (Type::Float, Type::Float) => Type::Float,
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int) | (Type::Int, Type::Float) => Type::Float,
                        _ => Type::Any,
                    }
                }
                _ => Type::Bool,
            },
            _ => Type::Any,
        }
    }

    #[inline]
    fn object_id_of(&self, name: &str) -> Option<ObjectId> {
        self.viewer
            .lookup(name)
            .and_then(|sym| match &sym.symbol_type {
                SymbolType::ObjectClass(id) => Some(*id),
                _ => None,
            })
    }

    #[inline]
    fn receiver_object(&mut self, path: &Path) -> Option<ObjectId> {
        match path {
            Path::Base((name, _)) => {
                if let Some(ty) = self.lookup_type(name)
                    && let Some(id) = self.object_member_of(&ty)
                {
                    return Some(id);
                }
                self.object_id_of(name)
            }
            _ => {
                // resolve the chain by type, `a.b:c()` looks `c` up on the type
                // of `a.b` instead of on the root of the chain
                let ty = self.type_of_path(path);
                self.object_member_of(&ty)
            }
        }
    }

    #[inline]
    fn object_member_of(&self, ty: &Type) -> Option<ObjectId> {
        match ty {
            Type::Object { id, .. } => Some(*id),
            Type::Union(ts) => ts.iter().find_map(|t| self.object_member_of(t)),
            _ => None,
        }
    }

    #[inline]
    fn object_of(&self, id: ObjectId) -> Type {
        let obj = &self.objects[id];
        Type::Object {
            id,
            name: obj.name.clone(),
            base: obj.base,
            args: [].into(),
        }
    }

    #[inline]
    fn return_type_of(&self, sig: &FunctionType) -> Option<Multi> {
        match (sig.returns.len(), sig.return_var_arg) {
            (0, false) => None,
            (0, true) => Some((vec![], true)),
            _ => Some((sig.returns.clone().into_vec(), sig.return_var_arg)),
        }
    }

    #[inline]
    fn find_method(&self, id: ObjectId, name: &str) -> Option<ObjectMethod> {
        self.objects[id]
            .methods
            .iter()
            .find(|m| m.name.as_ref() == name)
            .cloned()
    }

    fn infer_access(&mut self, path: &Path) -> Type {
        self.type_of_path(path)
    }

    /// Walks a path to the type it denotes, one suffix at a time
    fn type_of_path(&mut self, path: &Path) -> Type {
        match path {
            Path::Base((name, _)) => self.lookup_type(name.as_str()).unwrap_or(Type::Any),
            Path::Expr(expr) => self.infer_expr(expr),
            Path::Chain(receiver, suffix) => {
                let recv = self.type_of_path(receiver);
                self.suffix_type(&recv, suffix)
            }
        }
    }

    fn suffix_type(&mut self, recv: &Type, suffix: &PathSuffix) -> Type {
        match suffix {
            PathSuffix::Dot((name, _)) => self.member_type(recv, name),
            PathSuffix::Colon((name, _)) => self.method_type(recv, name),
            PathSuffix::Index(index) => self.index_type(recv, index),
            PathSuffix::TypeArgs(..) => Type::Any,
        }
    }

    /// `obj:method` used as a value keeps the method signature
    fn method_type(&mut self, recv: &Type, name: &str) -> Type {
        if let Some(id) = self.object_member_of(recv)
            && let Some(m) = self.find_method(id, name)
        {
            return Type::Function(Some(m.sig));
        }
        self.member_type(recv, name)
    }

    fn member_type(&mut self, recv: &Type, name: &str) -> Type {
        match recv {
            // A member read on a type parameter is a member read on whatever the
            // `where` bound says it may be. Without this the arm below answers
            // `any`, and a bound written to make `offset.x` checkable would
            // check nothing at all.
            Type::Param(var) => match self.lookup_type(&bound_key(var)) {
                Some(bound) => self.member_type(&bound, name),
                None => Type::Any,
            },
            Type::Object { id, .. } => self.objects[*id]
                .members
                .iter()
                .find(|m| m.name.as_ref() == name)
                .map(|m| self.resolve_type(&m.ty))
                .unwrap_or(Type::Any),
            Type::TypeTable(fields) => {
                let key = ConstValue::String(name.as_bytes().to_vec().into_boxed_slice());
                fields
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (**v).clone())
                    .unwrap_or(Type::Any)
            }
            Type::Table(Some(k), Some(v)) if matches!(**k, Type::String) => (**v).clone(),
            Type::Union(ts) => ts
                .iter()
                .map(|t| self.member_type(t, name))
                .find(|t| *t != Type::Any)
                .unwrap_or(Type::Any),
            _ => Type::Any,
        }
    }

    fn index_type(&mut self, recv: &Type, index: &Expr) -> Type {
        // a literal key picks one member, a computed one only gives the shape
        if let ExprKind::Literal(ConstValue::String(b)) = &index.0 {
            let name = String::from_utf8_lossy(b);
            if !matches!(recv, Type::TypeTable(..)) {
                return self.member_type(recv, &name);
            }
        }
        if let ExprKind::Literal(ConstValue::Int(i)) = &index.0
            && let Type::TypeTuple(items) = recv
            && let Some(ty) = usize::try_from(*i)
                .ok()
                .and_then(|i| items.get(i.saturating_sub(1)))
        {
            return ty.clone();
        }
        match recv {
            Type::Array(Some(inner)) => (**inner).clone(),
            Type::TypeTable(fields) => {
                let mut tys: Vec<Type> = fields.iter().map(|(_, v)| (**v).clone()).collect();
                tys.dedup();
                match tys.len() {
                    0 => Type::Any,
                    1 => tys.pop().unwrap_or(Type::Any),
                    _ => Type::Union(tys.into()),
                }
            }
            Type::Table(_, Some(v)) => (**v).clone(),
            Type::Union(ts) => ts
                .iter()
                .map(|t| self.index_type(t, index))
                .find(|t| *t != Type::Any)
                .unwrap_or(Type::Any),
            _ => Type::Any,
        }
    }

    /// `require` runs the module and hands back its value, a module we cannot
    /// resolve statically stays `Any` instead of failing the whole file
    fn require_value(&mut self, args: &[Expr]) -> Type {
        let Some(Expr(ExprKind::Literal(ConstValue::String(b)), _)) = args.first() else {
            return Type::Any;
        };
        let name = String::from_utf8_lossy(b);
        match self.resolve_module_type(&name) {
            Some(module) => module.value_type(),
            None => Type::Any,
        }
    }

    /// `a.b.c = value`: the record `owner` gains `key`, then every record on the
    /// way back to the symbol is rebuilt with it
    fn assign_into(&mut self, owner: &Path, key: &str, value: &Type, span: Span) {
        if self.collect_mode {
            return;
        }
        let recv = self.type_of_path(owner);
        if let Some(id) = self.object_member_of(&recv) {
            self.check_member_assign(id, key, value, span);
            return;
        }
        let Some(record) = self.record_after_write(&recv, key, value, span) else {
            return;
        };
        self.store_path(owner, record, span);
    }

    /// a record type with `key` set to `value`, the declared member type wins
    /// over a mismatching write
    fn record_after_write(
        &mut self,
        recv: &Type,
        key: &str,
        value: &Type,
        span: Span,
    ) -> Option<Type> {
        let ckey = ConstValue::String(key.as_bytes().to_vec().into_boxed_slice());
        let entry = || (ckey.clone(), Box::new(value.clone()));
        let mut fields = match recv {
            Type::Table(None, None) => vec![entry()],
            Type::TypeTable(fields) => {
                let mut fields = fields.clone();
                match fields.iter_mut().find(|(k, _)| *k == ckey) {
                    Some((_, slot)) => {
                        let merged = {
                            let declared = &**slot;
                            match (declared, value) {
                                (Type::TypeTable(_), Type::TypeTable(_)) => {
                                    Box::new(self.merge_record(declared, value))
                                }
                                (Type::Table(None, None) | Type::Any, _) => Box::new(value.clone()),
                                (declared, written) => {
                                    if !declared.accepts_value(written, None) {
                                        self.err(
                                            DukaSemanticError::TypeMismatchEqual(
                                                declared.to_string(),
                                                written.to_string(),
                                            ),
                                            span,
                                        );
                                    }
                                    slot.clone()
                                }
                            }
                        };
                        *slot = merged;
                    }
                    None => fields.push(entry()),
                }
                fields
            }
            _ => return None,
        };
        fields.dedup_by(|a, b| a.0 == b.0);
        Some(Type::TypeTable(fields))
    }

    /// merges a written record into the declared one, a declared member type
    /// wins over the write that disagrees with it
    fn merge_record(&self, declared: &Type, written: &Type) -> Type {
        match (declared, written) {
            (Type::TypeTable(declared), Type::TypeTable(written)) => {
                let mut fields = declared.clone();
                for (key, value) in written {
                    match fields.iter_mut().find(|(k, _)| k == key) {
                        Some(slot) => {
                            *slot.1 = self.merge_record(&slot.1, value);
                        }
                        None => fields.push((key.clone(), value.clone())),
                    }
                }
                Type::TypeTable(fields)
            }
            // an open table carries no information, the write refines it
            (Type::Table(None, None) | Type::Any, _) => written.clone(),
            _ => declared.clone(),
        }
    }

    /// stores `value` into the record denoted by `path`, refreshing the symbol
    fn store_path(&mut self, path: &Path, value: Type, span: Span) {
        match path {
            Path::Base((name, nspan)) => self.declare(name, *nspan, value),
            Path::Chain(base, PathSuffix::Dot((key, _))) => {
                let recv = self.type_of_path(base);
                if let Some(record) = self.record_after_write(&recv, key, &value, span) {
                    self.store_path(base, record, span);
                }
            }
            _ => {}
        }
    }

    fn check_member_assign(&mut self, id: ObjectId, key: &str, value: &Type, span: Span) {
        let Some(member) = self.objects[id]
            .members
            .iter()
            .find(|m| m.name.as_ref() == key)
            .cloned()
        else {
            return;
        };
        let declared = self.resolve_type(&member.ty);
        if !declared.accepts_value(value, None) {
            self.err(
                DukaSemanticError::TypeMismatchEqual(declared.to_string(), value.to_string()),
                span,
            );
        }
    }

    /// The tail statement of a module decides what `require` returns
    fn infer_tail(&mut self, tail: &Stmt) -> Option<Type> {
        match &tail.0 {
            StmtKind::Return(items, ..) => {
                let (tys, _) = self.infer_expr_list(items);
                tys.into_iter().next()
            }
            StmtKind::Expr(expr) => Some(self.infer_expr(expr)),
            StmtKind::Call(callee, args) => {
                let Expr(ExprKind::Access(path), span) = &**callee else {
                    return None;
                };
                self.infer_call(path, *span, args)
                    .and_then(|(tys, _)| tys.into_iter().next())
            }
            _ => None,
        }
    }

    /// `ExportDesugarer` returns the export table when the module has no tail
    /// expression of its own, so the same rule decides the module value type
    fn module_value(&self, tail: Option<Type>) -> ModuleValue {
        let members = self.export_members.clone();
        let ty = match tail {
            Some(ty) => ty,
            None if members.is_empty() => Type::Table(None, None),
            None => Type::TypeTable(
                members
                    .iter()
                    .map(|(name, ty)| {
                        (
                            ConstValue::String(name.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(ty.clone()),
                        )
                    })
                    .collect(),
            ),
        };
        ModuleValue { ty, members }
    }

    /// Annotations are nilable by default, so a callable is often reached
    /// through a union: dig the signature out instead of losing it
    #[inline]
    fn as_function(ty: &Type) -> Option<FunctionType> {
        match ty {
            Type::Function(Some(ft)) => Some(ft.clone()),
            Type::Union(ts) => ts.iter().find_map(Self::as_function),
            _ => None,
        }
    }

    /// Declared type parameters as solver variables, bounds and defaults may
    /// mention other variables so they are normalized like the body
    fn var_decls(&mut self, decl: &[TypeParam]) -> Vec<solver::VarDecl> {
        let names: Vec<&str> = decl
            .iter()
            .map(|TypeParam((n, _), _, _, _)| n.as_str())
            .collect();
        decl.iter()
            .map(
                |TypeParam((name, span), bound, default, shape)| solver::VarDecl {
                    name: name.clone().into_boxed_str(),
                    bound: bound
                        .as_ref()
                        .map(|b| self.resolve_type(&normalize_generic_names(b, &names))),
                    default: default
                        .as_ref()
                        .map(|d| self.resolve_type(&normalize_generic_names(d, &names))),
                    span: *span,
                    shape: *shape,
                },
            )
            .collect()
    }

    /// Checks the `where` list of a call, now that the variables are solved.
    ///
    /// This is the second run of the same clauses. The declaration pass read them
    /// with the variables still unknown, so a concept that mentioned one was an
    /// obligation rather than an answer; here the solution is substituted in and
    /// the question can finally be put.
    ///
    /// A concept that is still undecided after substitution is left alone. The
    /// solution does not always determine every variable -- one that only appears
    /// in a return type, say -- and refusing a call over a variable nobody passed
    /// would be worse than letting it through.
    fn discharge_where(
        &mut self,
        constraints: &[WhereClause],
        solution: &solver::Solution,
        call_span: Span,
    ) {
        for clause in constraints {
            match clause {
                WhereClause::Bound((name, _), bound, _) => {
                    let Some(actual) = solution.bindings.get(name.as_str()) else {
                        continue;
                    };
                    let bound = self.resolve_type(bound);
                    if mentions_param(&bound) {
                        continue;
                    }
                    if !bound.accepts(actual) {
                        self.err(
                            DukaSemanticError::WhereBoundViolated(
                                name.clone().into_boxed_str(),
                                bound.to_string(),
                                actual.to_string(),
                            ),
                            call_span,
                        );
                    }
                }
                WhereClause::Concept(cond, _) => {
                    // The solution goes into a scope of its own before the
                    // concept is read, rather than being substituted into the
                    // concept afterwards. Substitution cannot work here: the
                    // concept compares its operands, so `T == int` has to be
                    // read with `T` already answered -- substituting the answer
                    // into the result would compare `Param("T")` against `int`
                    // and then substitute into a `false` that is already
                    // decided. Without this the name does not resolve at all, and
                    // an unknown name in a type position is `any`.
                    self.types.push(std::collections::HashMap::new());
                    for (name, ty) in &solution.bindings {
                        self.types
                            .last_mut()
                            .expect("a scope was just pushed")
                            .insert(name.clone(), ty.clone());
                    }
                    let ty = self.resolve_type(&TypeDesc::Expr(cond.clone()));
                    self.types.pop();
                    if mentions_param(&ty) {
                        continue;
                    }
                    if matches!(
                        ty,
                        Type::Literal(ConstValue::Bool(false))
                            | Type::Literal(ConstValue::Nil)
                            | Type::Nil
                            | Type::Never
                    ) {
                        self.err(DukaSemanticError::WhereConceptFailed, call_span);
                    }
                }
                WhereClause::Bind(..) => {}
            }
        }
    }

    /// Checks the arguments of a generic call once the variables are known: the
    /// formal type is substituted first, and anything still mentioning a
    /// variable or carrying no information is left alone
    fn check_generic_args(
        &mut self,
        params: &[Type],
        arg_types: &[Type],
        args: &[Expr],
        solution: &solver::Solution,
    ) {
        for (idx, formal) in params.iter().enumerate() {
            let Some(actual) = arg_types.get(idx) else {
                break;
            };
            let formal = solution.substitute(formal);
            if mentions_param(&formal) || !solver::Solver::has_info(actual) {
                continue;
            }
            let cv = match args.get(idx) {
                Some(Expr(ExprKind::Literal(cv), _)) => Some(cv.clone()),
                _ => None,
            };
            if !formal.accepts_value(actual, cv.as_ref()) {
                self.err(
                    DukaSemanticError::TypeMismatchEqual(formal.to_string(), actual.to_string()),
                    args.get(idx).map(|e| e.1).unwrap_or_default(),
                );
            }
        }
    }

    /// Turns solver diagnostics into the language's own errors
    /// Records what each type parameter of a generic call solved to, rendered
    /// with the bound it was declared with so the server can show both
    fn record_bindings(
        &mut self,
        call_span: Span,
        decl: &[TypeParam],
        solution: &solver::Solution,
    ) {
        if !self.collect_mode {
            let bindings: Vec<GenericBinding> = decl
                .iter()
                .filter_map(|param| {
                    let name = param.0.0.as_str();
                    let value = solution.bindings.get(name)?;
                    Some(GenericBinding {
                        name: name.to_owned().into_boxed_str(),
                        value: value.to_string().into_boxed_str(),
                        bound: param
                            .1
                            .as_ref()
                            .map(|b| self.resolve_type(b).to_string().into_boxed_str()),
                    })
                })
                .collect();
            if !bindings.is_empty() {
                self.generic_bindings.push((call_span, bindings));
            }
        }
    }

    /// The solver reports diagnostics against the type parameter's own span,
    /// which is its declaration. That is where the reader cannot act, so every
    /// one of them is re-spanned onto the call that produced it.
    fn report_solution(&mut self, solution: &solver::Solution) {
        for diagnostic in &solution.diagnostics {
            match diagnostic {
                solver::Diagnostic::Ambiguous { name, .. } => {
                    self.err(
                        DukaSemanticError::TypeParamUnresolved(name.clone()),
                        solution.span,
                    );
                }
                solver::Diagnostic::BoundViolated {
                    name,
                    bound,
                    candidate,
                    ..
                } => {
                    self.err(
                        DukaSemanticError::TypeParamBoundViolated(
                            name.clone(),
                            bound.to_string(),
                            candidate.to_string(),
                        ),
                        solution.span,
                    );
                }
                solver::Diagnostic::ArityMismatch {
                    expected, given, ..
                } => {
                    self.err(
                        DukaSemanticError::TypeArgArityMismatch(*expected, *given),
                        solution.span,
                    );
                }
            }
        }
    }

    fn infer_call(&mut self, path: &Path, call_span: Span, args: &[Expr]) -> Option<Multi> {
        if let Path::Base((name, _)) = path
            && name.as_str() == cpar::REQUIRE
        {
            return Some((vec![self.require_value(args)], false));
        }
        let (arg_types, args_var_arg) = self.infer_expr_list(args);
        match path {
            Path::Base((name, _)) => {
                let sig = self.lookup_type(name)?;
                let Some(ft) = Self::as_function(&sig) else {
                    return Some((vec![Type::Any], false));
                };
                self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
                let Some(decl) = self.generic_fns.get(name.as_str()).cloned() else {
                    return Some((ft.returns.into_vec(), ft.return_var_arg));
                };
                let mut solver = solver::Solver::new(self.var_decls(&decl));
                solver.constrain_call(&ft.params, &arg_types, call_span);
                let solution = solver.solve(vec![], call_span);
                self.record_bindings(call_span, &decl, solution);
                self.report_solution(solution);
                self.check_generic_args(&ft.params, &arg_types, args, solution);
                self.discharge_where(&decl.constraints, solution, call_span);

                Some((
                    ft.returns.iter().map(|t| solver.substitute(t)).collect(),
                    ft.return_var_arg,
                ))
            }
            Path::Chain(receiver, PathSuffix::TypeArgs(ty_args, _)) => {
                // generic params are only tracked for named functions, a method
                // reached through a receiver has no declaration to solve against
                let Path::Base((fname, _)) = receiver.as_ref() else {
                    return Some((vec![Type::Any], false));
                }; // FIXME
                let sig = self.lookup_type(fname)?;
                let Some(ft) = Self::as_function(&sig) else {
                    return Some((vec![Type::Any], false));
                };
                self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
                let Some(decl) = self.generic_fns.get(fname.as_str()).cloned() else {
                    return Some((ft.returns.into_vec(), ft.return_var_arg));
                };
                let given: Vec<Type> = ty_args.iter().map(|a| self.resolve_type(a)).collect();
                let mut solver = solver::Solver::new(self.var_decls(&decl));
                let solution = solver.solve(given, call_span);
                self.record_bindings(call_span, &decl, solution);
                self.report_solution(solution);
                self.check_generic_args(&ft.params, &arg_types, args, solution);
                self.discharge_where(&decl.constraints, solution, call_span);

                Some((
                    ft.returns.iter().map(|t| solver.substitute(t)).collect(),
                    ft.return_var_arg,
                ))
            }
            Path::Chain(receiver, PathSuffix::Colon((mname, mspan))) => {
                let (mname, mspan) = (String::from(mname), *mspan);
                self.member_call(receiver, &mname, mspan, call_span, &arg_types, args_var_arg)
            }
            Path::Chain(receiver, PathSuffix::Dot((name, name_span))) => {
                let (name, name_span) = (String::from(name), *name_span);
                if name == csugar::NEW_FUNC
                    && let Some(id) = self.receiver_object(receiver)
                {
                    return Some((vec![self.object_of(id)], false));
                }
                self.member_call(
                    receiver,
                    &name,
                    name_span,
                    call_span,
                    &arg_types,
                    args_var_arg,
                )
            }
            Path::Expr(callee) => {
                let Type::Function(Some(ft)) = self.infer_expr(callee) else {
                    return None;
                };
                self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
                Some((ft.returns.into_vec(), ft.return_var_arg))
            }
            _ => None, // FIXME,
        }
    }

    /// `recv.name(...)`: an object method when the receiver is an object, a
    /// function stored in a record or table otherwise
    fn member_call(
        &mut self,
        receiver: &Path,
        name: &str,
        name_span: Span,
        call_span: Span,
        arg_types: &[Type],
        args_var_arg: bool,
    ) -> Option<Multi> {
        if let Some(id) = self.receiver_object(receiver)
            && let Some(m) = self.find_method(id, name)
        {
            self.links.push(MethodLink {
                call_span,
                name_span,
                decl_span: m.span,
                owner: id,
            });
            self.check_call_arity(&m.sig, arg_types.len(), args_var_arg, call_span);
            return self.return_type_of(&m.sig);
        }
        let recv = self.type_of_path(receiver);
        let Some(ft) = Self::as_function(&self.member_type(&recv, name)) else {
            return None;
        };
        self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
        Some((ft.returns.into_vec(), ft.return_var_arg))
    }

    fn check_call_arity(&mut self, ft: &FunctionType, got: usize, open_ended: bool, span: Span) {
        if open_ended || !self.final_args {
            return;
        }
        // a signature carries only the named parameters, the rest parameter is
        // `var_arg` and says nothing is required beyond them
        let required = ft.params.len();
        if got < required {
            self.err(
                DukaSemanticError::TypeMismatchArg(argument_count(required), argument_count(got)),
                span,
            );
        }
    }
}

/// Whether a type still mentions a type variable, which only happens inside a
/// generic body where the variable is bound at the call site
/// What a generic signature declared: the variables it solves for, and the
/// `where` list that says what the solution has to satisfy.
///
/// Both halves travel together because they are only useful together. The
/// variables are solved at the call site, and the obligations that were left
/// over from the declaration can only be discharged once the solution is known,
/// which needs the clauses that produced them.
#[derive(Debug, Clone)]
struct GenericDecl {
    params: Box<[TypeParam]>,
    constraints: Box<[WhereClause]>,
}

impl std::ops::Deref for GenericDecl {
    type Target = [TypeParam];
    fn deref(&self) -> &[TypeParam] {
        &self.params
    }
}

/// Something a `where` clause asked for that the declaration pass could not
/// decide, because it still mentions a type parameter. The call site is where
/// the arguments are known and this is discharged.
///
/// A concept is carried by its span and re-read there rather than by the value it
/// had at the declaration: at the declaration the value was a stand-in, and
/// keeping it would mean checking the stand-in instead of the concept.
#[derive(Debug, Clone)]
enum Obligation {
    Bound {
        name: Box<str>,
        bound: Type,
        span: Span,
    },
    Concept {
        span: Span,
    },
}

/// The names a `type V = T` clause introduces, in the order they are written.
fn where_bind_names(names: &Destructuring) -> Vec<Name> {
    fn walk(names: &Destructuring, out: &mut Vec<Name>) {
        match names {
            Destructuring::Array(terms) => terms.iter().for_each(|t| walk_term(t, out)),
            Destructuring::Table(terms) => terms.iter().for_each(|t| walk_term(&t.1, out)),
        }
    }
    fn walk_term(term: &DestructingTerm, out: &mut Vec<Name>) {
        match term {
            DestructingTerm::Bind(name) => out.push(name.clone()),
            DestructingTerm::Term(d) => walk(d, out),
        }
    }
    let mut out = vec![];
    walk(names, &mut out);
    out
}

/// The name a `where` bound is filed under inside a type frame.
///
/// A prefix that cannot be written, so a `where U: Point` cannot shadow a
/// parameter the program actually declared. `Type::Param` already reserves a
/// prefix for its own placeholders, so the two cannot collide either.
fn bound_key(param: &str) -> String {
    format!("{BOUND_PARAM_PREFIX}{param}")
}

const BOUND_PARAM_PREFIX: &str = "\u{1}bound\u{1}";

/// Whether a concept names one of these type parameters.
///
/// A bare name in a type position can only appear as a `TypeLit`, so that is the
/// one place a parameter can hide. The walk is over the whole expression because
/// a concept can nest a record or a call inside itself, and it looks at the
/// concept rather than at its answer: `T == int` answers `false`, which says
/// nothing about whether `T` was still undecided.
fn type_expr_names_type_param(expr: &Expr, params: &[Box<str>]) -> bool {
    struct Walk<'a>(&'a [Box<str>], bool);
    impl Visitor for Walk<'_> {
        fn visit_expr(&mut self, expr: &Expr) {
            if let ExprKind::TypeLit(td) = &expr.0 {
                self.1 |= type_desc_names_any(td, self.0);
            }
        }
    }
    let mut w = Walk(params, false);
    expr.visit(&mut w);
    w.1
}

fn type_desc_names_any(td: &TypeDesc, params: &[Box<str>]) -> bool {
    if let TypeDesc::Named(name, _) = td
        && params.iter().any(|p| p.as_ref() == name.as_ref())
    {
        return true;
    }
    td.type_children()
        .into_iter()
        .any(|c| type_desc_names_any(c, params))
}

fn mentions_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::Array(inner) => inner.as_deref().is_some_and(mentions_param),
        Type::Table(k, v) => {
            k.as_deref().is_some_and(mentions_param) || v.as_deref().is_some_and(mentions_param)
        }
        Type::Union(ts) => ts.iter().any(mentions_param),
        Type::TypeTuple(ts) => ts.iter().any(mentions_param),
        Type::TypeTable(fields) => fields.iter().any(|(_, t)| mentions_param(t)),
        Type::Object { args, .. } => args.iter().any(mentions_param),
        Type::Function(Some(ft)) => ft
            .params
            .iter()
            .chain(ft.returns.iter())
            .any(mentions_param),
        Type::Rec(inner) => mentions_param(inner),
        _ => false,
    }
}

/// `a.b.c = v` splits into the record owner `a.b` and the member `c`
fn assign_target(target: &Path) -> Option<(&Path, &str, Span)> {
    let Path::Chain(owner, PathSuffix::Dot((key, key_span))) = target else {
        return None;
    };
    match owner.as_ref() {
        Path::Base(_) | Path::Chain(..) => Some((owner, key, *key_span)),
        _ => None,
    }
}

/// Names an `export` statement publishes to the importer
fn exported_names(stmt: &StmtKind) -> Vec<(Box<str>, Span)> {
    match stmt {
        StmtKind::Define(names, ..) => names
            .iter()
            .map(|(((name, span), _, _), _)| (name.clone().into_boxed_str(), *span))
            .collect(),
        StmtKind::Function(Path::Base((name, span)), ..) => {
            vec![(name.clone().into_boxed_str(), *span)]
        }
        StmtKind::Function(..) => vec![],
        StmtKind::Assign(targets, _) => targets
            .iter()
            .filter_map(|target| match target {
                Path::Base((name, span)) => Some((name.clone().into_boxed_str(), *span)),
                Path::Chain(base, PathSuffix::Dot((name, span))) => match base.as_ref() {
                    Path::Base((base, _)) if base == name => {
                        Some((name.clone().into_boxed_str(), *span))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

fn inferred_init(ty: Type) -> Type {
    if ty == Type::Nil { Type::Any } else { ty }
}

fn argument_count(n: usize) -> String {
    format!("{n} argument{}", if n == 1 { "" } else { "s" })
}

pub(crate) fn substitute_params(ty: &Type, subst: &HashMap<Box<str>, Type>) -> Type {
    match ty {
        Type::TypeTable(t) => Type::TypeTable(
            t.iter()
                .map(|(k, v)| (k.clone(), Box::new(substitute_params(v, subst))))
                .collect(),
        ),
        Type::TypeTuple(v) => {
            Type::TypeTuple(v.iter().map(|t| substitute_params(t, subst)).collect())
        }
        //Type::TypeTable()
        Type::Param(name) => subst
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::Param(name.clone())),
        Type::Array(Some(inner)) => Type::Array(Some(Box::new(substitute_params(inner, subst)))),
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v) => Type::Table(
            k.as_deref().map(|k| Box::new(substitute_params(k, subst))),
            v.as_deref().map(|v| Box::new(substitute_params(v, subst))),
        ),
        Type::Union(ts) => Type::Union(ts.iter().map(|t| substitute_params(t, subst)).collect()),
        Type::Object {
            id,
            name,
            base,
            args,
        } => Type::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args.iter().map(|t| substitute_params(t, subst)).collect(),
        },
        Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
            params: ft
                .params
                .iter()
                .map(|t| substitute_params(t, subst))
                .collect(),
            returns: ft
                .returns
                .iter()
                .map(|t| substitute_params(t, subst))
                .collect(),
            var_arg: ft.var_arg,
            return_var_arg: ft.return_var_arg,
        })),
        Type::Rec(inner) => Type::Rec(Box::new(substitute_params(inner, subst))),
        other => other.clone(),
    }
}

/// Substitution that stops at a `Rec`. A recursive type is the boundary of
/// itself: the back edge inside its body is already bound, so a substitution
/// made from outside must not reach in. Without that boundary every projection
/// would deepen the type instead of answering with the same recursive type,
/// which is what `LList(int)[1][1]` should be.
pub(crate) fn substitute_back_edges(ty: &Type, subst: &HashMap<Box<str>, Type>) -> Type {
    match ty {
        Type::TypeTable(t) => Type::TypeTable(
            t.iter()
                .map(|(k, v)| (k.clone(), Box::new(substitute_back_edges(v, subst))))
                .collect(),
        ),
        Type::TypeTuple(v) => {
            Type::TypeTuple(v.iter().map(|t| substitute_back_edges(t, subst)).collect())
        }
        Type::Param(name) => subst
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::Param(name.clone())),
        Type::Array(Some(inner)) => {
            Type::Array(Some(Box::new(substitute_back_edges(inner, subst))))
        }
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v) => Type::Table(
            k.as_deref()
                .map(|k| Box::new(substitute_back_edges(k, subst))),
            v.as_deref()
                .map(|v| Box::new(substitute_back_edges(v, subst))),
        ),
        Type::Union(ts) => {
            Type::Union(ts.iter().map(|t| substitute_back_edges(t, subst)).collect())
        }
        Type::Object {
            id,
            name,
            base,
            args,
        } => Type::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
        },
        Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
            params: ft
                .params
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
            returns: ft
                .returns
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
            var_arg: ft.var_arg,
            return_var_arg: ft.return_var_arg,
        })),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser, SourceName};

    use crate::{
        analyzer::ScopeAnalyzer, analyzer::TypeChecker, lexer::LexerWithMacro, parser::Parser,
    };

    fn check(source: &str) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk =
            Parser::parse(stream, duka_shared::config::DukaParserConfig::default()).unwrap();
        dbg!(&chunk);
        dbg!(
            ScopeAnalyzer
                .chain(TypeChecker)
                .analyze(&chunk, Default::default())
                .1
                .collect()
        )
    }

    fn is_error(errors: &[DukaSpannedError]) -> bool {
        errors.iter().any(|e| {
            matches!(
                e.kind,
                DukaErrorKind::Semantic(
                    DukaSemanticError::TypeMismatchEqual(..)
                        | DukaSemanticError::TypeMismatchReturn(..)
                        | DukaSemanticError::TypeMismatchArg(..)
                        | DukaSemanticError::TypeParamBoundViolated(..)
                )
            )
        })
    }

    /// The rendered type of every type alias the source declares, which is the
    /// only way to see what a type annotation actually solved to.
    fn alias_types(source: &str) -> std::collections::HashMap<String, String> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let pipeline = crate::analyzer::ScopeAnalyzer
            .chain(crate::analyzer::BasicAnalyzer)
            .chain(crate::analyzer::TypeEval)
            .chain(TypeChecker);
        let ((_, analysis), errors) = pipeline.analyze(&chunk, Default::default());
        let errors: Vec<_> = errors.collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let mut out = std::collections::HashMap::new();
        for scope in analysis.symbols.scopes() {
            for (name, syms) in &scope.symbols {
                if let Some(last) = syms.last() {
                    out.insert(
                        name.to_string(),
                        last.ty.as_deref().unwrap_or("").to_owned(),
                    );
                }
            }
        }
        out
    }

    #[test]
    fn returns_of_different_lengths_line_up_by_position() {
        let types = alias_types(
            "function C<T>(a: T, ...)\n\
             \x20   if a == nil then return a end\n\
             \x20   return a, 1, 2\n\
             end\n",
        );
        assert_eq!(
            types.get("C").map(String::as_str),
            Some("function(T?, ...) -> (T?, int, int, ...)")
        );
    }

    #[test]
    fn a_return_that_always_fills_the_same_slots_is_not_var_arg() {
        let types = alias_types("function B<T>(a: T, ...)\n    return a, 1\nend\n");
        assert_eq!(
            types.get("B").map(String::as_str),
            Some("function(T?, ...) -> (T?, int)")
        );
    }

    #[test]
    fn a_varying_tail_needs_the_rest_parameter_to_exist() {
        let with = alias_types(
            "function D(a, ...)\n\
             \x20   if a == nil then return end\n\
             \x20   return a, 1, 2\n\
             end\n",
        );
        assert_eq!(
            with.get("D").map(String::as_str),
            Some("function(any, ...) -> (any, int, int, ...)")
        );
        let without = alias_types(
            "function E(a)\n\
             \x20   if a == nil then return end\n\
             \x20   return a, 1\n\
             end\n",
        );
        assert_eq!(
            without.get("E").map(String::as_str),
            Some("function(any) -> (any, int)")
        );
    }

    #[test]
    fn two_returns_of_one_value_do_not_become_two() {
        let types = alias_types(
            "function F(a)\n\
             \x20   if a == nil then return 1 end\n\
             \x20   return 2\n\
             end\n",
        );
        assert_eq!(
            types.get("F").map(String::as_str),
            Some("function(any) -> int")
        );
    }

    #[test]
    fn a_return_protocol_says_what_the_slots_are() {
        let types = alias_types(
            "@returns(result) function f(a: int)\n\
             \x20   return 1, a\n\
             end\n\
             @returns(exit) function g()\n\
             \x20   return\n\
             end\n\
             function h() return 1 end\n",
        );
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function(int?) -> (bool, ...)"),
            "the first slot says whether it succeeded and the rest are its own values"
        );
        assert_eq!(
            types.get("g").map(String::as_str),
            Some("function()"),
            "nothing comes back, so there is no question of what it returned"
        );
        assert_eq!(
            types.get("h").map(String::as_str),
            Some("function() -> int"),
            "a declaration that said nothing keeps what it inferred"
        );
    }

    #[test]
    fn a_return_protocol_wins_over_what_was_written() {
        let types = alias_types(
            "@returns(result) function f(a: int): (int, string)\n\
             \x20   return 1, \"s\"\n\
             end\n",
        );
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function(int?) -> (bool, ...)")
        );
    }

    #[test]
    fn a_keywordish_declaration_carries_that_on_the_symbol() {
        let lexer = LexerWithMacro::new(
            Cursor::new("@keywordish function f() end\n"),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let ((_, analysis), _) = ScopeAnalyzer.analyze(&chunk, Default::default());
        let declared = analysis.symbols.lookup_named("f").expect("f is declared");
        assert_eq!(
            declared.attribute,
            Some(duka_shared::docs::Attribute::Keywordish)
        );
        let plain = alias_types("function g() end\n");
        assert_eq!(plain.get("g").map(String::as_str), Some("function()"));
    }

    #[test]
    fn a_recursive_tail_is_the_same_recursive_type_however_often_it_is_read() {
        let types = alias_types(
            "type function LList(T) = [T, LList(T)?]\n\
             type A1 = LList(int)[1]\n\
             type A2 = LList(int)[1][1]\n\
             type A3 = LList(int)[1][1][1]\n",
        );
        let tail = types.get("A1").expect("A1");
        assert_eq!(tail, "rec [int, LList?]?");
        assert_eq!(types.get("A2").map(String::as_str), Some(tail.as_str()));
        assert_eq!(types.get("A3").map(String::as_str), Some(tail.as_str()));
    }

    #[test]
    fn reading_a_recursive_tail_does_not_leak_the_recursion_placeholder() {
        let types =
            alias_types("type function LList(T) = [T, LList(T)?]\ntype A2 = LList(int)[1][1]\n");
        let tail = types.get("A2").expect("A2");
        assert!(tail.starts_with("rec "), "{tail}");
        assert!(!tail.contains("__rec_"), "{tail}");
    }

    #[test]
    fn each_instantiation_of_a_recursive_type_function_keeps_its_own_tail() {
        let types = alias_types(
            "type function LList(T) = [T, LList(T)?]\n\
             type A = LList(int)[1]\n\
             type B = LList(string)[1]\n",
        );
        assert_eq!(
            types.get("A").map(String::as_str),
            Some("rec [int, LList?]?")
        );
        assert_eq!(
            types.get("B").map(String::as_str),
            Some("rec [string, LList?]?")
        );
    }

    #[test]
    fn a_record_reads_as_a_table_in_type_context() {
        let types = alias_types(
            "type D1 = { a: [int, string] }[\"a\"][1]\ntype D2 = { a: [int, string] }.a[1]\n",
        );
        assert_eq!(types.get("D1").map(String::as_str), Some("string"));
        assert_eq!(types.get("D2").map(String::as_str), Some("string"));
    }

    fn parse_err(source: &str) -> bool {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .is_err()
    }

    use duka_shared::errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError};

    #[test]
    fn accepts_int_for_num() {
        let errors = check("local n: num = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_int_for_float() {
        let errors = check("local n: float = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_float_for_int() {
        let errors = check("local n: int = 1.5");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_float_for_num() {
        let errors = check("local n: num = 1.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_string_for_int() {
        let errors = check("local n: int = \"hi\"");
        assert!(is_error(&errors), "expected a type error, got {:?}", errors);
    }

    #[test]
    fn allows_any_to_silent_unknown() {
        let errors = check("local n: int = some_unknown_call()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_wrong_return_type() {
        let errors = check("function f(): int return \"no\" end");
        assert!(
            is_error(&errors),
            "expected return type error, got {:?}",
            errors
        );
    }

    #[test]
    fn accepts_correct_return_type() {
        let errors = check("function f(): int return 42 end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_float_literal_bitand() {
        let errors = check("local x = 5.5 & 1");
        assert!(
            is_error(&errors),
            "expected float bitand error, got {:?}",
            errors
        );
    }

    #[test]
    fn allows_float_operand_unknown_meta() {
        let errors = check("local x = a & 1.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_int_bitand() {
        let errors = check("local n: int = 5 & 3");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn propagates_param_type_to_body() {
        let errors = check("function f(a: int) a = \"hi\" end");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn propagates_local_type_to_reassign() {
        let errors = check("local n: num = 1 n = \"hi\"");
        assert!(
            is_error(&errors),
            "expected assignment type error {:?}",
            errors
        );
    }

    #[test]
    fn infers_local_type_from_literal() {
        let errors = check("local n = 1 n = \"hi\"");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_correct_reassign() {
        let errors = check("local n: num = 1 n = 2.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn infers_constant_type() {
        let errors = check("@const local N = 3");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_union_member() {
        let errors = check("local x: int | nil = 5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_union_non_member() {
        let errors = check("local x: int | nil = \"hi\"");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_union_subtype_member() {
        let errors = check("local x: float | nil = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_empty_type_annotation() {
        assert!(
            parse_err("local a: = 1"),
            "`local a: = 1` must fail to parse"
        );
    }

    #[test]
    fn rejects_trailing_union_member() {
        assert!(
            parse_err("local a: int | = 1"),
            "`int | = 1` must fail to parse"
        );
    }

    #[test]
    fn accepts_fn_signature_annotation() {
        let errors = check("local cb: fn(int, string) -> bool = function(a, b) return true end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_vararg_annotation() {
        let errors = check("local cb: fn(int, ...) -> int = function(a, ...) return 1 end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_empty_signature() {
        let errors = check("local cb: fn() = function() end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_fn_missing_param_type() {
        assert!(parse_err("local cb: fn(int,) -> int = 1"));
    }

    #[test]
    fn rejects_fn_bad_param_syntax() {
        assert!(parse_err("local x: fn(int -> int) = 5"));
    }

    #[test]
    fn accepts_fn_multi_return() {
        let errors = check("local cb: fn(int) -> (int, string) = function(a) return a, \"x\" end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_vararg_return_tuple() {
        let errors =
            check("local cb: fn(int, ...) -> (int, ...) = function(a, ...) return 1, ... end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn expands_multi_return_as_call_args() {
        let errors =
            check("function pair() return 1, \"s\" end function take(a, b, c) end take(0, pair())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_too_few_call_args() {
        let errors = check("function f(a, b) end f(1)");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn deep_member_assign_refines_reads() {
        let errors = check("local a = {b = {c = {}}} a.b.c.d = 1 local s: string = a.b.c.d");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn tuple_literal_index_types() {
        let errors = check("local t = {1, \"s\"} local n: int = t[2]");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn index_then_field_keeps_type() {
        let errors = check("local t = {{x = 1}} local s: string = t[1].x");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn record_literal_key_index_keeps_type() {
        let errors = check("local M = {} M.n = 1 local s: string = M[\"n\"]");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn object_member_assign_must_match() {
        let errors = check("object A\n  n: int\nend\nlocal a = A.new()\na.n = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn object_member_assign_accepts_declared() {
        let errors = check("object A\n  n: int\nend\nlocal a = A.new()\na.n = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn record_field_reassign_must_match() {
        let errors = check("local M = {} M.n = 1 M.n = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn nil_init_keeps_annotated_type() {
        let errors = check("local v: int = nil v = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn accepts_extra_call_args() {
        let errors = check("function f(a) end f(1, 2, 3)");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_vararg_param_call_args() {
        let errors = check("function f(a, ...) end f(1, 2, 3)");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_missing_fixed_args_before_vararg() {
        let errors = check("function f(a, b, ...) end f(1)");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn skips_arity_for_open_vararg_call() {
        let errors = check("function g(...): ... return ... end function f(a, b) end f(g())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn expands_multi_return_in_table_and_array() {
        let errors =
            check("function pair() return 1, \"s\" end local a = {pair()} local b = {pair(), 3}");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn calls_result_of_call() {
        let errors = check(
            "function inner(): int return 1 end function outer(): fn() -> int return inner end local n: int = (outer())()",
        );
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn nilable_annotation_keeps_signature() {
        let errors = check("local x: fn(int) = function(a) end return x()");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn rejects_wrong_type_from_collected_multi_return() {
        let errors = check(
            "function pair() return 1, \"s\" end function g() return pair() end local a, b: fn() = g()",
        );
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn accepts_fn_bare_vararg_return() {
        let errors = check("local cb: fn(int) -> ... = function(a) return ... end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_fn_bad_multi_return_syntax() {
        assert!(parse_err("local cb: fn() -> (int,) = 1"));
    }

    #[test]
    fn rejects_fn_vararg_return_mid_list() {
        assert!(parse_err("local cb: fn() -> (int, ..., string) = 1"));
    }

    fn check_with(source: &str, nonnilable: bool) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk = Parser::parse(
            stream,
            duka_shared::config::DukaParserConfig {
                default_nonnilable: nonnilable,
                ..Default::default()
            },
        )
        .unwrap();
        dbg!(
            ScopeAnalyzer
                .chain(TypeChecker)
                .analyze(&chunk, Default::default())
                .1
                .collect()
        )
    }

    #[test]
    fn bang_strips_nil_from_atom() {
        let errors = check_with("local a: int! = nil", false);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with("local a: int! = 5", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn union_bang_keeps_other_nil() {
        let errors = check_with("local a: int | string! = nil", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn paren_group_bang_strips_whole_union() {
        let errors = check_with("local a: (int | string)! = nil", false);
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn question_suffix_adds_nil() {
        let errors = check_with("local a: int? = nil", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_rejects_nil() {
        let errors = check_with("local a: int = nil", true);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with("local a: int = 5", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_accepts_question() {
        let errors = check_with("local a: int? = nil", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_accepts_union_nil() {
        let errors = check_with("local a: int | nil = nil", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_union_without_nil_rejects() {
        let errors = check_with("local a: int | string = nil", true);
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn object_base_resolves() {
        let errors = check("object A\nend\nobject B : A\nend");
        assert!(
            !errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(
                    DukaSemanticError::UnknownBase(..) | DukaSemanticError::CircularExtends(..)
                )
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn generic_param_is_not_any() {
        let ok = check("function id<T>(x: T): T return x end local a: int = id(1)");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("function id<T>(x: T): T return x end local a: string = id(1)");
        assert!(is_error(&bad), "T must not degrade to any: {:?}", bad);
    }

    #[test]
    fn generic_return_annotation_is_substituted() {
        let errors = check("function id<T>(x: T): T return x end local a: string = id(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_bound_is_enforced_after_inference() {
        let errors = check("function bnd<T: int>(x: T): T return x end local a = bnd(\"s\")");
        assert!(is_error(&errors), "expected bound error {:?}", errors);
    }

    #[test]
    fn a_bound_violation_names_the_type_argument() {
        let errors = check("function bnd<T: int>(x: T): T return x end local a = bnd(\"s\")");
        let message = errors
            .iter()
            .find_map(|e| match &e.kind {
                DukaErrorKind::Semantic(DukaSemanticError::TypeParamBoundViolated(
                    name,
                    bound,
                    candidate,
                )) => Some(format!("{name} {bound} {candidate}")),
                _ => None,
            })
            .expect("a bound violation");
        assert_eq!(message, "T int? string");
        let help = errors
            .iter()
            .find(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeParamBoundViolated(..))
                )
            })
            .map(|e| e.kind.get_help())
            .unwrap_or_default();
        assert!(help.contains("'T'"), "{help}");
        assert!(help.contains("int?"), "{help}");
    }

    #[test]
    fn generic_type_function_param_participates() {
        let src = "type function Boxed(t) return { value = t } end function unbox<T>(x: Boxed(T)): T return x.value end";
        let ok = check(&format!("{src} local a: int = unbox({{ value = 1 }})"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local a: string = unbox({{ value = 1 }})"));
        assert!(
            is_error(&bad),
            "type function must see the real T: {:?}",
            bad
        );
    }

    #[test]
    fn generic_param_default_is_used_when_unresolved() {
        let src = "function f<T = int>(): T return 1 end";
        let ok = check(&format!("{src} local a: int = f()"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local a: string = f()"));
        assert!(is_error(&bad), "the default must decide T: {:?}", bad);
    }

    #[test]
    fn generic_explicit_type_argument_beats_default() {
        let errors =
            check("function f<T = int>(): T return \"s\" end local a: string = f.<string>()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_explicit_type_argument_checks_the_argument() {
        let ok = check("function id<T>(x: T): T return x end local a: int = id.<int>(1)");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("function id<T>(x: T): T return x end local a: int = id.<int>(\"s\")");
        assert!(
            is_error(&bad),
            "explicit type argument must be checked: {:?}",
            bad
        );
    }

    #[test]
    fn generic_shape_mismatch_is_reported_once() {
        let errors = check("function f<T>(x: array<T>) end f(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_opaque_argument_is_not_checked() {
        let errors = check("function f<T>(x: T) end f(unknown())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_argument_type_must_match_inference() {
        let errors = check("function f<T>(x: array<T>, y: T) end f([1], 2)");
        assert!(!is_error(&errors), "{:?}", errors);
        let bad = check("function f<T>(x: T, y: T) end f(1, \"s\")");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn nested_generic_annotation_parses() {
        let ok = check("local a: array<array<int>> = { { 1 } }");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("local a: array<array<int>> = { { \"s\" } }");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn deeply_nested_generic_annotation_parses() {
        let ok = check("local a: array<array<array<int>>> = { { { 1 } } }");
        assert!(!is_error(&ok), "{:?}", ok);
    }

    #[test]
    fn generic_default_may_use_another_param() {
        let errors =
            check("function f<T, U = array<T>>(): U return { 1 } end local a: array<int> = f()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_bound_and_default_together() {
        let errors = check("function f<T: num = int>(x: T): T return x end local a: int = f(1)");
        assert!(!is_error(&errors), "{:?}", errors);
        let bad = check("function f<T: num = int>(x: T): T return x end local a: int = f(\"s\")");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn generic_default_and_nested_angle_do_not_panic() {
        for src in [
            "function f<T = int>(): T return 1 end local a: int = f()",
            "local a: array<array<int>> = 1",
        ] {
            let _ = check(src);
        }
    }

    #[test]
    fn generic_param_default_is_not_unresolved_yet() {
        // `T = int` is not parsed yet, the default path lands with the solver
        let errors = check("function f<T>(x: T): T return x end local a: string = f(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn object_circular_base() {
        let errors = check("object A : B\nend\nobject B : A\nend");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::CircularExtends(..))
            )),
            "{:?}",
            errors
        );
    }

    fn analyze(source: &str) -> (Vec<DukaSpannedError>, crate::analyzer::ScopeAnalysis) {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let sa = ScopeAnalyzer.chain(TypeChecker);
        let (data, errors) = sa.analyze(&chunk, Default::default());
        (errors.collect(), data.1)
    }

    #[test]
    fn object_typed_instance_no_error() {
        let (errors, analysis) = analyze("object A\nend\nlocal a: A = A.new()\nlocal b = a");
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(analysis.objects.len(), 1);
    }

    #[test]
    fn object_unknown_annotation() {
        let (errors, _) = analyze("local a: NoSuch = 1");
        assert!(
            !errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{:?}",
            errors
        );
    }

    /// The other half of the same rule: a signature *is* a commitment, so a name
    /// in one that resolves to nothing is a mistake rather than a name for
    /// whatever it turns out to be. Answering `any` here would make every bound
    /// and every obligation pass and hide the typo until something unrelated
    /// failed to match.
    #[test]
    fn an_unknown_name_in_a_signature_is_reported() {
        let (errors, _) = analyze(
            r#"
function f(x: NoSuch): NoSuch
    return x
end
"#,
        );
        assert_eq!(
            errors
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
                ))
                .count(),
            2,
            "{errors:?}"
        );
    }

    /// A type parameter of the function itself is a name in a signature, and it
    /// resolves, so it is not this error.
    #[test]
    fn a_type_parameter_in_a_signature_is_not_unknown() {
        let (errors, _) = analyze(
            r#"
function id<T>(x: T): T
    return x
end
"#,
        );
        assert!(
            !errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{errors:?}"
        );
    }

    /// The name can be nested: `array<NoSuch>` is just as much a mistake as
    /// `NoSuch`, and a type is a tree rather than a single token.
    #[test]
    fn an_unknown_name_nested_in_a_signature_is_reported() {
        let (errors, _) = analyze(
            r#"
function f(x: array<NoSuch>)
    return x
end
"#,
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn method_call_links_to_decl() {
        let src = r#"
object A
    function :foo()
        return 1
    end
end
local a: A = A.new()
a:foo()
        "#;
        let (errors, analysis) = analyze(src);
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(analysis.links.len(), 1);
        let link = &analysis.links[0];
        assert_eq!(analysis.objects[link.owner].name.as_ref(), "A");
        assert_eq!(analysis.objects[link.owner].methods.len(), 1);
        let decl = analysis.objects[link.owner].methods[0].span;
        assert_eq!(link.decl_span, decl);
    }

    #[test]
    fn static_factory_call() {
        let (errors, _) = analyze(
            r#"
object a
    function foo()
        return 1
    end
end
local x = a.foo()
"#,
        );
        assert!(errors.is_empty(), "{:?}", errors);
    }
}
