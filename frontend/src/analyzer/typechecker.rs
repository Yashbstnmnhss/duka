use std::collections::HashMap;
use std::ops::BitOr;
use std::sync::{Arc, Mutex};

use duka_shared::{
    constants::csugar,
    dtype::{FunctionType, ObjectId, Type},
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::{
    analyzer::{
        AnalyzerData, CallResults, TypeFn, Visit, Visitor,
        eval::{EvalCtx, EvalCtxInit},
        modules::DukaSourceProvider,
        objects::{MethodLink, ObjectMethod, ObjectType},
        tyval::TypeValue,
    },
    parser::ast::{
        DukaChunk, Expr, ExprKind, Field, FuncBody, Param, Path, PathSuffix, StmtKind,
        TypeDescriptor, TypeFnValue, TypeParam,
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
    pub fn analyze_with_modules<'a>(
        &self,
        chunk: &'a DukaChunk,
        mut data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> (AnalyzerData, impl Iterator<Item = DukaSpannedError>) {
        let source = Arc::new(chunk.source_info.clone());
        let mut collect = TypeCheckerCtx::new(source.clone(), &data, provider);
        collect.collect_mode = true;
        chunk.visit(&mut collect);
        let inferred = collect.collected_returns;
        let mut ctx = TypeCheckerCtx::new(source, &data, provider);
        ctx.inferred_returns = inferred;
        chunk.visit(&mut ctx);
        let mut links = std::mem::take(&mut ctx.links);
        let mut uniq: Vec<MethodLink> = Vec::with_capacity(links.len());
        for link in links {
            if !uniq.iter().any(|u| {
                u.call_span == link.call_span
                    && u.name_span == link.name_span
                    && u.decl_span == link.decl_span
                    && u.owner == link.owner
            }) {
                uniq.push(link);
            }
        }
        links = uniq;
        let (errors, backfills) = ctx.finish();
        for (span, ty) in backfills {
            data.1.symbols.set_type_at_span(span, ty);
        }
        data.1.links = links;
        (data, errors)
    }
}

struct TypeCheckerCtx<'a> {
    source: Arc<SourceInfo>,
    ret_stack: Vec<Option<Type>>,
    types: Vec<HashMap<Box<str>, Type>>,
    viewer: SymbolTableViewer<'a>,
    objects: &'a [ObjectType],
    aliases: &'a [(Box<str>, TypeDescriptor)],
    type_fns: &'a [TypeFn],
    call_cache: Arc<Mutex<CallResults>>,
    modules: &'a HashMap<Box<str>, crate::analyzer::modules::ModuleType>,
    provider: Option<&'a dyn DukaSourceProvider>,
    generic_fns: HashMap<Box<str>, Box<[TypeParam]>>,
    links: Vec<MethodLink>,
    errors: Vec<DukaSpannedError>,
    backfills: Vec<(Span, Box<str>)>,

    collect_mode: bool,
    ret_collect: Vec<Vec<Type>>,
    finished_returns: Vec<Box<[Type]>>,
    collected_returns: HashMap<Box<str>, Box<[Type]>>,

    inferred_returns: HashMap<Box<str>, Box<[Type]>>,
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
            call_cache: data.1.call_cache.clone(),
            modules: &data.1.modules,
            provider,
            generic_fns: HashMap::new(),
            links: vec![],
            errors: vec![],
            backfills: vec![],
            collect_mode: false,
            ret_collect: vec![],
            finished_returns: vec![],
            collected_returns: HashMap::new(),
            inferred_returns: HashMap::new(),
        }
    }

    fn finish(
        self,
    ) -> (
        impl Iterator<Item = DukaSpannedError> + use<>,
        Vec<(Span, Box<str>)>,
    ) {
        (self.errors.into_iter(), self.backfills)
    }

    fn err(&mut self, v: DukaSemanticError, span: Span) {
        if self.collect_mode {
            return;
        }
        self.errors.push(DukaSpannedError {
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
        if let Some(sym) = self.viewer.lookup(name) {
            if let SymbolType::Constant(cv) = &sym.symbol_type {
                return Some(cv.type_of());
            }
        }
        None
    }

    fn declare(&mut self, name: &str, span: Span, ty: Type) {
        self.types
            .last_mut()
            .expect("there must be a type frame")
            .insert(name.into(), ty.clone());
        if !self.collect_mode {
            self.backfills.push((span, ty.to_string().into_boxed_str()));
        }
    }

    fn declare_params(&mut self, body: &FuncBody) {
        for param in body.0.iter() {
            match param {
                Param::Typed((name, span), ty) => {
                    let ty = self.resolve_type(ty, Some(*span));
                    self.declare(name, *span, ty)
                }
                Param::Name((name, span)) => self.declare(name, *span, Type::Any),
                Param::Var(_) => {}
            }
        }
    }

    fn resolve_type(&mut self, ty: &TypeDescriptor, _at: Option<Span>) -> Type {
        match ty {
            TypeDescriptor::Pure(t) => t.clone(),
            other => {
                let init = EvalCtxInit {
                    source: self.source.clone(),
                    viewer: self.viewer.clone(),
                    type_fns: self.type_fns,
                    objects: self.objects,
                    aliases: self.aliases,
                    results: self.call_cache.clone(),
                    modules: Some(self.modules),
                    provider: self.provider,
                    report_errors: true,
                };
                let mut hook = |t: &TypeDescriptor| match t {
                    TypeDescriptor::TypeOf { expr, .. } => {
                        Some(TypeValue::Type(self.infer_expr(expr)))
                    }
                    TypeDescriptor::Named(name, _) => self.lookup_type(name).map(TypeValue::Type),
                    _ => None,
                };
                let mut ev = EvalCtx::new(init).with_hook(Some(&mut hook));
                let r = ev.eval_type(other).concretize();
                let errs = std::mem::take(&mut ev.errors);
                self.errors.extend(errs);
                r
            }
        }
    }
    fn resolve_module_type(&self, name: &str) -> Option<&'a crate::analyzer::modules::ModuleType> {
        crate::analyzer::modules::resolve_module_type(
            self.modules,
            name,
            self.source.name.as_deref(),
            self.provider?,
        )
    }
}

