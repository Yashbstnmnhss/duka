use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::ops::{Div, Mul, Sub};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, ops::Add};

use duka_shared::constants::ctype;
use duka_shared::types::UnOp;
use duka_shared::{
    dtype::{FunctionType, Type},
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo},
    utils::{SymbolTableViewer, SymbolType},
    value::{ConstValue, DukaFloat, DukaInt},
};

use crate::analyzer::CallResults;
use crate::analyzer::builtin::TYPE_BUILTINS;
use crate::analyzer::modules::{
    DukaSourceProvider, ModuleMap, ModuleType, resolve_module_type, sanitize_foreign,
};
use crate::analyzer::tyval::{TypeClosure, TypeValue};
use crate::parser::ast::{Field, PatternArrayTerm, PatternOp};
use crate::{
    analyzer::{AnalyzerData, InlineTypeFn, ObjectType, TypeFn, Visit, Visitor},
    parser::ast::{
        DukaChunk, Expr, ExprKind, FuncBody, If, Match, Param, Path, PathSuffix, PatternTerm, Stmt,
        StmtKind, TypeDescriptor,
    },
};

/// type-context的解释器
/// See docs/type.md
pub struct TypeEval;
impl DukaAnalyzer for TypeEval {
    type InputType = DukaChunk;
    type InputData = AnalyzerData;
    type OutputData = AnalyzerData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        let (config, mut analysis) = data;
        let mut ctx = EvalCtx::new(EvalCtxInit {
            source: Arc::new(chunk.source_info.clone()),
            viewer: SymbolTableViewer::new(&analysis.symbols),
            type_fns: &analysis.type_fns,
            inline_type_fns: &analysis.inline_type_fns,
            objects: &analysis.objects,
            aliases: &analysis.aliases,
            results: analysis.call_cache.clone(),
            modules: Some(&analysis.modules),
            provider: None,
            report_errors: false,
        });
        chunk.visit(&mut ctx);
        let errors = std::mem::take(&mut ctx.errors);
        analysis.type_results = ctx.results.lock().unwrap().clone();
        ((config, analysis), errors.into_iter())
    }
}

impl TypeEval {
    pub fn analyze_with_provider<'a>(
        &self,
        chunk: &DukaChunk,
        data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> (AnalyzerData, impl Iterator<Item = DukaSpannedError>) {
        let (config, mut analysis) = data;
        let mut ctx = EvalCtx::new(EvalCtxInit {
            source: Arc::new(chunk.source_info.clone()),
            viewer: SymbolTableViewer::new(&analysis.symbols),
            type_fns: &analysis.type_fns,
            inline_type_fns: &analysis.inline_type_fns,
            objects: &analysis.objects,
            aliases: &analysis.aliases,
            results: analysis.call_cache.clone(),
            modules: Some(&analysis.modules),
            provider,
            report_errors: false,
        });
        chunk.visit(&mut ctx);
        let errors = std::mem::take(&mut ctx.errors);
        analysis.type_results = ctx.results.lock().unwrap().clone();
        ((config, analysis), errors.into_iter())
    }
}

const MAX_DEPTH: usize = 32;
const MAX_ITERS: usize = 1000;
const MAX_FUEL: usize = 10000;

pub(crate) struct EvalCtxInit<'a> {
    pub source: Arc<SourceInfo>,
    pub viewer: SymbolTableViewer<'a>,
    pub type_fns: &'a [TypeFn],
    pub inline_type_fns: &'a [InlineTypeFn],
    pub objects: &'a [ObjectType],
    pub aliases: &'a [(Box<str>, TypeDescriptor)],
    pub results: Arc<Mutex<CallResults>>,
    pub modules: Option<&'a ModuleMap>,
    pub provider: Option<&'a dyn DukaSourceProvider>,
    pub report_errors: bool,
}

/// 以递归计算运行时type
pub(crate) struct EvalCtx<'a> {
    source: Arc<SourceInfo>,
    viewer: SymbolTableViewer<'a>,
    type_fns: &'a [TypeFn],
    inline_type_fns: &'a [InlineTypeFn],
    objects: &'a [ObjectType],
    aliases: &'a [(Box<str>, TypeDescriptor)],
    frames: Vec<HashMap<Box<str>, (TypeValue, bool)>>,
    results: Arc<Mutex<CallResults>>,
    modules: Option<&'a ModuleMap>,
    provider: Option<&'a dyn DukaSourceProvider>,
    report_errors: bool,
    alias_depth: usize,
    module_stack: Vec<Box<str>>,
    depth: usize,
    cache_fp: HashMap<u64, usize>,
    rec_stack: HashMap<u64, Box<str>>,
    hook: Option<&'a mut dyn FnMut(&TypeDescriptor) -> Option<TypeValue>>,
    pub(crate) errors: Vec<DukaSpannedError>,
    call_span_stack: Vec<Span>,
    fuel: usize,
    evaluating_inline: HashSet<usize>,
    recursive_inline: Option<usize>,
}

enum Return<T> {
    Value(T),
    None,
    Break,
    Continue,
    Tail(Box<str>, Box<[T]>, Span),
}

// Eval type-context AST
impl<'a> EvalCtx<'a> {
    pub(crate) fn new(init: EvalCtxInit<'a>) -> Self {
        Self {
            source: init.source,
            viewer: init.viewer,
            type_fns: init.type_fns,
            inline_type_fns: init.inline_type_fns,
            objects: init.objects,
            aliases: init.aliases,
            frames: vec![HashMap::new()],
            results: init.results,
            modules: init.modules,
            provider: init.provider,
            report_errors: init.report_errors,
            alias_depth: 0,
            module_stack: vec![],
            depth: 0,
            cache_fp: HashMap::new(),
            rec_stack: HashMap::new(),
            hook: None,
            errors: vec![],
            call_span_stack: vec![],
            fuel: MAX_FUEL,
            evaluating_inline: HashSet::new(),
            recursive_inline: None,
        }
    }

    pub(crate) fn with_hook(
        mut self,
        hook: Option<&'a mut dyn FnMut(&TypeDescriptor) -> Option<TypeValue>>,
    ) -> Self {
        self.hook = hook;
        self
    }

