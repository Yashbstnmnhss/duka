use std::ops::{Div, Mul, Sub};
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, ops::Add};

use duka_shared::constants::ctype;
use duka_shared::types::UnOp;
use duka_shared::{
    dtype::Type,
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo},
    utils::{SymbolTableViewer, SymbolType},
    value::{ConstValue, DukaFloat, DukaInt},
};

use crate::analyzer::builtin::TYPE_BUILTINS;
use crate::parser::ast::{Field, PatternArrayTerm, PatternOp};
use crate::{
    analyzer::{AnalyzerData, ObjectType, TypeFn, Visit, Visitor},
    parser::ast::{
        DukaChunk, Expr, ExprKind, If, Match, Param, Path, PathSuffix, PatternTerm, Stmt, StmtKind,
        TypeFnValue, TypeValue,
    },
};

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
        let results = analysis.call_cache.clone();
        let mut ctx = EvalCtx::new(
            Arc::new(chunk.source_info.clone()),
            SymbolTableViewer::new(&analysis.symbols),
            &analysis.type_fns,
            &analysis.objects,
            &analysis.aliases,
            results,
        );
        chunk.visit(&mut ctx);
        let errors = std::mem::take(&mut ctx.errors);
        analysis.type_results = ctx.results.lock().unwrap().clone();
        ((config, analysis), errors.into_iter())
    }
}

const MAX_DEPTH: usize = 32;
const MAX_ITERS: usize = 1000;

/// 以递归计算运行时type
pub(crate) struct EvalCtx<'a> {
    source: Arc<SourceInfo>,
    viewer: SymbolTableViewer<'a>,
    type_fns: &'a [TypeFn],
    objects: &'a [ObjectType],
    aliases: &'a [(Box<str>, TypeValue)],
    frames: Vec<HashMap<Box<str>, (TypeValue, bool)>>,
    results: Arc<Mutex<Vec<(Box<str>, Box<[TypeValue]>, TypeValue)>>>,
    depth: usize,
    pub(crate) errors: Vec<DukaSpannedError>,
    call_span_stack: Vec<Span>,
}

enum Return<T> {
    Value(T),
    None,
    Break,
    Continue,
    Tail(Box<str>, Box<[T]>, Span),
}