impl TypeCheckerCtx<'_> {
    fn fn_type(&mut self, body: &FuncBody) -> Type {
        self.fn_type_ret(body, None)
    }

    fn fn_type_ret(&mut self, body: &FuncBody, inferred: Option<&Box<[Type]>>) -> Type {
        let FuncBody(params, type_params, ret, _) = body;
        let names: Vec<&str> = type_params
            .iter()
            .map(|TypeParam((n, _), _)| n.as_str())
            .collect();
        let returns: Box<[Type]> = match ret {
            Some(_) => ret
                .iter()
                .map(|t| {
                    let normalized = normalize_generic_names(t, &names);
                    self.resolve_type(&normalized, None)
                })
                .collect(),
            None => inferred
                .map(|r| r.iter().cloned().collect())
                .unwrap_or_default(),
        };
        Type::Function(Some(FunctionType {
            params: params
                .iter()
                .map(|p| match p {
                    Param::Typed(_, t) => {
                        let normalized = normalize_generic_names(t, &names);
                        self.resolve_type(&normalized, None)
                    }
                    _ => Type::Any,
                })
                .collect(),
            var_arg: body.has_var_arg(),
            returns,
            return_var_arg: false,
        }))
    }
}

fn normalize_generic_names(tv: &TypeDescriptor, names: &[&str]) -> TypeDescriptor {
    match tv {
        TypeDescriptor::Named(name, _) if names.contains(&name.as_ref()) => {
            TypeDescriptor::Pure(Type::Param(name.clone()))
        }
        TypeDescriptor::Named(..) => tv.clone(),
        TypeDescriptor::Generic { name, args, span } => TypeDescriptor::Generic {
            name: name.clone(),
            args: args
                .iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            span: *span,
        },
        TypeDescriptor::TypeCall { name, args, span } => TypeDescriptor::TypeCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            span: *span,
        },
        TypeDescriptor::Access {
            base,
            member,
            args,
            span,
        } => TypeDescriptor::Access {
            base: Box::new(normalize_generic_names(base, names)),
            member: member.clone(),
            args: args.as_ref().map(|a| {
                a.iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect()
            }),
            span: *span,
        },
        TypeDescriptor::TypeOf { .. } => tv.clone(),
        TypeDescriptor::Array(e) => TypeDescriptor::Array(
            e.as_deref()
                .map(|e| Box::new(normalize_generic_names(e, names))),
        ),
        TypeDescriptor::Table(k, v) => TypeDescriptor::Table(
            k.as_deref()
                .map(|k| Box::new(normalize_generic_names(k, names))),
            v.as_deref()
                .map(|v| Box::new(normalize_generic_names(v, names))),
        ),
        TypeDescriptor::Union(ts) => TypeDescriptor::Union(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
        ),
        TypeDescriptor::TypeTuple(ts) => TypeDescriptor::TypeTuple(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
        ),
        TypeDescriptor::TypeTable(ts) => TypeDescriptor::TypeTable(
            ts.iter()
                .map(|(k, v)| (k.clone(), normalize_generic_names(v, names)))
                .collect(),
        ),
        TypeDescriptor::Function(ft) => TypeDescriptor::Function(ft.as_ref().map(|ft| {
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
        other => other.clone(),
    }
}

impl<'a> Visitor for TypeCheckerCtx<'a> {
    fn visit_stmt(&mut self, stmt: &crate::parser::ast::Stmt) {
        match &stmt.0 {
            StmtKind::Define(names, exprs, _) => {
                for (idx, (((name, span), _attrs, ty), _)) in names.iter().enumerate() {
                    let actual = exprs.get(idx).map(|e| self.infer_expr_const(e));
                    let declared = ty.as_ref().map(|t| self.resolve_type(t, Some(*span)));
                    if let Some(declared) = &declared
                        && let Some((actual, cv)) = &actual
                        && !declared.accepts_value(actual, cv.as_ref())
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                declared.to_string(),
                                actual.to_string(),
                            ),
                            exprs[idx].1,
                        );
                    }
                    let inferred = actual
                        .as_ref()
                        .map(|(t, cv)| {
                            if matches!(t, Type::Nil) && cv.as_ref() == Some(&ConstValue::Nil) {
                                Type::Any
                            } else {
                                t.clone()
                            }
                        })
                        .unwrap_or(Type::Any);
                    self.declare(name, *span, declared.unwrap_or(inferred));
                }
            }
            StmtKind::Assign(targets, exprs) => {
                for (idx, target) in targets.iter().enumerate() {
                    let some_expr = exprs.get(idx);
                    if let Path::Base((name, span)) = target
                        && let Some(expr) = some_expr
                        && self.lookup_type(name).is_none()
                    {
                        let actual = self.infer_expr_const(expr);
                        self.declare(name, *span, actual.0);
                        continue;
                    }
                    if let Path::Base((name, _)) = target
                        && let Some(expr) = some_expr
                        && let Some(declared) = self.lookup_type(name)
                    {
                        let (actual, cv) = self.infer_expr_const(expr);
                        if !declared.accepts_value(&actual, cv.as_ref()) && actual != Type::Any {
                            self.err(
                                DukaSemanticError::TypeMismatchEqual(
                                    declared.to_string(),
                                    actual.to_string(),
                                ),
                                expr.1,
                            );
                        }
                    }
                }
            }
            StmtKind::Return(items) => {
                if self.collect_mode {
                    let collected: Vec<Type> = items.iter().map(|e| self.infer_expr(e)).collect();
                    if let Some(buf) = self.ret_collect.last_mut() {
                        buf.extend(collected);
                    }
                }
                let ret = self.ret_stack.last().cloned().flatten();
                if let Some(ret) = ret {
                    for e in items {
                        let (actual, cv) = self.infer_expr_const(e);
                        if !ret.accepts_value(&actual, cv.as_ref()) && actual != Type::Any {
                            self.err(
                                DukaSemanticError::TypeMismatchReturn(
                                    ret.to_string(),
                                    actual.to_string(),
                                ),
                                e.1,
                            );
                        }
                    }
                }
            }
            StmtKind::Function(path, _, body, _) => {
                if let Path::Base((name, span)) = path {
                    if !body.1.is_empty() {
                        self.generic_fns
                            .insert(name.clone().into_boxed_str(), body.1.clone());
                    }
                    let ty = match &body.2 {
                        Some(_) => self.fn_type(body),
                        None => {
                            let inferred = self.inferred_returns.get(name.as_str()).cloned();
                            match inferred {
                                Some(r) => self.fn_type_ret(body, Some(&r)),
                                None => self.fn_type(body),
                            }
                        }
                    };
                    self.declare(name, *span, ty);
                }
                if self.collect_mode
                    && let Some(returns) = self.finished_returns.pop()
                {
                    if let Path::Base((name, _)) = path
                        && body.2.is_none()
                        && !returns.is_empty()
                    {
                        self.collected_returns
                            .insert(name.clone().into_boxed_str(), returns);
                    }
                }
            }
            StmtKind::Expr(expr) => {
                let _ = self.infer_expr(expr);
            }
            StmtKind::TypeAlias((_, span), ty) => {
                let resolved = self.resolve_type(ty, None);
                if !self.collect_mode {
                    self.backfills
                        .push((*span, resolved.to_string().into_boxed_str()));
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
                    ));
                }
            }
            StmtKind::Call(callee, args) => {
                if let Expr(ExprKind::Access(path), span) = &**callee {
                    let _ = self.infer_call(path, *span, args);
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        let _ = self.infer_expr(expr);
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
            for TypeParam((name, span), _) in block.1.iter() {
                self.declare(name, *span, Type::Param(name.clone().into_boxed_str()));
            }
            let ret = match &block.2 {
                Some(t) => Some(self.resolve_type(t, None)),
                None => None,
            };
            self.ret_stack.push(ret);
            self.declare_params(block);
            if self.collect_mode {
                self.ret_collect.push(vec![]);
            }
        } else {
            if self.collect_mode {
                let collected = self.ret_collect.pop().unwrap_or_default();
                self.finished_returns.push(collected.into_boxed_slice());
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
    fn infer_expr(&mut self, Expr(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_const(&mut self, Expr(kind, _): &Expr) -> (Type, Option<ConstValue>) {
        let ty = self.infer_expr_kind(kind);
        let cv = match kind {
            ExprKind::Literal(cv) => Some(cv.clone()),
            _ => None,
        };
        (ty, cv)
    }

    fn infer_expr_kind(&mut self, kind: &ExprKind) -> Type {
        match kind {
            ExprKind::Literal(lit) => lit.type_of(),
            ExprKind::Table(fields) => {
                let mut vec = vec![];
                for f in fields {
                    if let Field::NameValue(n, v) = f {
                        let e = &self.infer_expr_kind(&v.0);
                        vec.push((n.0.clone().into_boxed_str(), Box::new(e.clone())))
                    } else {
                        return Type::Table(None, None);
                    }
                }
                Type::TypeTable(vec.into_boxed_slice())
            }
            ExprKind::Array(_) => Type::Array(None),
            ExprKind::Function(body) => {
                let Type::Function(Some(ft)) = self.fn_type(body) else {
                    return Type::Any;
                };
                Type::Function(Some(FunctionType {
                    params: ft.params.iter().map(|t| t.clone()).collect(),
                    returns: ft.returns.iter().map(|t| t.clone()).collect(),
                    var_arg: ft.var_arg,
                    return_var_arg: ft.return_var_arg,
                }))
            }
            ExprKind::Access(path) => self.infer_access(path),
            ExprKind::Call(callee, args) => match &**callee {
                Expr(ExprKind::Access(path), span) => self.infer_call(path, *span, args),
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

    fn object_id_of(&self, name: &str) -> Option<ObjectId> {
        self.viewer
            .lookup(name)
            .and_then(|sym| match &sym.symbol_type {
                SymbolType::ObjectClass(id) => Some(*id),
                _ => None,
            })
    }

    fn receiver_object(&self, path: &Path) -> Option<ObjectId> {
        match path {
            Path::Base((name, _)) => {
                if let Some(ty) = self.lookup_type(name)
                    && let Some(id) = self.object_member_of(&ty)
                {
                    return Some(id);
                }
                self.object_id_of(name)
            }
            Path::Chain(p, _) => self.receiver_object(p),
            _ => None,
        }
    }

    fn object_member_of(&self, ty: &Type) -> Option<ObjectId> {
        match ty {
            Type::Object { id, .. } => Some(*id),
            Type::Union(ts) => ts.iter().find_map(|t| self.object_member_of(t)),
            _ => None,
        }
    }

    fn object_of(&self, id: ObjectId) -> Type {
        let obj = &self.objects[id];
        Type::Object {
            id,
            name: obj.name.clone(),
            base: obj.base,
            args: [].into(),
        }
    }

    fn return_type_of(&self, sig: &FunctionType) -> Type {
        match sig.returns.len() {
            0 => Type::Any,
            1 => sig.returns[0].clone(),
            _ => sig
                .returns
                .iter()
                .cloned()
                .reduce(BitOr::bitor)
                .unwrap_or(Type::Any),
        }
    }

    fn find_method(&self, id: ObjectId, name: &str) -> Option<ObjectMethod> {
        self.objects[id]
            .methods
            .iter()
            .find(|m| m.name.as_ref() == name)
            .cloned()
    }

    fn infer_access(&mut self, path: &Path) -> Type {
        match path {
            Path::Chain(receiver, PathSuffix::Dot((name, _))) => {
                match self.receiver_object(receiver) {
                    Some(id) => {
                        let obj = &self.objects[id];
                        obj.members
                            .iter()
                            .find(|m| m.name.as_ref() == name.as_str())
                            .map(|m| self.resolve_type(&m.ty, None))
                            .unwrap_or(Type::Any)
                    }
                    None => match self.require_module_name(receiver) {
                        Some(module_name) => {
                            let Some(module) = self.resolve_module_type(&module_name) else {
                                return Type::Any;
                            };
                            let _ = module;
                            Type::Any
                        }
                        None => Type::Any,
                    },
                }
            }
            _ => match path {
                Path::Base((name, _)) => self.lookup_type(name.as_str()).unwrap_or(Type::Any),
                _ => Type::Any,
            },
        }
    }

    fn require_module_name(&self, path: &Path) -> Option<String> {
        let Path::Expr(expr) = path else {
            return None;
        };
        if let ExprKind::Call(f, args) = &expr.0
            && let ExprKind::Access(p) = &f.0
            && let Path::Base((name, _)) = p.as_ref()
            && name.as_str() == "require"
            && let Some(Expr(ExprKind::Literal(ConstValue::String(b)), _)) = args.first()
        {
            Some(String::from_utf8_lossy(b).into_owned())
        } else {
            None
        }
    }

    fn infer_call(&mut self, path: &Path, call_span: Span, args: &[Expr]) -> Type {
        match path {
            Path::Base((name, _)) => {
                if name.as_str() == "require" {
                    return match args.first() {
                        Some(Expr(ExprKind::Literal(ConstValue::String(b)), _)) => {
                            let module_name = String::from_utf8_lossy(b).into_owned();
                            let Some(module) = self.resolve_module_type(&module_name) else {
                                return Type::Any;
                            };
                            let _ = module;
                            Type::Table(None, None)
                        }
                        _ => Type::Any,
                    };
                }
                let Some(sig) = self.lookup_type(name) else {
                    return Type::Any;
                };
                let Type::Function(Some(ft)) = sig else {
                    return Type::Any;
                };
                let Some(decl) = self.generic_fns.get(name.as_str()).cloned() else {
                    return ft.returns.first().cloned().unwrap_or(Type::Any);
                };
                let arg_types: Vec<Type> = args.iter().map(|a| self.infer_expr(a)).collect();
                let mut subst: HashMap<Box<str>, Type> = HashMap::new();
                for (i, param_ty) in ft.params.iter().enumerate() {
                    if let Some(at) = arg_types.get(i) {
                        collect_params(param_ty, &mut subst, at);
                    }
                }
                for (TypeParam((pname, _), bound), _) in decl.iter().zip(0..) {
                    if let Some(bound) = bound
                        && let Some(arg) = subst.get(pname.as_str())
                    {
                        let bound = self.resolve_type(bound, None);
                        if !bound.accepts(arg) {
                            self.err(
                                DukaSemanticError::TypeMismatchEqual(
                                    bound.to_string(),
                                    arg.to_string(),
                                ),
                                call_span,
                            );
                        }
                    }
                }
                ft.returns
                    .first()
                    .map(|t| substitute_params(t, &subst))
                    .unwrap_or(Type::Any)
            }
            Path::Chain(receiver, PathSuffix::TypeArgs(ty_args, ty_span)) => {
                let Path::Base((fname, _)) = receiver.as_ref() else {
                    return Type::Any;
                };
                let Some(sig) = self.lookup_type(fname) else {
                    return Type::Any;
                };
                let Type::Function(Some(ft)) = sig else {
                    return Type::Any;
                };
                let Some(decl) = self.generic_fns.get(fname.as_str()).cloned() else {
                    return Type::Any;
                };
                if decl.len() != ty_args.len() {
                    return Type::Any;
                }
                let mut subst: HashMap<Box<str>, Type> = HashMap::new();
                for (TypeParam((pname, _), _), arg) in decl.iter().zip(ty_args.iter()) {
                    let arg = self.resolve_type(arg, Some(*ty_span));
                    subst.insert(pname.clone().into_boxed_str(), arg);
                }
                for (TypeParam((pname, _), bound), _) in decl.iter().zip(0..) {
                    if let Some(bound) = bound
                        && let Some(arg) = subst.get(pname.as_str())
                    {
                        let bound = self.resolve_type(bound, None);
                        if !bound.accepts(arg) {
                            self.err(
                                DukaSemanticError::TypeMismatchEqual(
                                    bound.to_string(),
                                    arg.to_string(),
                                ),
                                call_span,
                            );
                        }
                    }
                }
                ft.returns
                    .first()
                    .map(|t| substitute_params(t, &subst))
                    .unwrap_or(Type::Any)
            }
            Path::Chain(receiver, PathSuffix::Colon((mname, mspan))) => {
                let (mname, mspan) = (String::from(&*mname), *mspan);
                let Some(id) = self.receiver_object(receiver) else {
                    return Type::Any;
                };
                if let Some(m) = self.find_method(id, &mname) {
                    self.links.push(MethodLink {
                        call_span,
                        name_span: mspan,
                        decl_span: m.span,
                        owner: id,
                    });
                    return self.return_type_of(&m.sig);
                }
                Type::Any
            }
            Path::Chain(receiver, PathSuffix::Dot((name, name_span))) => {
                let (name, name_span) = (String::from(name), *name_span);
                let Some(id) = self.receiver_object(receiver) else {
                    return Type::Any;
                };
                if name == csugar::NEW_FUNC {
                    return self.object_of(id);
                }
                if let Some(m) = self.find_method(id, &name) {
                    self.links.push(MethodLink {
                        call_span,
                        name_span,
                        decl_span: m.span,
                        owner: id,
                    });
                    return self.return_type_of(&m.sig);
                }
                Type::Any
            }
            _ => Type::Any,
        }
    }
}

fn collect_params(ty: &Type, subst: &mut HashMap<Box<str>, Type>, actual: &Type) {
    match ty {
        Type::Param(name) => {
            if !subst.contains_key(name) {
                subst.insert(name.clone(), actual.clone());
            }
        }
        Type::Array(Some(inner)) => collect_params(inner, subst, actual),
        Type::Table(k, v) => {
            if let Some(k) = k.as_deref() {
                collect_params(k, subst, actual);
            }
            if let Some(v) = v.as_deref() {
                collect_params(v, subst, actual);
            }
        }
        Type::Union(ts) => {
            for t in ts.iter() {
                collect_params(t, subst, actual);
            }
        }
        Type::TypeTable(t) => {
            for a in t {
                collect_params(&a.1, subst, actual);
            }
        }
        Type::TypeTuple(t) => {
            for a in t {
                collect_params(a, subst, actual);
            }
        }
        Type::Object { args, .. } => {
            for a in args.iter() {
                collect_params(a, subst, actual);
            }
        }
        Type::Function(Some(ft)) => {
            for t in ft.params.iter().chain(ft.returns.iter()) {
                collect_params(t, subst, actual);
            }
        }
        _ => {}
    }
}

fn substitute_params(ty: &Type, subst: &HashMap<Box<str>, Type>) -> Type {
    match ty {
        Type::TypeTable(t) => Type::TypeTable(
            t.iter()
                .map(|(k, v)| (k.clone(), Box::new(substitute_params(v, subst))))
                .collect(),
        ),
        Type::TypeTuple(v) => v
            .first()
            .map(|t| substitute_params(t, subst))
            .unwrap_or_default(),
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
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser};

    use crate::{
        analyzer::ScopeAnalyzer, analyzer::TypeChecker, lexer::LexerWithMacro, parser::Parser,
    };

    fn check(source: &str) -> Vec<DukaSpannedError> {
        let lexer =
            LexerWithMacro::new(Cursor::new(source), Some("test".into()), Default::default());
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
                )
            )
        })
    }

    fn parse_err(source: &str) -> bool {
        let lexer =
            LexerWithMacro::new(Cursor::new(source), Some("test".into()), Default::default());
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
        let lexer =
            LexerWithMacro::new(Cursor::new(source), Some("test".into()), Default::default());
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
    fn object_unknown_base() {
        let errors = check("object B : A\nend");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownBase(..))
            )),
            "{:?}",
            errors
        );
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
        let lexer =
            LexerWithMacro::new(Cursor::new(source), Some("test".into()), Default::default());
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let errors: Vec<_> = ScopeAnalyzer
            .chain(TypeChecker)
            .analyze(&chunk, Default::default())
            .1
            .collect();
        let data = {
            let (d, _) = ScopeAnalyzer
                .chain(TypeChecker)
                .analyze(&chunk, Default::default());
            d
        };
        (errors, data.1)
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