    fn fingerprint(name: &str, args: &[TypeValue], body: &FuncBody) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut h);
        for a in args {
            match a {
                TypeValue::Type(t) => {
                    format!("{t}").hash(&mut h);
                }
                TypeValue::Tagged { ty: _, id } => {
                    id.hash(&mut h);
                }
                TypeValue::Closure(c) => {
                    c.name.hash(&mut h);
                }
            }
        }
        // hash body structure to invalidate cache when body changes
        format!("{body:?}").hash(&mut h);
        h.finish()
    }

    fn err(&mut self, name: &str, msg: impl Into<Box<str>>, span: Span) {
        let msg: Box<str> = msg.into();
        let report_span = self.call_span_stack.first().copied().unwrap_or(span);
        let related: Box<[(Box<str>, Span)]> = if report_span != span {
            [(msg.clone(), span)].into()
        } else {
            [].into()
        };
        self.errors.push(DukaSpannedError {
            kind: DukaSemanticError::TypeFnError(name.into(), msg).into(),
            span: report_span,
            related,
            source_info: self.source.clone(),
        });
    }

    fn lookup_frame(&self, key: &str) -> Option<TypeValue> {
        for frame in self.frames.iter().rev() {
            if let Some((t, _)) = frame.get(key) {
                return Some(t.clone());
            }
        }
        None
    }

    fn find_frame(&self, key: &str) -> Option<usize> {
        for (i, frame) in self.frames.iter().enumerate().rev() {
            if frame.contains_key(key) {
                return Some(i);
            }
        }
        None
    }

    fn resolve_module_base_tv(&self, base: &TypeDescriptor) -> Option<&'a ModuleType> {
        match base {
            TypeDescriptor::TypeCall { name, args, .. } if name.as_ref() == ctype::REQUIRE => {
                let m = args.first().and_then(|a| match a {
                    TypeDescriptor::Pure(Type::Literal(ConstValue::String(b))) => {
                        Some(String::from_utf8_lossy(b).into_owned())
                    }
                    _ => None,
                })?;
                let modules = self.modules?;
                let caller = self.source.name.as_deref();
                let provider = self.provider?;
                resolve_module_type(modules, &m, caller, provider)
            }
            TypeDescriptor::Named(name, _) => {
                if let Some(sym) = self.viewer.lookup(name) {
                    if let SymbolType::TypeAlias(id) = sym.symbol_type.clone() {
                        if let Some((_, tv)) = self.aliases.get(id) {
                            return self.resolve_module_base_tv(tv);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn module_ctx(&self, module: &'a ModuleType) -> EvalCtxInit<'a> {
        EvalCtxInit {
            source: module.source.clone(),
            viewer: SymbolTableViewer::new(&module.analysis.symbols),
            type_fns: &module.analysis.type_fns,
            inline_type_fns: &module.analysis.inline_type_fns,
            objects: &module.analysis.objects,
            aliases: &module.analysis.aliases,
            results: self.results.clone(),
            modules: self.modules,
            provider: self.provider,
            report_errors: self.report_errors,
        }
    }

    fn resolve_exported_val(
        &mut self,
        module: &'a ModuleType,
        member: &str,
        args: Option<&[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        if self.module_stack.contains(&module.key) {
            if self.report_errors {
                self.errors.push(DukaSpannedError {
                    kind: DukaSemanticError::CircularRequire(module.key.clone()).into(),
                    span,
                    related: [].into(),
                    source_info: self.source.clone(),
                });
            }
            return TypeValue::Type(Type::Any);
        }
        self.module_stack.push(module.key.clone());
        let r = self.resolve_exported_val_inner(module, member, args, span);
        self.module_stack.pop();
        r
    }

    fn resolve_exported_val_inner(
        &mut self,
        module: &'a ModuleType,
        member: &str,
        args: Option<&[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let Some(kind) = module.exported.get(member) else {
            return TypeValue::Type(Type::Any);
        };
        match kind {
            crate::analyzer::modules::ExportedTypeKind::Object(_) => TypeValue::Type(Type::Any),
            crate::analyzer::modules::ExportedTypeKind::Alias(id) => {
                let Some((_, tv)) = module.analysis.aliases.get(*id) else {
                    return TypeValue::Type(Type::Any);
                };
                let mut ev = EvalCtx::new(self.module_ctx(module));
                ev.module_stack = self.module_stack.clone();
                let res = ev.eval_type(tv);
                self.errors.extend(ev.errors);
                TypeValue::Type(sanitize_foreign(res.to_type()))
            }
            crate::analyzer::modules::ExportedTypeKind::TypeFn(id) => {
                let Some(fn_def) = module.analysis.type_fns.get(*id) else {
                    return TypeValue::Type(Type::Any);
                };
                let Some(args) = args else {
                    return TypeValue::Type(Type::Any);
                };
                let mut ev = EvalCtx::new(self.module_ctx(module));
                ev.module_stack = self.module_stack.clone();
                let res = ev.call_type_fn(&fn_def.name, Box::from(args), span);
                self.errors.extend(ev.errors);
                TypeValue::Type(sanitize_foreign(res.to_type()))
            }
            crate::analyzer::modules::ExportedTypeKind::InlineFn(id) => {
                let Some(inline_fn) = module.analysis.inline_type_fns.get(*id) else {
                    return TypeValue::Type(Type::Any);
                };
                let Some(args) = args else {
                    return TypeValue::Type(Type::Any);
                };
                let mut ev = EvalCtx::new(self.module_ctx(module));
                ev.module_stack = self.module_stack.clone();
                let res = ev.call_type_fn(&inline_fn.name, Box::from(args), span);
                self.errors.extend(ev.errors);
                TypeValue::Type(sanitize_foreign(res.to_type()))
            }
        }
    }

    pub(crate) fn eval_type_assign(&mut self, fn_name: &str, path: &Path, expr: &Expr, span: Span) {
        match path {
            Path::Base((key, _)) => {
                let Some(idx) = self.find_frame(key) else {
                    self.err(fn_name, format!("unknown type local '{key}'"), span);
                    return;
                };
                if !self.frames[idx]
                    .get(key.as_str())
                    .map(|(_, m)| *m)
                    .unwrap_or(false)
                {
                    self.err(
                        fn_name,
                        format!("cannot assign to immutable type local '{key}'"),
                        span,
                    );
                    return;
                }
                let v = self.eval_expr_to_type(fn_name, expr, expr.1);
                self.frames[idx].insert(key.clone().into_boxed_str(), (v, true));
            }
            Path::Expr(_) => {
                self.err(
                    fn_name,
                    "unsupported assignment target in type function",
                    span,
                );
                return;
            }
            Path::Chain(base, suffix) => {
                let root_key = {
                    fn collect(
                        base: &Path,
                        first: &PathSuffix,
                    ) -> Option<(Box<str>, Vec<PathSuffix>)> {
                        match base {
                            Path::Base((k, _)) => Some((k.clone().into(), vec![first.clone()])),
                            Path::Chain(inner, s) => {
                                let (k, mut v) = collect(inner, s)?;
                                v.push(first.clone());
                                Some((k, v))
                            }
                            Path::Expr(_) => None,
                        }
                    }
                    collect(base, suffix)
                };
                let Some((root_key, suffixes)) = root_key else {
                    self.err(
                        fn_name,
                        "unsupported assignment target in type function",
                        span,
                    );
                    return;
                };

                let Some(idx) = self.find_frame(&root_key) else {
                    self.err(fn_name, format!("unknown type local '{root_key}'"), span);
                    return;
                };
                let Some((root_tv, _)) = self.frames[idx].get(&root_key).cloned() else {
                    self.err(fn_name, format!("unknown type local '{root_key}'"), span);
                    return;
                };
                let mut cur = root_tv.to_type();

                for s in &suffixes[..suffixes.len().saturating_sub(1)] {
                    match s {
                        PathSuffix::Dot((name, _)) | PathSuffix::Colon((name, _)) => {
                            let Type::TypeTable(fields) = &cur else {
                                self.err(fn_name, "intermediate layer is not a table", span);
                                return;
                            };
                            let Some((_, f)) = fields.iter().find(|(k, _)| matches!(k, ConstValue::String(s) if s.as_ref() == name.as_bytes()))
                            else {
                                self.err(fn_name, format!("middle layer not found: {name}"), span);
                                return;
                            };
                            cur = *f.clone();
                        }
                        PathSuffix::Index(idx_expr) => {
                            let idx_tv = self.eval_expr_to_type(fn_name, idx_expr, span);
                            match &cur {
                                Type::TypeTable(items) => {
                                    if items.is_empty() {
                                        self.err(
                                            fn_name,
                                            "unknown key in intermediate layer",
                                            span,
                                        );
                                        return;
                                    }
                                    let s = match idx_tv {
                                        TypeValue::Type(Type::Literal(ConstValue::String(s))) => {
                                            ConstValue::String(s)
                                        }
                                        b => ConstValue::String(
                                            b.to_type().to_string().into_bytes().into_boxed_slice(),
                                        ),
                                    };
                                    let Some(idx) = items.iter().position(|p| p.0 == s) else {
                                        self.err(
                                            fn_name,
                                            "unknown key in intermediate layer",
                                            span,
                                        );
                                        return;
                                    };
                                    cur = *items[idx].1.clone();
                                }
                                Type::TypeTuple(items) => {
                                    if let TypeValue::Type(Type::Literal(ConstValue::Int(i))) =
                                        idx_tv
                                    {
                                        let i = i as usize;
                                        if i >= items.len() {
                                            self.err(
                                                fn_name,
                                                "index out of bounds in intermediate layer",
                                                span,
                                            );
                                            return;
                                        }
                                        cur = items[i].clone();
                                    } else {
                                        self.err(fn_name, "index must be integer literal", span);
                                        return;
                                    }
                                }
                                _ => {
                                    self.err(
                                        fn_name,
                                        "intermediate layer is not a type array/table",
                                        span,
                                    );
                                    return;
                                }
                            }
                        }
                        PathSuffix::TypeArgs(..) => {
                            self.err(fn_name, "TypeArgs not allowed in assignment path", span);
                            return;
                        }
                    }
                }

                let terminal = &suffixes[suffixes.len() - 1];
                let new_val = self.eval_expr_to_type(fn_name, expr, expr.1).to_type();

                match terminal {
                    PathSuffix::Dot((name, _)) | PathSuffix::Colon((name, _)) => {
                        let Type::TypeTable(fields) = cur else {
                            self.err(fn_name, "assignment target is not a table", span);
                            return;
                        };
                        let key = ConstValue::String(name.as_bytes().to_vec().into_boxed_slice());
                        let mut fields_vec = fields;
                        if let Some((_, f)) = fields_vec.iter_mut().find(|(k, _)| *k == key) {
                            *f = Box::new(new_val);
                        } else {
                            fields_vec.push((key, Box::new(new_val)));
                        }
                        cur = Type::TypeTable(fields_vec);
                    }
                    PathSuffix::Index(idx_expr) => {
                        match cur {
                            Type::TypeTable(items) => {
                                let mut items_vec = items;
                                let idx_tv = self.eval_expr_to_type(fn_name, idx_expr, span);
                                let s = match idx_tv {
                                    TypeValue::Type(Type::Literal(ConstValue::String(s))) => {
                                        ConstValue::String(s)
                                    }
                                    b => ConstValue::String(
                                        b.to_type().to_string().into_bytes().into_boxed_slice(),
                                    ),
                                };
                                match items_vec.iter().position(|p| p.0 == s) {
                                    Some(idx) => items_vec[idx].1 = Box::new(new_val),
                                    None => items_vec.push((s, Box::new(new_val))),
                                };
                                cur = Type::TypeTable(items_vec);
                            }
                            Type::TypeTuple(items) => {
                                let mut items_vec: Vec<Type> = items;
                                let idx_tv = self.eval_expr_to_type(fn_name, idx_expr, span);
                                if let TypeValue::Type(Type::Literal(ConstValue::Int(i))) = idx_tv {
                                    let i = i as usize;
                                    if i > items_vec.len() {
                                        self.err(fn_name, "index out of bounds", span);
                                        return;
                                    } else if i == items_vec.len() {
                                        items_vec.push(new_val);
                                    } else {
                                        items_vec[i] = new_val;
                                    }
                                    cur = Type::TypeTuple(items_vec);
                                } else {
                                    self.err(fn_name, "index must be integer literal", span);
                                    return;
                                }
                            }
                            _ => {
                                self.err(fn_name, "assignment target is not a tuple/array", span);
                                return;
                            }
                        };
                    }
                    PathSuffix::TypeArgs(..) => {
                        self.err(fn_name, "TypeArgs not allowed in assignment terminal", span);
                        return;
                    }
                }

                self.frames[idx].insert(root_key, (TypeValue::Type(cur), true));
            }
        };
    }

    pub(crate) fn eval_type_access(
        &mut self,
        fn_name: &str,
        base: TypeValue,
        member: TypeValue,
        span: Span,
    ) -> Option<TypeValue> {
        let b = base.to_type();
        let m = member.to_type();

        let found = Self::type_access_inner(self, &b, &m);
        if found.is_none() {
            self.err(fn_name, "unsupported access expression", span);
        }
        found
    }

    fn type_access_inner(ctx: &mut EvalCtx, b: &Type, m: &Type) -> Option<TypeValue> {
        if let Type::Rec(inner) = b {
            return Self::type_access_inner(ctx, inner, m);
        }
        if let Type::Union(us) = b {
            return us.iter().find_map(|u| Self::type_access_inner(ctx, u, m));
        }
        match (b, m) {
            (Type::TypeTable(items), Type::Literal(key)) => items
                .iter()
                .find(|(k, _)| *k == *key)
                .map(|(_, v)| TypeValue::Type(*v.clone())),
            (Type::TypeTuple(items), Type::Literal(ConstValue::Int(idx))) => {
                items.get(*idx as usize).cloned().map(TypeValue::Type)
            }
            (Type::Object { id, .. }, Type::Literal(ConstValue::String(key))) => {
                let objs = ctx.objects;
                objs[*id]
                    .members
                    .iter()
                    .map(|v| (v.name.clone(), ctx.eval_type(&v.ty)))
                    .chain(
                        objs[*id]
                            .methods
                            .iter()
                            .cloned()
                            .map(|v| (v.name, TypeValue::Type(Type::Function(Some(v.sig))))),
                    )
                    .find_map(|i| (i.0.as_bytes() == &**key).then_some(i.1))
            }
            _ => None,
        }
    }

    pub(crate) fn eval_type(&mut self, ty: &TypeDescriptor) -> TypeValue {
        if let TypeDescriptor::Named(name, _) = ty
            && let Some(t) = self.lookup_frame(name)
        {
            return t;
        }
        if let Some(hook) = &mut self.hook
            && let Some(v) = hook(ty)
        {
            return v;
        }
        match ty {
            TypeDescriptor::TypeOf { .. } => TypeValue::Type(Type::Any),
            TypeDescriptor::FnLit(body) => {
                for p in body.0.iter() {
                    if let Param::Typed(.., t) = p {
                        let _ = self.eval_type(t);
                    }
                }
                if let Some(rt) = &body.2 {
                    for t in rt.tys.iter() {
                        let _ = self.eval_type(t);
                    }
                }
                TypeValue::Closure(Box::new(TypeClosure {
                    name: "__anon".into(),
                    params: body.0.clone(),
                    body: body.clone(),
                    captured: self.frames.clone(),
                }))
            }
            TypeDescriptor::NonNil(inner) => {
                let t = self.eval_type(inner).to_type();
                TypeValue::Type(t.nonnilable())
            }
            TypeDescriptor::Nilable(inner) => {
                let t = self.eval_type(inner).to_type();
                TypeValue::Type(t.nilable())
            }
            TypeDescriptor::Rec(inner) => {
                let t = self.eval_type(inner).to_type();
                TypeValue::Type(Type::Rec(Box::new(t)))
            }
            TypeDescriptor::TypeCall { name, args, span } => {
                let args: Box<[TypeValue]> = args.iter().map(|a| self.eval_type(a)).collect();
                if name.as_ref() == ctype::REQUIRE {
                    TypeValue::Type(Type::Any)
                } else {
                    self.call_type_fn(name, args, *span)
                }
            }
            TypeDescriptor::Access {
                base,
                member,
                args,
                span,
            } => {
                if let Some(module) = self.resolve_module_base_tv(base) {
                    let TypeDescriptor::Pure(Type::Literal(ConstValue::String(s))) = &**member
                    else {
                        return TypeValue::Type(Type::Any);
                    };
                    let Ok(name) = str::from_utf8(s) else {
                        return TypeValue::Type(Type::Any);
                    };
                    let argv = match args {
                        Some(a) => Some(
                            a.iter()
                                .map(|x| self.eval_type(x))
                                .collect::<Box<[TypeValue]>>(),
                        ),
                        None => None,
                    };
                    return self.resolve_exported_val(module, name, argv.as_deref(), *span);
                }
                if let TypeDescriptor::TypeCall { name, .. } = base.as_ref()
                    && name.as_ref() == ctype::REQUIRE
                {
                    return TypeValue::Type(Type::Any);
                }
                let base = self.eval_type(base);
                let member = self.eval_type(member);
                if args.as_ref().is_some() {
                    TypeValue::Type(Type::Any)
                } else {
                    self.eval_type_access("Descriptor", base, member, *span)
                        .unwrap_or_default()
                }
            }
            TypeDescriptor::Array(e) => TypeValue::Type(Type::Array(
                e.as_deref().map(|e| Box::new(self.eval_type(e).to_type())),
            )),
            TypeDescriptor::Table(k, v) => TypeValue::Type(Type::Table(
                k.as_deref().map(|k| Box::new(self.eval_type(k).to_type())),
                v.as_deref().map(|v| Box::new(self.eval_type(v).to_type())),
            )),
            TypeDescriptor::Union(ts) => {
                let mut acc = Type::Never;
                for t in ts.iter() {
                    acc = acc | self.eval_type(t).to_type();
                }
                TypeValue::Type(acc)
            }
            TypeDescriptor::Function(ft) => {
                let ft = ft.as_ref().map(|ft| FunctionType {
                    params: ft
                        .params
                        .iter()
                        .map(|t| self.eval_type(t).to_type())
                        .collect(),
                    var_arg: ft.var_arg,
                    returns: ft
                        .returns
                        .iter()
                        .map(|t| self.eval_type(t).to_type())
                        .collect(),
                    return_var_arg: ft.return_var_arg,
                });
                TypeValue::Type(Type::Function(ft))
            }
            TypeDescriptor::TypeTuple(ts) => TypeValue::Type(Type::TypeTuple(
                ts.iter().map(|t| self.eval_type(t).to_type()).collect(),
            )),
            TypeDescriptor::TypeTable(ts) => TypeValue::Type(Type::TypeTable(
                ts.iter()
                    .map(|(k, v)| {
                        (
                            ConstValue::String(k.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(self.eval_type(v).to_type()),
                        )
                    })
                    .collect(),
            )),
            TypeDescriptor::Generic { name, args, .. } => {
                let args: Box<[TypeValue]> = args.iter().map(|a| self.eval_type(a)).collect();
                if let Some(sym) = self.viewer.lookup(name) {
                    if let SymbolType::ObjectClass(id) = sym.symbol_type.clone() {
                        if let Some(o) = self.objects.get(id) {
                            return TypeValue::Type(Type::Object {
                                id,
                                name: o.name.clone(),
                                base: o.base,
                                args: args.iter().map(|a| a.to_type()).collect(),
                            });
                        }
                    }
                }
                TypeValue::Type(Type::Any)
            }
            TypeDescriptor::Named(name, _) => {
                if let Some(t) = self.lookup_frame(name) {
                    return t;
                }
                if let Some(symbol) = self.viewer.lookup(name) {
                    match symbol.symbol_type.clone() {
                        SymbolType::TypeAlias(id) => match self.aliases.get(id) {
                            Some((_, tv)) if self.alias_depth < 32 => {
                                self.alias_depth += 1;
                                let r = self.eval_type(tv);
                                self.alias_depth -= 1;
                                r
                            }
                            _ => TypeValue::Type(Type::Any),
                        },
                        SymbolType::ObjectClass(id) => {
                            if let Some(o) = self.objects.get(id) {
                                TypeValue::Type(Type::Object {
                                    id,
                                    name: o.name.clone(),
                                    base: o.base,
                                    args: Box::new([]),
                                })
                            } else {
                                TypeValue::Type(Type::Any)
                            }
                        }
                        SymbolType::Constant(cv) => TypeValue::Type(cv.type_of()),
                        SymbolType::TypeFunction(id) => {
                            if let Some(fn_def) = self.type_fns.get(id) {
                                TypeValue::Closure(Box::new(TypeClosure {
                                    name: name.clone(),
                                    params: fn_def.body.0.clone(),
                                    body: fn_def.body.clone(),
                                    captured: vec![],
                                }))
                            } else {
                                TypeValue::Type(Type::Any)
                            }
                        }
                        SymbolType::InlineTypeFunction(_) => TypeValue::Type(Type::Any),
                        _ => TypeValue::Type(Type::Any),
                    }
                } else {
                    TypeValue::Type(Type::Any)
                }
            }
            TypeDescriptor::Pure(t) => TypeValue::Type(t.clone()),
        }
    }

    pub(crate) fn call_type_fn(
        &mut self,
        name: &str,
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let Some(symbol) = self.viewer.lookup(name) else {
            return self.call_builtin_or_unknown(name, args, span);
        };
        match symbol.symbol_type.clone() {
            SymbolType::TypeFunction(id) => match self.type_fns.get(id) {
                Some(fn_def) => self.apply(name, &fn_def.body.0, &fn_def.body, &[], args, span),
                None => {
                    self.err(name, "type function body missing", span);
                    TypeValue::Type(Type::Any)
                }
            },
            SymbolType::InlineTypeFunction(id) => self.call_inline_type_fn(name, id, args, span),
            SymbolType::TypeAlias(id) => {
                let Some((_, tv)) = self.aliases.get(id) else {
                    self.err(name, "alias body missing", span);
                    return TypeValue::Type(Type::Any);
                };
                let val = self.eval_type(tv);
                match &val {
                    TypeValue::Closure(c) => {
                        if args.is_empty() && c.params.is_empty() {
                            val
                        } else {
                            self.apply_closure(c, args, span)
                        }
                    }
                    _ if args.is_empty() => val,
                    _ => {
                        self.err(name, "not a callable type function", span);
                        TypeValue::Type(Type::Any)
                    }
                }
            }
            _ => {
                self.err(name, "not a type function", span);
                TypeValue::Type(Type::Any)
            }
        }
    }

    fn call_inline_type_fn(
        &mut self,
        name: &str,
        id: usize,
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let Some(inline_fn) = self.inline_type_fns.get(id) else {
            self.err(name, "type function body missing", span);
            return TypeValue::Type(Type::Any);
        };
        if self.evaluating_inline.contains(&id) {
            self.recursive_inline = Some(id);
            return TypeValue::Type(Type::Param(name.into()));
        }
        if self.fuel == 0 {
            self.err(name, "type function fuel exhausted", span);
            return TypeValue::Type(Type::Any);
        }
        self.fuel -= 1;
        let expected_arity = inline_fn
            .params
            .iter()
            .filter(|p| !matches!(p, Param::Var(_)))
            .count();
        if args.len() != expected_arity {
            self.err(
                name,
                format!("expected {expected_arity} arguments, got {}", args.len()),
                span,
            );
            return TypeValue::Type(Type::Any);
        }
        let mut frame = HashMap::new();
        for (param, arg) in inline_fn.params.iter().zip(args.iter()) {
            let pname = match param {
                Param::Typed((n, _), _) => n,
                Param::Name((n, _)) => n,
                Param::Var(_) => continue,
            };
            frame.insert(pname.clone().into_boxed_str(), (arg.clone(), false));
        }
        let saved_frames = std::mem::take(&mut self.frames);
        self.frames.push(frame);
        self.evaluating_inline.insert(id);
        self.depth += 1;
        let result = if self.depth >= MAX_DEPTH {
            self.err(
                name,
                format!("reached max recursion depth ({MAX_DEPTH})"),
                span,
            );
            TypeValue::Type(Type::Any)
        } else {
            self.eval_type(&inline_fn.ret_ty)
        };
        self.depth -= 1;
        self.frames = saved_frames;
        self.evaluating_inline.remove(&id);
        if self.recursive_inline == Some(id) {
            self.recursive_inline = None;
            let t = result.to_type();
            let mut subst = HashMap::new();
            subst.insert(name.into(), Type::Rec(Box::new(t.clone())));
            TypeValue::Type(crate::analyzer::typechecker::substitute_params(&t, &subst))
        } else {
            result
        }
    }

    fn apply_closure(&mut self, c: &TypeClosure, args: Box<[TypeValue]>, span: Span) -> TypeValue {
        self.apply(&c.name, &c.params, &c.body, &c.captured, args, span)
    }

    fn apply(
        &mut self,
        name: &str,
        params: &[Param],
        body: &FuncBody,
        captured: &[HashMap<Box<str>, (TypeValue, bool)>],
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let fp = Self::fingerprint(name, &args, body);
        if let Some(marker) = self.rec_stack.get(&fp).cloned() {
            return TypeValue::Type(Type::Param(marker));
        }
        let marker: Box<str> = format!("__rec_{name}").into_boxed_str();
        self.rec_stack.insert(fp, marker);
        let result = self.apply_inner(name, params, body, captured, args, span);
        self.rec_stack.remove(&fp);
        let mut ty = result.to_type();
        if Self::contains_rec_param(&ty) {
            ty = Type::Rec(Box::new(ty));
            TypeValue::Type(ty)
        } else {
            result
        }
    }

    fn apply_inner(
        &mut self,
        name: &str,
        params: &[Param],
        body: &FuncBody,
        captured: &[HashMap<Box<str>, (TypeValue, bool)>],
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let fp = Self::fingerprint(name, &args, body);
        if let Some(&idx) = self.cache_fp.get(&fp) {
            let res = self.results.lock().unwrap()[idx].2.clone();
            return match res {
                TypeValue::Tagged { ty, .. } => TypeValue::Tagged { ty, id: idx },
                TypeValue::Type(ty) => TypeValue::Tagged { ty, id: idx },
                _ => TypeValue::Type(Type::Any),
            };
        }
        if self.fuel == 0 {
            self.err(name, "type function fuel exhausted", span);
            return TypeValue::Type(Type::Any);
        }
        self.fuel -= 1;
        let idx = {
            let mut cache = self.results.lock().unwrap();
            let i = cache.len();
            cache.push((name.into(), args.clone(), TypeValue::Type(Type::Any)));
            i
        };
        if self.depth >= MAX_DEPTH {
            self.err(
                name,
                format!("reached max recursion depth ({MAX_DEPTH})"),
                span,
            );
            let tagged = TypeValue::Tagged {
                ty: Type::Any,
                id: idx,
            };
            let mut cache = self.results.lock().unwrap();
            if idx < cache.len() && matches!(cache[idx].2, TypeValue::Type(Type::Any)) {
                cache[idx].2 = tagged.clone();
            }
            return tagged;
        }
        let Some(frame) = self.bind_params(name, params, &body.1, &args, span) else {
            return TypeValue::Type(Type::Any);
        };
        let saved_len = self.frames.len();
        for f in captured.iter() {
            self.frames.push(f.clone());
        }
        self.frames.push(frame);
        self.depth += 1;
        self.call_span_stack.push(span);
        let mut current_name: Box<str> = name.into();
        let mut current_def: &FuncBody = body;
        let mut iters = 0;
        let result = loop {
            if self.fuel == 0 {
                self.err(&current_name, "type function fuel exhausted", span);
                break TypeValue::Type(Type::Any);
            }
            self.fuel -= 1;
            iters += 1;
            if iters > MAX_ITERS {
                self.err(
                    &current_name,
                    format!("type function tail recursion exceeded max iterations ({MAX_ITERS})"),
                    span,
                );
                break TypeValue::Type(Type::Any);
            }
            match self.eval_block(&current_name, &current_def.3) {
                Return::Value(v) => break v,
                Return::Tail(next_name, next_args, next_span) => {
                    let Some(symbol) = self.viewer.lookup(&next_name) else {
                        break self.call_builtin_or_unknown(&next_name, next_args, next_span);
                    };
                    let SymbolType::TypeFunction(next_id) = symbol.symbol_type.clone() else {
                        self.err(&next_name, "not a type function", next_span);
                        break TypeValue::Type(Type::Any);
                    };
                    let Some(next_def) = self.type_fns.get(next_id) else {
                        self.err(&next_name, "type function body missing", next_span);
                        break TypeValue::Type(Type::Any);
                    };
                    let Some(next_frame) = self.bind_params(
                        &next_name,
                        &next_def.body.0,
                        &next_def.body.1,
                        &next_args,
                        next_span,
                    ) else {
                        break TypeValue::Type(Type::Any);
                    };
                    if let Some(top) = self.frames.last_mut() {
                        *top = next_frame;
                    }
                    current_name = next_name;
                    current_def = &next_def.body;
                }
                Return::Break | Return::Continue | Return::None => {
                    break TypeValue::Type(Type::Never);
                }
            }
        };
        while self.frames.len() > saved_len {
            self.frames.pop();
        }
        self.call_span_stack.pop();
        self.depth -= 1;
        let ty = result.to_type();
        let tagged = TypeValue::Tagged { ty, id: idx };
        let mut cache = self.results.lock().unwrap();
        if idx < cache.len() && matches!(cache[idx].2, TypeValue::Type(Type::Any)) {
            cache[idx].2 = tagged.clone();
        }
        drop(cache);
        tagged
    }

    fn contains_rec_param(ty: &Type) -> bool {
        match ty {
            Type::Param(p) => p.starts_with("__rec_"),
            Type::Union(us) => us.iter().any(Self::contains_rec_param),
            Type::TypeTuple(items) => items.iter().any(Self::contains_rec_param),
            Type::TypeTable(fields) => fields.iter().any(|(_, v)| Self::contains_rec_param(v)),
            Type::Array(Some(inner)) => Self::contains_rec_param(inner),
            Type::Table(k, v) => {
                k.as_deref().is_some_and(Self::contains_rec_param)
                    || v.as_deref().is_some_and(Self::contains_rec_param)
            }
            Type::Rec(inner) => Self::contains_rec_param(inner),
            Type::Function(Some(ft)) => {
                ft.params.iter().any(Self::contains_rec_param)
                    || ft.returns.iter().any(Self::contains_rec_param)
            }
            Type::Object { args, .. } => args.iter().any(Self::contains_rec_param),
            _ => false,
        }
    }

    fn call_builtin_or_unknown(
        &mut self,
        name: &str,
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let Ok(bi) = TYPE_BUILTINS.read() else {
            self.err(name, "failed to load builtin type functions", span);
            return TypeValue::Type(Type::Any);
        };
        let Some(f) = bi.get(&name) else {
            self.err(name, "unknown type function", span);
            return TypeValue::Type(Type::Any);
        };
        match f(args.clone()) {
            Err(msg) => {
                self.err(name, format!("builtin type function error: {msg}"), span);
                TypeValue::Type(Type::Any)
            }
            Ok(result) => {
                self.results
                    .lock()
                    .unwrap()
                    .push((name.into(), args, result.clone()));
                result
            }
        }
    }

    fn bind_params(
        &mut self,
        fn_name: &str,
        params: &[Param],
        type_params: &[crate::parser::ast::TypeParam],
        args: &[TypeValue],
        span: Span,
    ) -> Option<HashMap<Box<str>, (TypeValue, bool)>> {
        if params.len() != args.len() {
            self.err(
                fn_name,
                format!("expected {} arguments, got {}", params.len(), args.len()),
                span,
            );
            return None;
        }
        let generics: std::collections::HashSet<Box<str>> = type_params
            .iter()
            .map(|crate::parser::ast::TypeParam((n, _), _)| Box::from(n.as_str()))
            .collect();
        let mut frame = HashMap::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            if let Param::Typed(_, t) = param {
                if let TypeDescriptor::Named(gn, _) = t {
                    if generics.contains(gn.as_ref()) && !frame.contains_key(gn.as_ref()) {
                        frame.insert(gn.clone(), (arg.clone(), false));
                    }
                }
            }
            let pname = match param {
                Param::Typed((n, _), _) | Param::Name((n, _)) => n.clone().into_boxed_str(),
                Param::Var(_) => continue,
            };
            frame.insert(pname, (arg.clone(), false));
        }
        self.frames.push(frame.clone());
        for (param, arg) in params.iter().zip(args.iter()) {
            if let Param::Typed((n, _), t) = param {
                let bound = self.eval_type(t);
                if !bound.to_type().accepts(&arg.to_type()) {
                    self.frames.pop();
                    self.err(
                        fn_name,
                        format!("argument {n} has invalid type, expected {t}"),
                        span,
                    );
                    return None;
                }
            }
        }
        self.frames.pop();
        Some(frame)
    }

    fn eval_block(
        &mut self,
        fn_name: &str,
        block: &crate::parser::ast::Block,
    ) -> Return<TypeValue> {
        fn r<T>(o: Option<T>) -> Return<T> {
            match o {
                Some(x) => Return::Value(x),
                None => Return::None,
            }
        }
        for stmt in &block.0 {
            let ret = match &stmt.0 {
                StmtKind::Break => Return::Break,
                StmtKind::Continue => Return::Continue,
                StmtKind::Return(exprs, _) => {
                    if exprs.len() == 1
                        && let Some((tail_name, tail_args, tail_span)) =
                            self.tailcall_target(&exprs[0])
                    {
                        let args: Box<[TypeValue]> = tail_args
                            .iter()
                            .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                            .collect();
                        return Return::Tail(tail_name, args, tail_span);
                    }
                    r((!exprs.is_empty()).then(|| {
                        TypeValue::Type(Type::TypeTuple(
                            exprs
                                .iter()
                                .map(|e| self.eval_expr_to_type(fn_name, e, stmt.1).to_type())
                                .collect(),
                        ))
                    }))
                }
                StmtKind::If(if_stmt) => self.eval_if(fn_name, if_stmt),
                StmtKind::Match(m) => self.eval_match(fn_name, m),
                StmtKind::TypeAlias((key, span), ty) => {
                    let ty = self.eval_type(ty);
                    let Some(frame) = self.frames.last_mut() else {
                        self.err(fn_name, "no scope to declare a type alias", *span);
                        return Return::None;
                    };
                    frame.insert(key.clone().into_boxed_str(), (ty, false));
                    Return::None
                }
                StmtKind::Define(names, exprs, is_global, _) => {
                    if *is_global {
                        self.err(
                            fn_name,
                            "global declarations are not allowed in a type function",
                            stmt.1,
                        );
                        return Return::None;
                    }
                    if names.len() != 1 || exprs.len() != 1 {
                        self.err(
                            fn_name,
                            "a type local requires a single initializer",
                            stmt.1,
                        );
                        return Return::None;
                    }
                    let (name, _) = &names[0].0.0;
                    let ty = self.eval_expr_to_type(fn_name, &exprs[0], exprs[0].1);
                    let Some(frame) = self.frames.last_mut() else {
                        self.err(fn_name, "no scope to declare a type local", stmt.1);
                        return Return::None;
                    };
                    frame.insert(name.clone().into_boxed_str(), (ty, true));
                    Return::None
                }
                StmtKind::Assign(paths, exprs) => {
                    for (p, e) in paths.iter().zip(exprs.iter()) {
                        self.eval_type_assign(fn_name, p, e, p.get_span())
                    }
                    Return::None
                }
                StmtKind::While(cond, body, _) => {
                    let mut iters = 0;
                    while self.eval_cond(fn_name, cond) {
                        iters += 1;
                        if iters > MAX_ITERS {
                            self.err(
                                fn_name,
                                format!("type function loop exceeded max iterations ({MAX_ITERS})"),
                                stmt.1,
                            );
                            return Return::None;
                        }
                        match self.eval_block(fn_name, body) {
                            Return::Break => break,
                            Return::Value(v) => return Return::Value(v),
                            Return::Continue => continue,
                            Return::None => (),
                            Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                        };
                    }
                    Return::None
                }
                StmtKind::ForNumeric(path, start, limit, step, body) => {
                    let start_t = self.eval_expr_to_type(fn_name, start, start.1);
                    let limit_t = self.eval_expr_to_type(fn_name, limit, limit.1);
                    let step_t = match step {
                        Some(s) => self.eval_expr_to_type(fn_name, s, s.1),
                        None => TypeValue::Type(Type::Literal(ConstValue::Int(1))),
                    };
                    let (Some(mut i), Some(stop), Some(inc)) = (
                        literal_num(&start_t),
                        literal_num(&limit_t),
                        literal_num(&step_t),
                    ) else {
                        self.err(
                            fn_name,
                            "numeric for in type function requires numeric literal bounds",
                            stmt.1,
                        );
                        return Return::None;
                    };
                    if inc == 0.0 {
                        self.err(fn_name, "numeric for step cannot be zero", stmt.1);
                        return Return::None;
                    }
                    let Path::Base((key, _)) = path else {
                        self.err(fn_name, "unsupported for target in type function", stmt.1);
                        return Return::None;
                    };
                    let mut iters = 0;
                    while (inc > 0.0 && i < stop) || (inc < 0.0 && i > stop) {
                        iters += 1;
                        if iters > MAX_ITERS {
                            self.err(
                                fn_name,
                                format!("type function loop exceeded max iterations ({MAX_ITERS})"),
                                stmt.1,
                            );
                            return Return::None;
                        }
                        self.frames.push(HashMap::from([(
                            key.clone().into_boxed_str(),
                            (TypeValue::Type(Type::Literal(num_cv(i))), false),
                        )]));
                        let res = self.eval_block(fn_name, body);
                        self.frames.pop();
                        match res {
                            Return::Break => break,
                            Return::Value(v) => return Return::Value(v),
                            Return::Continue => continue,
                            Return::None => (),
                            Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                        };
                        i += inc;
                    }
                    Return::None
                }
                StmtKind::ForGeneric(paths, exprs, body, _) => {
                    if exprs.len() != 1 {
                        self.err(
                            fn_name,
                            "type function for-in only supports a single loop expression",
                            stmt.1,
                        );
                        return Return::None;
                    }
                    let Some(iter) = exprs.first() else {
                        return Return::None;
                    };
                    let iter_t = self.eval_expr_to_type(fn_name, iter, iter.1);
                    match iter_t {
                        TypeValue::Type(Type::TypeTuple(tuple)) => match paths.iter().as_slice() {
                            [Path::Base((val, _))] => {
                                for t in tuple {
                                    self.frames.push(HashMap::from([(
                                        val.clone().into_boxed_str(),
                                        (TypeValue::Type(t), false),
                                    )]));
                                    let res = self.eval_block(fn_name, body);
                                    self.frames.pop();
                                    match res {
                                        Return::Break => break,
                                        Return::Value(v) => return Return::Value(v),
                                        Return::Continue => continue,
                                        Return::None => (),
                                        Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                                    };
                                }
                            }
                            [Path::Base((key, _)), Path::Base((val, _))] => {
                                for (i, t) in tuple.into_iter().enumerate() {
                                    self.frames.push(HashMap::from([
                                        (
                                            key.clone().into_boxed_str(),
                                            (
                                                TypeValue::Type(Type::Literal(ConstValue::Int(
                                                    i as DukaInt,
                                                ))),
                                                false,
                                            ),
                                        ),
                                        (val.clone().into_boxed_str(), (TypeValue::Type(t), false)),
                                    ]));
                                    let res = self.eval_block(fn_name, body);
                                    self.frames.pop();
                                    match res {
                                        Return::Break => break,
                                        Return::Value(v) => return Return::Value(v),
                                        Return::Continue => continue,
                                        Return::None => (),
                                        Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                                    };
                                }
                            }
                            _ => {
                                self.err(
                                    fn_name,
                                    "unsupported for-in target in type function",
                                    stmt.1,
                                );
                                return Return::None;
                            }
                        },
                        TypeValue::Type(Type::Object { id, .. }) => {
                            let obj = &self.objects[id];
                            let properties = &obj.members;
                            let methods = &obj.methods;
                            match paths.iter().as_slice() {
                                [Path::Base((key, _)), Path::Base((val, _))] => {
                                    for (k, v) in properties
                                        .iter()
                                        .map(|i| {
                                            (
                                                &i.name,
                                                TypeValue::Type(
                                                    i.ty.clone().expect_pure().unwrap_or(Type::Any),
                                                ),
                                            )
                                        })
                                        .chain(methods.iter().map(|i| {
                                            (
                                                &i.name,
                                                TypeValue::Type(Type::Function(Some(
                                                    i.sig.clone(),
                                                ))),
                                            )
                                        }))
                                    {
                                        self.frames.push(HashMap::from([
                                            (
                                                key.clone().into_boxed_str(),
                                                (
                                                    TypeValue::Type(Type::Literal(
                                                        ConstValue::String(
                                                            k.clone().into_boxed_bytes(),
                                                        ),
                                                    )),
                                                    false,
                                                ),
                                            ),
                                            (val.clone().into_boxed_str(), (v, false)),
                                        ]));
                                        let res = self.eval_block(fn_name, body);
                                        self.frames.pop();
                                        match res {
                                            Return::Break => break,
                                            Return::Value(v) => return Return::Value(v),
                                            Return::Continue => continue,
                                            Return::None => (),
                                            Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                                        };
                                    }
                                }
                                _ => {
                                    self.err(
                                        fn_name,
                                        "unsupported for-in target in type function",
                                        stmt.1,
                                    );
                                    return Return::None;
                                }
                            }
                        }
                        TypeValue::Type(Type::TypeTable(properties)) => {
                            match paths.iter().as_slice() {
                                [Path::Base((key, _)), Path::Base((val, _))] => {
                                    for (k, v) in properties.into_iter() {
                                        self.frames.push(HashMap::from([
                                            (
                                                key.clone().into_boxed_str(),
                                                (TypeValue::Type(Type::Literal(k)), false),
                                            ),
                                            (
                                                val.clone().into_boxed_str(),
                                                (TypeValue::Type(*v), false),
                                            ),
                                        ]));
                                        let res = self.eval_block(fn_name, body);
                                        self.frames.pop();
                                        match res {
                                            Return::Break => break,
                                            Return::Value(v) => return Return::Value(v),
                                            Return::Continue => continue,
                                            Return::None => (),
                                            Return::Tail(n, a, s) => return Return::Tail(n, a, s),
                                        };
                                    }
                                }
                                _ => {
                                    self.err(
                                        fn_name,
                                        "unsupported for-in target in type function",
                                        stmt.1,
                                    );
                                    return Return::None;
                                }
                            }
                        }
                        _ => {
                            self.err(fn_name, "type cannot be iterated in type function", stmt.1);
                            return Return::None;
                        }
                    };
                    Return::None
                }
                StmtKind::Do(blk, _) => {
                    self.frames.push(HashMap::new());
                    let res = self.eval_block(fn_name, blk);
                    self.frames.pop();
                    res
                }
                StmtKind::Call(callee, call_args) => {
                    let tv_args: Box<[TypeValue]> = call_args
                        .iter()
                        .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                        .collect();
                    let callee_tv = self.eval_expr_to_type(fn_name, callee, stmt.1);
                    if let TypeValue::Closure(ref c) = callee_tv.without_tag() {
                        let _ = self.apply_closure(c, tv_args, stmt.1);
                    }
                    Return::None
                }
                StmtKind::Expr(expr) => {
                    let _ = self.eval_expr_to_type(fn_name, expr, stmt.1);
                    Return::None
                }
                _ => Return::None,
            };
            if let r @ (Return::Value(_) | Return::Tail(..)) = ret {
                return r;
            }
        }
        if let Some(stmt) = &block.1 {
            if let StmtKind::Return(exprs, _) = &stmt.0 {
                if let Some(e) = exprs.first() {
                    if let Some((tail_name, tail_args, tail_span)) = self.tailcall_target(e) {
                        let args: Box<[TypeValue]> = tail_args
                            .iter()
                            .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                            .collect();
                        return Return::Tail(tail_name, args, tail_span);
                    }
                    return Return::Value(self.eval_expr_to_type(fn_name, e, stmt.1));
                }
            }
        }
        Return::None
    }

    fn eval_if(
        &mut self,
        fn_name: &str,
        If(if_clause, else_ifs, else_clause): &If,
    ) -> Return<TypeValue> {
        if self.eval_cond(fn_name, &if_clause.1) {
            return self.eval_block(fn_name, &if_clause.0);
        }
        for clause in else_ifs.iter() {
            if self.eval_cond(fn_name, &clause.1) {
                return self.eval_block(fn_name, &clause.0);
            }
        }
        if let Some(else_b) = else_clause {
            return self.eval_block(fn_name, else_b);
        }
        Return::None
    }

    fn eval_cond(&mut self, fn_name: &str, expr: &Expr) -> bool {
        let v = self.eval_expr_to_type(fn_name, expr, expr.1);
        match v {
            TypeValue::Type(Type::Literal(ConstValue::Nil))
            | TypeValue::Type(Type::Nil)
            | TypeValue::Type(Type::Never) => false,
            TypeValue::Type(Type::Literal(ConstValue::Bool(b))) => b,
            _ => true,
        }
    }

    fn eval_match(&mut self, fn_name: &str, m: &Match) -> Return<TypeValue> {
        let target = self.eval_expr_to_type(fn_name, m.0.as_ref(), m.0.1);
        for clause in m.1.iter() {
            let mut bindings = HashMap::new();
            if self.match_pattern(fn_name, &clause.0, &target, &mut bindings, m.0.1) {
                self.frames.push(bindings);
                let res = self.eval_block(fn_name, &clause.1);
                self.frames.pop();
                return res;
            }
        }
        if let Some(else_b) = &m.2 {
            return self.eval_block(fn_name, else_b);
        }
        Return::None
    }

    fn match_pattern(
        &mut self,
        fn_name: &str,
        pattern: &(PatternTerm, Option<Box<Expr>>),
        target: &TypeValue,
        bindings: &mut HashMap<Box<str>, (TypeValue, bool)>,
        span: Span,
    ) -> bool {
        (match &pattern.0 {
            PatternTerm::Constant(expr) => self.eval_expr_to_type(fn_name, expr, expr.1) == *target,
            PatternTerm::Bind((key, _), _) => {
                bindings.insert(key.clone().into_boxed_str(), (target.clone(), false));
                true
            }
            PatternTerm::Not(a) => {
                !self.match_pattern(fn_name, &(*a.clone(), None), target, bindings, span)
            }
            // PatternTerm::Custom(keyword, params, subs) => match keyword.0.as_str() {
            //     csugar::REGEX_PAT => {
            //         let Some(pat_expr) = params.first() else {
            //             return false;
            //         };
            //         let Type::Literal(ConstValue::String(target_str)) = target.to_type() else {
            //             return false;
            //         };

            //         let Type::Literal(ConstValue::String(str)) = self
            //             .eval_expr_to_type(fn_name, pat_expr, pat_expr.1)
            //             .to_type()
            //         else {
            //             return false;
            //         };

            //         let Ok(pattern) = str::from_utf8(&str) else {
            //             return false;
            //         };
            //         let Ok(target) = str::from_utf8(&target_str) else {
            //             return false;
            //         };

            //         let compiled = match regex::compile(pattern) {
            //             Ok(k) => k,
            //             Err(e) => {
            //                 self.err(fn_name, e.to_string(), pat_expr.1);
            //                 return false;
            //             }
            //         };
            //         match regex::Runner::new(&compiled).search(target, 0) {
            //             Some(mat) => {
            //                 for (range, sub) in mat.captures.into_iter().zip(subs) {
            //                     if !self.match_pattern(
            //                         fn_name,
            //                         &(sub.clone(), None),
            //                         &TypeValue::Type(Type::Literal(ConstValue::String(
            //                             target[range.0..range.1].as_bytes().into(),
            //                         ))),
            //                         bindings,
            //                         span,
            //                     ) {
            //                         return false;
            //                     }
            //                 }
            //                 true
            //             }
            //             None => false,
            //         }
            //     }
            //     _ => false,
            // },
            PatternTerm::Compound(a, b, op) => match op {
                PatternOp::And => {
                    self.match_pattern(fn_name, &(*a.clone(), None), target, bindings, span)
                        && self.match_pattern(fn_name, &(*b.clone(), None), target, bindings, span)
                }
                PatternOp::Or => {
                    self.match_pattern(fn_name, &(*a.clone(), None), target, bindings, span)
                        || self.match_pattern(fn_name, &(*b.clone(), None), target, bindings, span)
                }
                PatternOp::Xor => {
                    self.match_pattern(fn_name, &(*a.clone(), None), target, bindings, span)
                        ^ self.match_pattern(fn_name, &(*b.clone(), None), target, bindings, span)
                }
            },
            PatternTerm::Type((name, _), args) => {
                self.match_type_ctor(fn_name, name, args, target, bindings, span)
            }
            PatternTerm::Array(items) => {
                let TypeValue::Type(Type::TypeTuple(types)) = target else {
                    return false;
                };
                let mut first_many = None;
                let mut i = 0usize;
                let mut i2 = 0usize;
                for item in items {
                    let idx = if let Some(f) = first_many {
                        types.len() - (items.len() - f - i2)
                    } else {
                        i
                    };
                    match item {
                        PatternArrayTerm::Discard(count) => {
                            if idx > types.len() {
                                return false;
                            }

                            if first_many.is_some() {
                                i2 += *count
                            } else {
                                i += *count
                            }
                        }
                        PatternArrayTerm::DiscardMany => {
                            if first_many.is_some() {
                                self.err(fn_name, "invalid ... pattern", span);
                                return false;
                            }

                            if i == items.len() - 1 {
                                return true;
                            }

                            if idx > types.len() {
                                return false;
                            }

                            first_many = Some(i);
                        }
                        PatternArrayTerm::Term(t) => {
                            if idx >= types.len() {
                                return false;
                            }
                            if !self.match_pattern(
                                fn_name,
                                &(t.clone(), None),
                                &TypeValue::Type(types[idx].clone()),
                                bindings,
                                span,
                            ) {
                                return false;
                            }
                        }
                    }
                    i += 1
                }
                false
            }
            PatternTerm::Table(_) => {
                self.err(
                    fn_name,
                    "structural (table) matching in type functions is not yet supported",
                    span,
                );
                false
            }
            _ => false,
        }) && ({
            if let Some(g) = &pattern.1 {
                self.eval_cond(fn_name, g)
            } else {
                true
            }
        })
    }

    fn match_type_ctor(
        &mut self,
        fn_name: &str,
        name: &str,
        args: &[PatternTerm],
        target: &TypeValue,
        bindings: &mut HashMap<Box<str>, (TypeValue, bool)>,
        span: Span,
    ) -> bool {
        let (target_ty, tag_id) = match target {
            TypeValue::Type(ty) => (ty.clone(), None),
            TypeValue::Tagged { ty, id } => (ty.clone(), Some(*id)),
            _ => return false,
        };
        if let Some(id) = tag_id {
            let entry = self.results.lock().unwrap().get(id).cloned();
            if let Some((ctor, call_args, _)) = entry
                && ctor.as_ref() == name
            {
                for (i, pat) in args.iter().enumerate() {
                    let Some(orig) = call_args.get(i) else {
                        return false;
                    };
                    if !self.match_pattern(fn_name, &(pat.clone(), None), orig, bindings, span) {
                        return false;
                    }
                }
                return true;
            }
        }
        match (name, target_ty) {
            (ctype::ARR, Type::Array(inner_t)) | (ctype::LIS, Type::Array(inner_t)) => {
                if args.is_empty() {
                    return true;
                }
                if args.len() != 1 {
                    self.err(
                        fn_name,
                        format!("'{name}' pattern expects 0 or 1 arguments"),
                        span,
                    );
                    return false;
                }
                match inner_t.as_deref() {
                    Some(t) => self.match_pattern(
                        fn_name,
                        &(args[0].clone(), None),
                        &TypeValue::Type(t.clone()),
                        bindings,
                        span,
                    ),
                    None => {
                        self.err(fn_name, "cannot structurally match an untyped list", span);
                        false
                    }
                }
            }
            (ctype::TAB, Type::Table(k, v)) => {
                if args.is_empty() {
                    return true;
                }
                if args.len() != 2 {
                    self.err(
                        fn_name,
                        format!("'Table' pattern expects 0 or 2 arguments"),
                        span,
                    );
                    return false;
                }
                let (Some(k), Some(v)) = (k.as_deref(), v.as_deref()) else {
                    self.err(fn_name, "cannot structurally match an untyped table", span);
                    return false;
                };
                self.match_pattern(
                    fn_name,
                    &(args[0].clone(), None),
                    &TypeValue::Type(k.clone()),
                    bindings,
                    span,
                ) && self.match_pattern(
                    fn_name,
                    &(args[1].clone(), None),
                    &TypeValue::Type(v.clone()),
                    bindings,
                    span,
                )
            }
            (ctype::FUN, Type::Function(ft)) => {
                let (params, returns) = if let Some(ft) = ft {
                    (
                        TypeValue::Type(Type::TypeTuple(ft.params.to_vec())),
                        TypeValue::Type(Type::TypeTuple(ft.returns.to_vec())),
                    )
                } else {
                    (TypeValue::Type(Type::Any), TypeValue::Type(Type::Any))
                };
                self.match_pattern(fn_name, &(args[0].clone(), None), &params, bindings, span)
                    && self.match_pattern(
                        fn_name,
                        &(args[1].clone(), None),
                        &returns,
                        bindings,
                        span,
                    )
            }
            (ctype::OBJ, Type::Object { id, .. }) => {
                let obj = &self.objects[id];
                let inner = obj
                    .members
                    .iter()
                    .map(|i| {
                        (
                            ConstValue::String(i.name.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(i.ty.clone().expect_pure().unwrap_or(Type::Any)),
                        )
                    })
                    .chain(obj.methods.iter().map(|i| {
                        (
                            ConstValue::String(i.name.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(Type::Function(Some(i.sig.clone()))),
                        )
                    }));
                let props = TypeValue::Type(Type::TypeTable(inner.collect()));
                self.match_pattern(fn_name, &(args[0].clone(), None), &props, bindings, span)
            }
            _ => false,
        }
    }

    fn tailcall_target<'b>(&self, expr: &'b Expr) -> Option<(Box<str>, &'b [Expr], Span)> {
        let ExprKind::Call(callee, args) = &expr.0 else {
            return None;
        };
        let ExprKind::Access(path) = &callee.0 else {
            return None;
        };
        let Path::Base((name, _)) = path.as_ref() else {
            return None;
        };
        let Some(symbol) = self.viewer.lookup(name) else {
            return None;
        };
        if matches!(&symbol.symbol_type, SymbolType::TypeFunction(_)) {
            return Some((name.clone().into(), args, expr.1));
        }
        None
    }

    fn unsupported(&mut self, fn_name: &str, span: Span) -> TypeValue {
        self.err(
            fn_name,
            "unsupported expression in type function body",
            span,
        );
        TypeValue::Type(Type::Any)
    }

    fn eval_expr_to_type(&mut self, fn_name: &str, expr: &Expr, caller_span: Span) -> TypeValue {
        fn cmp(
            a: ConstValue,
            b: ConstValue,
            fi: fn(DukaInt, DukaInt) -> bool,
            ff: fn(DukaFloat, DukaFloat) -> bool,
        ) -> ConstValue {
            match (a, b) {
                (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Bool(ff(a, b)),
                (ConstValue::Float(a), ConstValue::Int(b)) => {
                    ConstValue::Bool(ff(a, b as DukaFloat))
                }
                (ConstValue::Int(a), ConstValue::Float(b)) => {
                    ConstValue::Bool(ff(a as DukaFloat, b))
                }
                (ConstValue::Int(a), ConstValue::Int(b)) => ConstValue::Bool(fi(a, b)),
                _ => unreachable!(),
            }
        }
        fn calc(
            a: ConstValue,
            b: ConstValue,
            fi: fn(DukaInt, DukaInt) -> DukaInt,
            ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
        ) -> ConstValue {
            match (a, b) {
                (ConstValue::Float(a), ConstValue::Float(b)) => ConstValue::Float(ff(a, b)),
                (ConstValue::Float(a), ConstValue::Int(b)) => {
                    ConstValue::Float(ff(a, b as DukaFloat))
                }
                (ConstValue::Int(a), ConstValue::Float(b)) => {
                    ConstValue::Float(ff(a as DukaFloat, b))
                }
                (ConstValue::Int(a), ConstValue::Int(b)) => ConstValue::Int(fi(a, b)),
                _ => unreachable!(),
            }
        }

        match &expr.0 {
            ExprKind::Table(fields) => {
                let r = fields
                    .iter()
                    .map(|f| match f {
                        Field::KeyValue(Expr(_, span), _) | Field::Value(Expr(_, span)) => Err((
                            "key-value and value are not supported for typed table",
                            span,
                        )),
                        Field::NameValue(n, v) => Ok((
                            ConstValue::String(n.0.as_bytes().to_vec().into_boxed_slice()),
                            self.eval_expr_to_type(fn_name, v, caller_span),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>();
                match r {
                    Ok(k) => TypeValue::Type(Type::TypeTable(
                        k.into_iter()
                            .map(|(k, v)| (k, Box::new(v.to_type())))
                            .collect(),
                    )),
                    Err(e) => {
                        self.err(fn_name, e.0, *e.1);
                        TypeValue::Type(Type::Any)
                    }
                }
            }
            ExprKind::Array(items) => TypeValue::Type(Type::TypeTuple(
                items
                    .iter()
                    .map(|i| self.eval_expr_to_type(fn_name, i, caller_span).to_type())
                    .collect(),
            )),
            ExprKind::If(ifb) => match self.eval_if(fn_name, ifb) {
                Return::Value(v) => v,
                Return::Tail(name, args, span) => self.call_type_fn(&name, args, span),
                _ => TypeValue::Type(Type::Never),
            },
            ExprKind::Unary(who, op) => {
                let ty = self.eval_expr_to_type(fn_name, who, caller_span);
                match (ty.without_tag(), op) {
                    (TypeValue::Type(Type::Literal(ConstValue::Bool(b))), UnOp::Not) => {
                        TypeValue::Type(Type::Literal(ConstValue::Bool(!b)))
                    }
                    (TypeValue::Type(Type::Literal(ConstValue::Int(i))), UnOp::Minus) => {
                        TypeValue::Type(Type::Literal(ConstValue::Int(-i)))
                    }
                    (TypeValue::Type(Type::Literal(ConstValue::Float(f))), UnOp::Minus) => {
                        TypeValue::Type(Type::Literal(ConstValue::Float(-f)))
                    }
                    (TypeValue::Type(Type::Literal(ConstValue::String(s))), UnOp::Length) => {
                        TypeValue::Type(Type::Literal(ConstValue::Int(s.len() as DukaInt)))
                    }
                    (TypeValue::Type(Type::TypeTable(l)), UnOp::Length) => {
                        TypeValue::Type(Type::Literal(ConstValue::Int(l.len() as DukaInt)))
                    }
                    (TypeValue::Type(Type::TypeTuple(l)), UnOp::Length) => {
                        TypeValue::Type(Type::Literal(ConstValue::Int(l.len() as DukaInt)))
                    }
                    _ => self.unsupported(fn_name, caller_span),
                }
            }
            ExprKind::TypeLit(ty) => self.eval_type(ty),
            ExprKind::Literal(cv) => TypeValue::Type(Type::Literal(cv.clone())),
            ExprKind::Access(path) => self
                .eval_path_to_type(fn_name, path.as_ref(), caller_span)
                .unwrap_or_default(),
            ExprKind::Call(callee, args) => {
                let callee_name = match &callee.0 {
                    ExprKind::Access(path) => {
                        let Path::Base((n, _)) = path.as_ref() else {
                            self.err(
                                fn_name,
                                "unsupported callee, only a type function name",
                                caller_span,
                            );
                            return TypeValue::Type(Type::Any);
                        };
                        n.clone()
                    }
                    _ => {
                        self.err(
                            fn_name,
                            "unsupported callee, only a type function name",
                            caller_span,
                        );
                        return TypeValue::Type(Type::Any);
                    }
                };
                let args: Box<[TypeValue]> = args
                    .iter()
                    .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                    .collect();
                if let Some(TypeValue::Closure(c)) = self.lookup_frame(&callee_name) {
                    return self.apply_closure(&c, args, caller_span);
                }
                self.call_type_fn(&callee_name, args, caller_span)
            }

            ExprKind::Binary(a, b, BinOp::Equal) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1).without_tag();
                let tb = self.eval_expr_to_type(fn_name, b, b.1).without_tag();
                TypeValue::Type(Type::Literal(ConstValue::Bool(ta == tb)))
            }
            ExprKind::Binary(a, b, BinOp::NotEqual) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1).without_tag();
                let tb = self.eval_expr_to_type(fn_name, b, b.1).without_tag();
                TypeValue::Type(Type::Literal(ConstValue::Bool(ta != tb)))
            }
            ExprKind::Binary(a, b, op) => {
                let (TypeValue::Type(Type::Literal(a)), TypeValue::Type(Type::Literal(b))) = (
                    self.eval_expr_to_type(fn_name, a, caller_span)
                        .without_tag(),
                    self.eval_expr_to_type(fn_name, b, caller_span)
                        .without_tag(),
                ) else {
                    return self.unsupported(fn_name, caller_span);
                };
                TypeValue::Type(Type::Literal(match (a, b, op) {
                    (ConstValue::String(a), ConstValue::String(b), BinOp::Concat) => {
                        let c = [a, b].concat();
                        ConstValue::String(c.into_boxed_slice())
                    }
                    (a, b, BinOp::Concat) => ConstValue::String(
                        format!("{}{}", a.to_string(), b.to_string())
                            .into_bytes()
                            .into_boxed_slice(),
                    ),
                    (
                        a @ ConstValue::Float(..) | a @ ConstValue::Int(..),
                        b @ ConstValue::Float(..) | b @ ConstValue::Int(..),
                        op,
                    ) => match op {
                        BinOp::Add => calc(a, b, Add::add, Add::add),
                        BinOp::Sub => calc(a, b, Sub::sub, Sub::sub),
                        BinOp::Multiply => calc(a, b, Mul::mul, Mul::mul),
                        BinOp::Divide => calc(a, b, Div::div, Div::div),
                        BinOp::Less => cmp(a, b, |a, b| a < b, |a, b| a < b),
                        BinOp::LessEqual => cmp(a, b, |a, b| a <= b, |a, b| a <= b),
                        BinOp::Greater => cmp(a, b, |a, b| a > b, |a, b| a > b),
                        BinOp::GreaterEqual => cmp(a, b, |a, b| a >= b, |a, b| a >= b),
                        _ => return self.unsupported(fn_name, caller_span),
                    },
                    _ => {
                        return self.unsupported(fn_name, caller_span);
                    }
                }))
            }
            ExprKind::Match(m) => match self.eval_match(fn_name, m) {
                Return::Value(v) => v,
                Return::Tail(name, args, span) => self.call_type_fn(&name, args, span),
                _ => {
                    self.err(fn_name, "no match clause matched the type", caller_span);
                    TypeValue::Type(Type::Any)
                }
            },
            _ => {
                self.err(
                    fn_name,
                    "unsupported expression in type function body",
                    caller_span,
                );
                TypeValue::Type(Type::Any)
            }
        }
    }

    fn eval_path_to_type(
        &mut self,
        fn_name: &str,
        mut path: &Path,
        span: Span,
    ) -> Option<TypeValue> {
        let mut base_type;
        let mut suffixes = vec![];
        loop {
            match path {
                Path::Base((key, _)) => {
                    if let Some(t) = self.lookup_frame(key) {
                        base_type = t;
                        break;
                    }
                    if let Some(t) = Type::from_keyword(key) {
                        if suffixes.is_empty() {
                            return Some(TypeValue::Type(t));
                        } else {
                            self.err(fn_name, "unsupported name in path", span);
                            return None;
                        }
                    }
                    base_type = match self.viewer.lookup(key) {
                        Some(symbol) => match symbol.symbol_type.clone() {
                            SymbolType::TypeAlias(id) => {
                                if let Some((_, tv)) = self.aliases.get(id) {
                                    self.eval_type(tv)
                                } else {
                                    self.err(fn_name, "unknown type alias", span);
                                    return None;
                                }
                            }
                            SymbolType::ObjectClass(id) => {
                                if let Some(o) = self.objects.get(id) {
                                    TypeValue::Type(Type::Object {
                                        id,
                                        name: o.name.clone(),
                                        base: o.base,
                                        args: Box::new([]),
                                    })
                                } else {
                                    self.err(fn_name, "unknown object type", span);
                                    return None;
                                }
                            }
                            SymbolType::TypeFunction(id) => {
                                if let Some(fn_def) = self.type_fns.get(id) {
                                    TypeValue::Closure(Box::new(TypeClosure {
                                        name: key.clone().into_boxed_str(),
                                        params: fn_def.body.0.clone(),
                                        body: fn_def.body.clone(),
                                        captured: vec![],
                                    }))
                                } else {
                                    self.err(fn_name, "unknown type function", span);
                                    return None;
                                }
                            }
                            SymbolType::InlineTypeFunction(_) => TypeValue::Type(Type::Any),
                            _ => {
                                self.err(fn_name, format!("'{key}' is not a type"), span);
                                return None;
                            }
                        },
                        None => {
                            self.err(fn_name, format!("unknown type '{key}'"), span);
                            return None;
                        }
                    };
                    break;
                }
                Path::Expr(expr) => {
                    base_type = self.eval_expr_to_type(fn_name, expr, span);
                    break;
                }
                Path::Chain(next, suffix) => {
                    path = next;
                    suffixes.push(suffix);
                    continue;
                }
            }
        }

        for s in suffixes.into_iter().rev() {
            match s {
                PathSuffix::Dot((k, _)) | PathSuffix::Colon((k, _)) => {
                    let Some(v) = self.eval_type_access(
                        fn_name,
                        base_type,
                        TypeValue::Type(Type::Literal(ConstValue::String(k.as_bytes().into()))),
                        span,
                    ) else {
                        self.err(fn_name, "unknown key", span);
                        return None;
                    };
                    base_type = v;
                }
                PathSuffix::Index(expr) => {
                    let member = self.eval_expr_to_type(fn_name, expr, span);
                    let Some(v) = self.eval_type_access(fn_name, base_type, member, span) else {
                        self.err(fn_name, "unknown index", span);
                        return None;
                    };
                    base_type = v;
                }
                PathSuffix::TypeArgs(..) => continue,
            }
        }

        Some(base_type)
    }
}

// Eval type in AST
impl Visitor for EvalCtx<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.0 {
            StmtKind::Define(names, ..) => {
                for an in names.iter() {
                    if let Some(ty) = &an.0.2 {
                        self.eval_type(ty);
                    }
                }
            }
            StmtKind::TypeAlias(_, ty) => {
                self.eval_type(ty);
            }
            StmtKind::Function(_, _, body, _) => {
                self.eval_func_annotations(body);
            }
            StmtKind::TypeFunction(_, body) => {
                self.eval_func_annotations(body);
            }
            StmtKind::Match(m) => {
                for clause in m.1.iter() {
                    if let PatternTerm::Bind(_, Some(ty)) = &clause.0.0 {
                        self.eval_type(ty);
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        if let ExprKind::Access(path) = &expr.0 {
            self.eval_path(path.as_ref());
        }
    }
}

// Eval type in AST
impl EvalCtx<'_> {
    fn eval_func_annotations(&mut self, body: &crate::parser::ast::FuncBody) {
        for param in body.0.iter() {
            if let Param::Typed((_, _), ty) = param {
                self.eval_type(ty);
            }
        }
        if let Some(ty) = &body.2 {
            for t in ty.tys.iter() {
                self.eval_type(t);
            }
        }
    }

    fn eval_path(&mut self, path: &Path) {
        match path {
            Path::Base(_) | Path::Expr(_) => {}
            Path::Chain(p, PathSuffix::TypeArgs(args, _)) => {
                self.eval_path(p.as_ref());
                for a in args.iter() {
                    self.eval_type(a);
                }
            }
            Path::Chain(p, _) => self.eval_path(p.as_ref()),
        }
    }
}

fn literal_num(t: &TypeValue) -> Option<f64> {
    match t {
        TypeValue::Type(Type::Literal(cv)) => match cv {
            ConstValue::Int(i) => Some(*i as f64),
            ConstValue::Float(f) => Some(*f),
            _ => None,
        },
        _ => None,
    }
}

fn num_cv(v: f64) -> ConstValue {
    if v.fract() == 0.0 {
        ConstValue::Int(v as i64)
    } else {
        ConstValue::Float(v)
    }
}