impl<'a> EvalCtx<'a> {
    pub(crate) fn new(
        source: Arc<SourceInfo>,
        viewer: SymbolTableViewer<'a>,
        type_fns: &'a [TypeFn],
        objects: &'a [ObjectType],
        aliases: &'a [(Box<str>, TypeValue)],
        results: Arc<Mutex<Vec<(Box<str>, Box<[TypeValue]>, TypeValue)>>>,
    ) -> Self {
        Self {
            source,
            viewer,
            type_fns,
            objects,
            aliases,
            frames: vec![HashMap::new()],
            results,
            depth: 0,
            errors: vec![],
            call_span_stack: vec![],
        }
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

    pub(crate) fn eval_type(&mut self, ty: &TypeValue) -> TypeValue {
        match ty {
            TypeValue::TypeCall { name, args, span } => {
                let args: Box<[TypeValue]> = args.iter().map(|a| self.eval_type(a)).collect();
                if name.as_ref() == ctype::REQUIRE {
                    TypeValue::TypeCall {
                        name: name.clone(),
                        args,
                        span: *span,
                    }
                } else if args.iter().all(TypeValue::is_resolved) {
                    self.call_type_fn(name, args, *span)
                } else {
                    TypeValue::TypeCall {
                        name: name.clone(),
                        args,
                        span: *span,
                    }
                }
            }
            TypeValue::Access {
                base,
                member,
                args,
                span,
            } => TypeValue::Access {
                base: Box::new(self.eval_type(base)),
                member: member.clone(),
                args: args
                    .as_ref()
                    .map(|a| a.iter().map(|x| self.eval_type(x)).collect()),
                span: *span,
            },
            TypeValue::Array(e) => TypeValue::array_of(e.as_deref().map(|e| self.eval_type(e))),
            TypeValue::Table(k, v) => TypeValue::table_of(
                k.as_deref().map(|k| self.eval_type(k)),
                v.as_deref().map(|v| self.eval_type(v)),
            ),
            TypeValue::Union(ts) => ts
                .iter()
                .map(|t| self.eval_type(t))
                .reduce(TypeValue::union)
                .unwrap_or(TypeValue::Pure(Type::Never)),
            TypeValue::Function(ft) => TypeValue::function_of(ft.as_ref().map(|ft| TypeFnValue {
                params: ft.params.iter().map(|t| self.eval_type(t)).collect(),
                var_arg: ft.var_arg,
                returns: ft.returns.iter().map(|t| self.eval_type(t)).collect(),
                return_var_arg: ft.return_var_arg,
            })),
            TypeValue::TypeTuple(ts) => {
                TypeValue::tuple_of(ts.iter().map(|t| self.eval_type(t)).collect())
            }
            TypeValue::TypeTable(ts) => TypeValue::typetable_of(
                ts.iter()
                    .map(|(k, v)| (k.clone(), self.eval_type(v)))
                    .collect(),
            ),
            TypeValue::Generic { name, args, span } => TypeValue::Generic {
                name: name.clone(),
                args: args.iter().map(|a| self.eval_type(a)).collect(),
                span: *span,
            },
            TypeValue::Named(name, _) => {
                if let Some(t) = self.lookup_frame(name) {
                    t
                } else if let Some(symbol) = self.viewer.lookup(name) {
                    match symbol.symbol_type.clone() {
                        SymbolType::TypeAlias(id) => {
                            if let Some((_, tv)) = self.aliases.get(id) {
                                self.eval_type(tv)
                            } else {
                                TypeValue::Named(name.clone(), symbol.span)
                            }
                        }
                        SymbolType::ObjectClass(id) => {
                            if let Some(o) = self.objects.get(id) {
                                TypeValue::Pure(Type::Object {
                                    id,
                                    name: o.name.clone(),
                                    base: o.base,
                                    args: Box::new([]),
                                })
                            } else {
                                TypeValue::Named(name.clone(), symbol.span)
                            }
                        }
                        _ => TypeValue::Named(name.clone(), symbol.span),
                    }
                } else {
                    ty.clone()
                }
            }
            other => other.clone(),
        }
    }

    pub(crate) fn call_type_fn(
        &mut self,
        name: &str,
        args: Box<[TypeValue]>,
        span: Span,
    ) -> TypeValue {
        let cached = {
            let results = self.results.lock().unwrap();
            results
                .iter()
                .position(|(n, a, _)| n.as_ref() == name && a == &args)
                .map(|idx| (idx, results[idx].2.clone()))
        };
        if let Some((idx, res)) = cached {
            return match res {
                TypeValue::Tagged { ty, .. } => TypeValue::Tagged { ty, id: idx },
                TypeValue::Pure(ty) => TypeValue::Tagged { ty, id: idx },
                _ => TypeValue::Pure(Type::Any),
            };
        }
        if self.depth >= MAX_DEPTH {
            self.err(
                name,
                format!("reached max recursion depth ({MAX_DEPTH})"),
                span,
            );
            return TypeValue::Pure(Type::Any);
        }
        let Some(symbol) = self.viewer.lookup(name) else {
            return TypeValue::Pure(self.call_builtin_or_unknown(name, args, span));
        };
        let SymbolType::TypeFunction(id) = symbol.symbol_type.clone() else {
            self.err(name, "not a type function", span);
            return TypeValue::Pure(Type::Any);
        };
        let Some(fn_def) = self.type_fns.get(id) else {
            self.err(name, "type function body missing", span);
            return TypeValue::Pure(Type::Any);
        };
        let Some(frame) = self.bind_params(name, &fn_def.body.0, &args, span) else {
            return TypeValue::Pure(Type::Any);
        };
        self.frames.push(frame);
        self.depth += 1;
        self.call_span_stack.push(span);
        let outer_name = name.to_owned();
        let outer_args = args;
        let mut current_name: Box<str> = name.into();
        let mut current_def: &'a TypeFn = fn_def;
        let mut iters = 0;
        let result = loop {
            iters += 1;
            if iters > MAX_ITERS {
                self.err(
                    &current_name,
                    format!("type function tail recursion exceeded max iterations ({MAX_ITERS})"),
                    span,
                );
                break TypeValue::Pure(Type::Any);
            }
            match self.eval_block(&current_name, &current_def.body.3) {
                Return::Value(v) => break v,
                Return::Tail(next_name, next_args, next_span) => {
                    let Some(symbol) = self.viewer.lookup(&next_name) else {
                        break TypeValue::Pure(
                            self.call_builtin_or_unknown(&next_name, next_args, next_span),
                        );
                    };
                    let SymbolType::TypeFunction(next_id) = symbol.symbol_type.clone() else {
                        self.err(&next_name, "not a type function", next_span);
                        break TypeValue::Pure(Type::Any);
                    };
                    let Some(next_def) = self.type_fns.get(next_id) else {
                        self.err(&next_name, "type function body missing", next_span);
                        break TypeValue::Pure(Type::Any);
                    };
                    let Some(next_frame) =
                        self.bind_params(&next_name, &next_def.body.0, &next_args, next_span)
                    else {
                        break TypeValue::Pure(Type::Any);
                    };
                    if let Some(top) = self.frames.last_mut() {
                        *top = next_frame;
                    }
                    current_name = next_name;
                    current_def = next_def;
                }
                Return::Break | Return::Continue | Return::None => {
                    break TypeValue::Pure(Type::Never);
                }
            }
        };
        self.call_span_stack.pop();
        self.depth -= 1;
        self.frames.pop();
        let ty = result.unwrap_type();
        let mut cache = self.results.lock().unwrap();
        let id = cache.len();
        let tagged = TypeValue::Tagged { ty: ty.clone(), id };
        cache.push((outer_name.into(), outer_args, tagged.clone()));
        drop(cache);
        tagged
    }

    fn call_builtin_or_unknown(&mut self, name: &str, args: Box<[TypeValue]>, span: Span) -> Type {
        let Ok(bi) = TYPE_BUILTINS.read() else {
            self.err(name, "failed to load builtin type functions", span);
            return Type::Any;
        };
        let Some(f) = bi.get(&name) else {
            self.err(name, "unknown type function", span);
            return Type::Any;
        };
        let Some(pure_args) = args
            .iter()
            .map(|a| a.clone().expect_pure())
            .collect::<Option<Box<[Type]>>>()
        else {
            self.err(name, "builtin type function requires pure arguments", span);
            return Type::Any;
        };
        match f(pure_args) {
            Err(msg) => {
                self.err(name, format!("builtin type function error: {msg}"), span);
                Type::Any
            }
            Ok(result) => {
                self.results.lock().unwrap().push((
                    name.into(),
                    args,
                    TypeValue::Pure(result.clone()),
                ));
                result
            }
        }
    }

    fn bind_params(
        &mut self,
        fn_name: &str,
        params: &[Param],
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
        let mut frame = HashMap::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            match param {
                Param::Typed((n, _), t) => {
                    if let (Some(tt), Some(aa)) = (t.clone().expect_pure(), Some(arg.unwrap_type()))
                    {
                        if !tt.accepts(&aa) {
                            self.err(
                                fn_name,
                                format!("argument {n} has invalid type, expected {t}"),
                                span,
                            );
                            return None;
                        }
                    }
                    frame.insert(n.clone().into_boxed_str(), (arg.clone(), false));
                }
                Param::Name((n, _)) => {
                    frame.insert(n.clone().into_boxed_str(), (arg.clone(), false));
                }
                Param::Var(_) => {
                    self.err(fn_name, "var args not supported in type function", span);
                    return None;
                }
            }
        }
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
                StmtKind::Return(exprs) => {
                    if exprs.len() == 1
                        && let Some((tail_name, tail_args, tail_span)) =
                            self.tail_call_target(&exprs[0])
                    {
                        let args: Box<[TypeValue]> = tail_args
                            .iter()
                            .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                            .collect();
                        return Return::Tail(tail_name, args, tail_span);
                    }
                    r((!exprs.is_empty()).then(|| {
                        TypeValue::tuple_of(
                            exprs
                                .iter()
                                .map(|e| self.eval_expr_to_type(fn_name, e, stmt.1))
                                .collect(),
                        )
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
                StmtKind::Define(names, exprs, is_global) => {
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
                        let Path::Base((key, _)) = p else {
                            self.err(
                                fn_name,
                                "unsupported assignment target in type function",
                                stmt.1,
                            );
                            return Return::None;
                        };
                        let Some(idx) = self.find_frame(key) else {
                            self.err(fn_name, format!("unknown type local '{key}'"), stmt.1);
                            return Return::None;
                        };
                        if !self.frames[idx]
                            .get(key.as_str())
                            .map(|(_, m)| *m)
                            .unwrap_or(false)
                        {
                            self.err(
                                fn_name,
                                format!("cannot assign to immutable type local '{key}'"),
                                stmt.1,
                            );
                            return Return::None;
                        }
                        let v = self.eval_expr_to_type(fn_name, e, e.1);
                        self.frames[idx].insert(key.clone().into_boxed_str(), (v, true));
                    }
                    Return::None
                }
                StmtKind::While(cond, body) => {
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
                        None => TypeValue::Pure(Type::Literal(ConstValue::Int(1))),
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
                    while (inc > 0.0 && i <= stop) || (inc < 0.0 && i >= stop) {
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
                            (TypeValue::Pure(Type::Literal(num_cv(i))), false),
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
                StmtKind::ForGeneric(paths, exprs, body) => {
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
                        TypeValue::Pure(Type::TypeTuple(tuple)) => match paths.iter().as_slice() {
                            [Path::Base((val, _))] => {
                                for t in tuple {
                                    self.frames.push(HashMap::from([(
                                        val.clone().into_boxed_str(),
                                        (TypeValue::Pure(t), false),
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
                                                TypeValue::Pure(Type::Literal(ConstValue::Int(
                                                    i as DukaInt,
                                                ))),
                                                false,
                                            ),
                                        ),
                                        (val.clone().into_boxed_str(), (TypeValue::Pure(t), false)),
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
                        TypeValue::Pure(Type::Object { id, .. }) => {
                            let obj = &self.objects[id];
                            let properties = &obj.members;
                            let methods = &obj.methods;
                            match paths.iter().as_slice() {
                                [Path::Base((key, _)), Path::Base((val, _))] => {
                                    for (k, v) in properties
                                        .iter()
                                        .map(|i| (&i.name, i.ty.clone()))
                                        .chain(methods.iter().map(|i| {
                                            (
                                                &i.name,
                                                TypeValue::Pure(Type::Function(Some(
                                                    i.sig.clone(),
                                                ))),
                                            )
                                        }))
                                    {
                                        self.frames.push(HashMap::from([
                                            (
                                                key.clone().into_boxed_str(),
                                                (
                                                    TypeValue::Pure(Type::Literal(
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
                        TypeValue::Pure(Type::TypeTable(properties)) => {
                            match paths.iter().as_slice() {
                                [Path::Base((key, _)), Path::Base((val, _))] => {
                                    for (k, v) in properties.into_iter() {
                                        self.frames.push(HashMap::from([
                                            (
                                                key.clone().into_boxed_str(),
                                                (
                                                    TypeValue::Pure(Type::Literal(
                                                        ConstValue::String(k.into_boxed_bytes()),
                                                    )),
                                                    false,
                                                ),
                                            ),
                                            (
                                                val.clone().into_boxed_str(),
                                                (TypeValue::Pure(*v), false),
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
                StmtKind::Do(blk) => {
                    self.frames.push(HashMap::new());
                    let res = self.eval_block(fn_name, blk);
                    self.frames.pop();
                    res
                }
                _ => Return::None,
            };
            if let r @ (Return::Value(_) | Return::Tail(..)) = ret {
                return r;
            }
        }
        if let Some(stmt) = &block.1 {
            if let StmtKind::Return(exprs) = &stmt.0 {
                if let Some(e) = exprs.first() {
                    if let Some((tail_name, tail_args, tail_span)) = self.tail_call_target(e) {
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
            TypeValue::Pure(Type::Literal(ConstValue::Nil))
            | TypeValue::Pure(Type::Nil)
            | TypeValue::Pure(Type::Never) => false,
            TypeValue::Pure(Type::Literal(ConstValue::Bool(b))) => b,
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
                let TypeValue::TypeTuple(types) = target else {
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
                                &types[idx],
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
        } && {
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
            TypeValue::Pure(ty) => (ty.clone(), None),
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
                        &TypeValue::Pure(t.clone()),
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
                    &TypeValue::Pure(k.clone()),
                    bindings,
                    span,
                ) && self.match_pattern(
                    fn_name,
                    &(args[1].clone(), None),
                    &TypeValue::Pure(v.clone()),
                    bindings,
                    span,
                )
            }
            (ctype::FUN, Type::Function(ft)) => {
                let (params, returns) = if let Some(ft) = ft {
                    (
                        TypeValue::Pure(Type::TypeTuple(ft.params.clone())),
                        TypeValue::Pure(Type::TypeTuple(ft.returns.clone())),
                    )
                } else {
                    (TypeValue::Pure(Type::Any), TypeValue::Pure(Type::Any))
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
                            i.name.clone(),
                            Box::new(i.ty.clone().expect_pure().unwrap_or(Type::Any)),
                        )
                    })
                    .chain(obj.methods.iter().map(|i| {
                        (
                            i.name.clone(),
                            Box::new(Type::Function(Some(i.sig.clone()))),
                        )
                    }));
                let props = TypeValue::Pure(Type::TypeTable(inner.collect()));
                self.match_pattern(fn_name, &(args[0].clone(), None), &props, bindings, span)
            }
            _ => false,
        }
    }

    fn tail_call_target<'b>(&self, expr: &'b Expr) -> Option<(Box<str>, &'b [Expr], Span)> {
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
        TypeValue::Pure(Type::Any)
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
                            n.0.clone().into_boxed_str(),
                            self.eval_expr_to_type(fn_name, v, caller_span),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>();
                match r {
                    Ok(k) => TypeValue::typetable_of(k.into_boxed_slice()),
                    Err(e) => {
                        self.err(fn_name, e.0, *e.1);
                        TypeValue::Pure(Type::Any)
                    }
                }
            }
            ExprKind::Array(items) => TypeValue::tuple_of(
                items
                    .iter()
                    .map(|i| self.eval_expr_to_type(fn_name, i, caller_span))
                    .collect(),
            ),
            ExprKind::If(ifb) => match self.eval_if(fn_name, ifb) {
                Return::Value(v) => v,
                Return::Tail(name, args, span) => self.call_type_fn(&name, args, span),
                _ => TypeValue::Pure(Type::Never),
            },
            ExprKind::Unary(who, op) => {
                let ty = self.eval_expr_to_type(fn_name, who, caller_span);
                match (ty, op) {
                    (TypeValue::Pure(Type::Literal(ConstValue::Bool(b))), UnOp::Not) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Bool(!b)))
                    }
                    (TypeValue::Pure(Type::Literal(ConstValue::Int(i))), UnOp::Minus) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Int(-i)))
                    }
                    (TypeValue::Pure(Type::Literal(ConstValue::Float(f))), UnOp::Minus) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Float(-f)))
                    }
                    (TypeValue::Pure(Type::Literal(ConstValue::String(s))), UnOp::Length) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Int(s.len() as DukaInt)))
                    }
                    (TypeValue::Pure(Type::TypeTable(l)), UnOp::Length) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Int(l.len() as DukaInt)))
                    }
                    (TypeValue::Pure(Type::TypeTuple(l)), UnOp::Length) => {
                        TypeValue::Pure(Type::Literal(ConstValue::Int(l.len() as DukaInt)))
                    }
                    _ => self.unsupported(fn_name, caller_span),
                }
            }
            ExprKind::TypeLit(ty) => self.eval_type(ty),
            ExprKind::Literal(cv) => TypeValue::Pure(Type::Literal(cv.clone())),
            ExprKind::Access(path) => self.eval_path_to_type(fn_name, path.as_ref(), caller_span),
            ExprKind::Call(callee, args) => {
                let callee_name = match &callee.0 {
                    ExprKind::Access(path) => {
                        let Path::Base((n, _)) = path.as_ref() else {
                            self.err(
                                fn_name,
                                "unsupported callee, only a type function name",
                                caller_span,
                            );
                            return TypeValue::Pure(Type::Any);
                        };
                        n.clone()
                    }
                    _ => {
                        self.err(
                            fn_name,
                            "unsupported callee, only a type function name",
                            caller_span,
                        );
                        return TypeValue::Pure(Type::Any);
                    }
                };
                let args: Box<[TypeValue]> = args
                    .iter()
                    .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                    .collect();
                self.call_type_fn(&callee_name, args, caller_span)
            }

            ExprKind::Binary(a, b, BinOp::Equal) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1);
                let tb = self.eval_expr_to_type(fn_name, b, b.1);
                TypeValue::Pure(Type::Literal(ConstValue::Bool(ta == tb)))
            }
            ExprKind::Binary(a, b, BinOp::NotEqual) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1);
                let tb = self.eval_expr_to_type(fn_name, b, b.1);
                TypeValue::Pure(Type::Literal(ConstValue::Bool(ta != tb)))
            }
            ExprKind::Binary(a, b, op) => {
                let (TypeValue::Pure(Type::Literal(a)), TypeValue::Pure(Type::Literal(b))) = (
                    self.eval_expr_to_type(fn_name, a, caller_span),
                    self.eval_expr_to_type(fn_name, b, caller_span),
                ) else {
                    return self.unsupported(fn_name, caller_span);
                };
                TypeValue::Pure(Type::Literal(match (a, b, op) {
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
                    TypeValue::Pure(Type::Any)
                }
            },
            _ => {
                self.err(
                    fn_name,
                    "unsupported expression in type function body",
                    caller_span,
                );
                TypeValue::Pure(Type::Any)
            }
        }
    }

    fn eval_path_to_type(&mut self, fn_name: &str, path: &Path, span: Span) -> TypeValue {
        let Path::Base((key, _)) = path else {
            self.err(fn_name, "unsupported path in type function body", span);
            return TypeValue::Pure(Type::Any);
        };
        if let Some(t) = self.lookup_frame(key) {
            return t;
        }
        if let Some(t) = Type::from_keyword(key) {
            return TypeValue::Pure(t);
        }
        match self.viewer.lookup(key) {
            Some(symbol) => match symbol.symbol_type.clone() {
                SymbolType::TypeAlias(id) => {
                    if let Some((_, tv)) = self.aliases.get(id) {
                        self.eval_type(tv)
                    } else {
                        TypeValue::Named(key.clone().into_boxed_str(), symbol.span)
                    }
                }
                SymbolType::ObjectClass(id) => {
                    if let Some(o) = self.objects.get(id) {
                        TypeValue::Pure(Type::Object {
                            id,
                            name: o.name.clone(),
                            base: o.base,
                            args: Box::new([]),
                        })
                    } else {
                        TypeValue::Named(key.clone().into_boxed_str(), symbol.span)
                    }
                }
                SymbolType::TypeFunction(_) => {
                    self.err(
                        fn_name,
                        format!("'{key}' is a type function, call it with ()"),
                        span,
                    );
                    TypeValue::Pure(Type::Any)
                }
                _ => {
                    self.err(fn_name, format!("'{key}' is not a type"), span);
                    TypeValue::Pure(Type::Any)
                }
            },
            None => {
                self.err(fn_name, format!("unknown type '{key}'"), span);
                TypeValue::Pure(Type::Any)
            }
        }
    }
}

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
            self.eval_path_suffix(path.as_ref());
        }
    }
}

impl EvalCtx<'_> {
    fn eval_func_annotations(&mut self, body: &crate::parser::ast::FuncBody) {
        for param in body.0.iter() {
            if let Param::Typed((_, _), ty) = param {
                self.eval_type(ty);
            }
        }
        if let Some(ty) = &body.2 {
            self.eval_type(ty);
        }
    }

    fn eval_path_suffix(&mut self, path: &Path) {
        match path {
            Path::Base(_) | Path::Expr(_) => {}
            Path::Chain(p, PathSuffix::TypeArgs(args, _)) => {
                self.eval_path_suffix(p.as_ref());
                for a in args.iter() {
                    self.eval_type(a);
                }
            }
            Path::Chain(p, _) => self.eval_path_suffix(p.as_ref()),
        }
    }
}

fn literal_num(t: &TypeValue) -> Option<f64> {
    match t {
        TypeValue::Pure(Type::Literal(cv)) => match cv {
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
