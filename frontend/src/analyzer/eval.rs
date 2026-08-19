use std::ops::{BitOr, Div, Mul, Sub};
use std::sync::Arc;
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

use crate::analyzer::builtin::TYPE_BUILTINS;
use crate::{
    analyzer::{AnalyzerData, ObjectType, TypeFn, Visit, Visitor},
    parser::ast::{
        DukaChunk, Expr, ExprKind, If, Match, Param, Path, PathSuffix, PatternTerm, Stmt, StmtKind,
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
        let mut ctx = EvalCtx::new(
            Arc::new(chunk.source_info.clone()),
            SymbolTableViewer::new(&analysis.symbols),
            &analysis.type_fns,
            &analysis.objects,
        );
        chunk.visit(&mut ctx);
        let errors = std::mem::take(&mut ctx.errors);
        analysis.type_results = std::mem::take(&mut ctx.results);
        ((config, analysis), errors.into_iter())
    }
}

const MAX_DEPTH: usize = 32;
const MAX_ITERS: usize = 1000;

/// 以递归计算运行时type
struct EvalCtx<'a> {
    source: Arc<SourceInfo>,
    viewer: SymbolTableViewer<'a>,
    type_fns: &'a [TypeFn],
    objects: &'a [ObjectType],
    frames: Vec<HashMap<Box<str>, (Type, bool)>>,
    results: Vec<(Box<str>, Box<[Type]>, Type)>,
    depth: usize,
    errors: Vec<DukaSpannedError>,
    call_span_stack: Vec<Span>,
}

enum Return<T> {
    Value(T),
    None,
    Break,
    Continue,
    Tail(Box<str>, Box<[Type]>, Span),
}

impl<'a> EvalCtx<'a> {
    fn new(
        source: Arc<SourceInfo>,
        viewer: SymbolTableViewer<'a>,
        type_fns: &'a [TypeFn],
        objects: &'a [ObjectType],
    ) -> Self {
        Self {
            source,
            viewer,
            type_fns,
            objects,
            frames: vec![HashMap::new()],
            results: vec![],
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

    fn lookup_frame(&self, key: &str) -> Option<Type> {
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

    fn eval_type(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeCall { name, args, span } => {
                let args: Box<[Type]> = args.iter().map(|a| self.eval_type(a)).collect();
                self.call_type_fn(name, args, *span)
            }
            Type::Array(Some(inner)) => Type::Array(Some(Box::new(self.eval_type(inner)))),
            Type::Array(None) => Type::Array(None),
            Type::Table(k, v) => Type::Table(
                k.as_deref().map(|k| Box::new(self.eval_type(k))),
                v.as_deref().map(|v| Box::new(self.eval_type(v))),
            ),
            Type::Union(ts) => ts
                .iter()
                .map(|t| self.eval_type(t))
                .reduce(BitOr::bitor)
                .unwrap_or(Type::Never),
            Type::Function(ft) => Type::Function(ft.as_ref().map(|ft| FunctionType {
                params: ft.params.iter().map(|t| self.eval_type(t)).collect(),
                var_arg: ft.var_arg,
                returns: ft.returns.iter().map(|t| self.eval_type(t)).collect(),
                return_var_arg: ft.return_var_arg,
            })),
            Type::Object {
                id,
                name,
                base,
                args,
            } => Type::Object {
                id: *id,
                name: name.clone(),
                base: *base,
                args: args.iter().map(|a| self.eval_type(a)).collect(),
            },
            Type::Generic { name, args } => Type::Generic {
                name: name.clone(),
                args: args.iter().map(|a| self.eval_type(a)).collect(),
            },
            Type::Named(name) => {
                if let Some(t) = self.lookup_frame(name) {
                    t
                } else if let Some(symbol) = self.viewer.lookup(name) {
                    match symbol.symbol_type.clone() {
                        SymbolType::TypeAlias(ty) => self.eval_type(&ty),
                        SymbolType::ObjectClass(id) => self
                            .objects
                            .get(id)
                            .map(|o| Type::Object {
                                id,
                                name: o.name.clone(),
                                base: o.base,
                                args: Box::new([]),
                            })
                            .unwrap_or_else(|| Type::Named(name.clone())),
                        _ => Type::Named(name.clone()),
                    }
                } else {
                    ty.clone()
                }
            }
            other => other.clone(),
        }
    }

    fn call_type_fn(&mut self, name: &str, args: Box<[Type]>, span: Span) -> Type {
        if let Some((_, _, res)) = self
            .results
            .iter()
            .find(|(n, a, _)| n.as_ref() == name && a == &args)
        {
            return res.clone();
        }
        if self.depth >= MAX_DEPTH {
            self.err(
                name,
                format!("reached max recursion depth ({MAX_DEPTH})"),
                span,
            );
            return Type::Any;
        }
        let Some(symbol) = self.viewer.lookup(name) else {
            return self.call_builtin_or_unknown(name, args, span);
        };
        let SymbolType::TypeFunction(id) = symbol.symbol_type.clone() else {
            self.err(name, "not a type function", span);
            return Type::TypeCall {
                name: name.into(),
                args,
                span,
            };
        };
        let Some(fn_def) = self.type_fns.get(id) else {
            self.err(name, "type function body missing", span);
            return Type::Any;
        };
        let Some(frame) = self.bind_params(name, &fn_def.body.0, &args, span) else {
            return Type::Any;
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
                break Type::Any;
            }
            match self.eval_block(&current_name, &current_def.body.3) {
                Return::Value(v) => break v,
                Return::Tail(next_name, next_args, next_span) => {
                    let Some(symbol) = self.viewer.lookup(&next_name) else {
                        break self.call_builtin_or_unknown(&next_name, next_args, next_span);
                    };
                    let SymbolType::TypeFunction(next_id) = symbol.symbol_type.clone() else {
                        self.err(&next_name, "not a type function", next_span);
                        break Type::TypeCall {
                            name: next_name,
                            args: next_args,
                            span: next_span,
                        };
                    };
                    let Some(next_def) = self.type_fns.get(next_id) else {
                        self.err(&next_name, "type function body missing", next_span);
                        break Type::Any;
                    };
                    let Some(next_frame) =
                        self.bind_params(&next_name, &next_def.body.0, &next_args, next_span)
                    else {
                        break Type::Any;
                    };
                    if let Some(top) = self.frames.last_mut() {
                        *top = next_frame;
                    }
                    current_name = next_name;
                    current_def = next_def;
                }
                Return::Break | Return::Continue | Return::None => break Type::Never,
            }
        };
        self.call_span_stack.pop();
        self.depth -= 1;
        self.frames.pop();
        self.results
            .push((outer_name.into(), outer_args, result.clone()));
        result
    }

    fn call_builtin_or_unknown(&mut self, name: &str, args: Box<[Type]>, span: Span) -> Type {
        let Ok(bi) = TYPE_BUILTINS.read() else {
            self.err(name, "failed to load builtin type functions", span);
            return Type::Any;
        };
        let Some(f) = bi.get(&name) else {
            self.err(name, "unknown type function", span);
            return Type::TypeCall {
                name: name.into(),
                args,
                span,
            };
        };
        match f(args.clone()) {
            Err(msg) => {
                self.err(name, format!("builtin type function error: {msg}"), span);
                Type::Any
            }
            Ok(result) => {
                self.results.push((name.into(), args, result.clone()));
                result
            }
        }
    }

    fn bind_params(
        &mut self,
        fn_name: &str,
        params: &[Param],
        args: &[Type],
        span: Span,
    ) -> Option<HashMap<Box<str>, (Type, bool)>> {
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
                    if !t.accepts(arg) {
                        self.err(
                            fn_name,
                            format!("argument {n} has invalid type, expected {t}"),
                            span,
                        );
                        return None;
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

    fn eval_block(&mut self, fn_name: &str, block: &crate::parser::ast::Block) -> Return<Type> {
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
                        let args: Box<[Type]> = tail_args
                            .iter()
                            .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                            .collect();
                        return Return::Tail(tail_name, args, tail_span);
                    }
                    r((!exprs.is_empty()).then(|| {
                        Type::TypeTuple(
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
                        None => Type::Literal(ConstValue::Int(1)),
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
                            (Type::Literal(num_cv(i)), false),
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
                        Type::TypeTuple(tuple) => match paths.iter().as_slice() {
                            [Path::Base((val, _))] => {
                                for t in tuple {
                                    self.frames.push(HashMap::from([(
                                        val.clone().into_boxed_str(),
                                        (t, false),
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
                                            (Type::Literal(ConstValue::Int(i as DukaInt)), false),
                                        ),
                                        (val.clone().into_boxed_str(), (t, false)),
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
                        Type::Object { id, .. } => {
                            let obj = &self.objects[id];
                            let properties = &obj.members;
                            let methods = &obj.methods;
                            match paths.iter().as_slice() {
                                [Path::Base((key, _)), Path::Base((val, _))] => {
                                    for (k, v) in properties
                                        .iter()
                                        .map(|i| (&i.name, i.ty.clone()))
                                        .chain(methods.iter().map(|i| {
                                            (&i.name, Type::Function(Some(i.sig.clone())))
                                        }))
                                    {
                                        self.frames.push(HashMap::from([
                                            (
                                                key.clone().into_boxed_str(),
                                                (
                                                    Type::Literal(ConstValue::String(
                                                        k.clone().into_boxed_bytes(),
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
                        Type::TypeTable(properties) => match paths.iter().as_slice() {
                            [Path::Base((key, _)), Path::Base((val, _))] => {
                                for (k, v) in properties.into_iter() {
                                    self.frames.push(HashMap::from([
                                        (
                                            key.clone().into_boxed_str(),
                                            (
                                                Type::Literal(ConstValue::String(
                                                    k.into_boxed_bytes(),
                                                )),
                                                false,
                                            ),
                                        ),
                                        (val.clone().into_boxed_str(), (*v, false)),
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
                        let args: Box<[Type]> = tail_args
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
    ) -> Return<Type> {
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
            Type::Literal(ConstValue::Nil) | Type::Nil | Type::Never => false,
            Type::Literal(ConstValue::Bool(b)) => b,
            _ => true,
        }
    }

    fn eval_match(&mut self, fn_name: &str, m: &Match) -> Return<Type> {
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
        target: &Type,
        bindings: &mut HashMap<Box<str>, (Type, bool)>,
        span: Span,
    ) -> bool {
        match &pattern.0 {
            PatternTerm::Constant(expr) => self.eval_expr_to_type(fn_name, expr, expr.1) == *target,
            PatternTerm::Bind((key, _), _) => {
                bindings.insert(key.clone().into_boxed_str(), (target.clone(), false));
                true
            }
            PatternTerm::Type((name, _), args) => {
                self.match_type_ctor(fn_name, name, args, target, bindings, span)
            }
            PatternTerm::Table(..) => {
                self.err(
                    fn_name,
                    "structural (table) matching in type functions is not yet supported",
                    span,
                );
                false
            }
            _ => false,
        }
    }

    fn match_type_ctor(
        &mut self,
        fn_name: &str,
        name: &str,
        args: &[PatternTerm],
        target: &Type,
        bindings: &mut HashMap<Box<str>, (Type, bool)>,
        span: Span,
    ) -> bool {
        match (name, target) {
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
                    Some(t) => {
                        self.match_pattern(fn_name, &(args[0].clone(), None), t, bindings, span)
                    }
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
                self.match_pattern(fn_name, &(args[0].clone(), None), k, bindings, span)
                    && self.match_pattern(fn_name, &(args[1].clone(), None), v, bindings, span)
            }
            (ctype::FUN, Type::Function(ft)) => {
                let (params, returns) = if let Some(ft) = ft {
                    (
                        Type::TypeTuple(ft.params.clone()),
                        Type::TypeTuple(ft.returns.clone()),
                    )
                } else {
                    (Type::Any, Type::Any)
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
                let obj = &self.objects[*id];
                let inner = obj
                    .members
                    .iter()
                    .map(|i| (i.name.clone(), Box::new(i.ty.clone())))
                    .chain(obj.methods.iter().map(|i| {
                        (
                            i.name.clone(),
                            Box::new(Type::Function(Some(i.sig.clone()))),
                        )
                    }));
                let props = Type::TypeTable(inner.collect());
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

    fn unsupported(&mut self, fn_name: &str, span: Span) -> Type {
        self.err(
            fn_name,
            "unsupported expression in type function body",
            span,
        );
        Type::Any
    }

    fn eval_expr_to_type(&mut self, fn_name: &str, expr: &Expr, caller_span: Span) -> Type {
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
            ExprKind::If(ifb) => match self.eval_if(fn_name, ifb) {
                Return::Value(v) => v,
                Return::Tail(name, args, span) => self.call_type_fn(&name, args, span),
                _ => Type::Never,
            },
            ExprKind::Unary(who, op) => {
                let ty = self.eval_expr_to_type(fn_name, who, caller_span);
                match (ty, op) {
                    (Type::Literal(ConstValue::Bool(b)), UnOp::Not) => {
                        Type::Literal(ConstValue::Bool(!b))
                    }
                    (Type::Literal(ConstValue::Int(i)), UnOp::Minus) => {
                        Type::Literal(ConstValue::Int(-i))
                    }
                    (Type::Literal(ConstValue::Float(f)), UnOp::Minus) => {
                        Type::Literal(ConstValue::Float(-f))
                    }
                    (Type::Literal(ConstValue::String(s)), UnOp::Length) => {
                        Type::Literal(ConstValue::Int(s.len() as DukaInt))
                    }
                    (Type::TypeTable(l), UnOp::Length) => {
                        Type::Literal(ConstValue::Int(l.len() as DukaInt))
                    }
                    (Type::TypeTuple(l), UnOp::Length) => {
                        Type::Literal(ConstValue::Int(l.len() as DukaInt))
                    }
                    _ => self.unsupported(fn_name, caller_span),
                }
            }
            ExprKind::TypeLit(ty) => self.eval_type(ty),
            ExprKind::Literal(cv) => Type::Literal(cv.clone()),
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
                            return Type::Any;
                        };
                        n.clone()
                    }
                    _ => {
                        self.err(
                            fn_name,
                            "unsupported callee, only a type function name",
                            caller_span,
                        );
                        return Type::Any;
                    }
                };
                let args: Box<[Type]> = args
                    .iter()
                    .map(|a| self.eval_expr_to_type(fn_name, a, a.1))
                    .collect();
                self.call_type_fn(&callee_name, args, caller_span)
            }

            ExprKind::Binary(a, b, BinOp::Equal) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1);
                let tb = self.eval_expr_to_type(fn_name, b, b.1);
                Type::Literal(ConstValue::Bool(ta == tb))
            }
            ExprKind::Binary(a, b, BinOp::NotEqual) => {
                let ta = self.eval_expr_to_type(fn_name, a, a.1);
                let tb = self.eval_expr_to_type(fn_name, b, b.1);
                Type::Literal(ConstValue::Bool(ta != tb))
            }
            ExprKind::Binary(a, b, op) => {
                let (Type::Literal(a), Type::Literal(b)) = (
                    self.eval_expr_to_type(fn_name, a, caller_span),
                    self.eval_expr_to_type(fn_name, b, caller_span),
                ) else {
                    return self.unsupported(fn_name, caller_span);
                };
                Type::Literal(match (a, b, op) {
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
                })
            }
            ExprKind::Match(m) => match self.eval_match(fn_name, m) {
                Return::Value(v) => v,
                Return::Tail(name, args, span) => self.call_type_fn(&name, args, span),
                _ => {
                    self.err(fn_name, "no match clause matched the type", caller_span);
                    Type::Any
                }
            },
            _ => {
                self.err(
                    fn_name,
                    "unsupported expression in type function body",
                    caller_span,
                );
                Type::Any
            }
        }
    }

    fn eval_path_to_type(&mut self, fn_name: &str, path: &Path, span: Span) -> Type {
        let Path::Base((key, _)) = path else {
            self.err(fn_name, "unsupported path in type function body", span);
            return Type::Any;
        };
        if let Some(t) = self.lookup_frame(key) {
            return t;
        }
        if let Some(t) = Type::from_keyword(key) {
            return t;
        }
        match self.viewer.lookup(key) {
            Some(symbol) => match symbol.symbol_type.clone() {
                SymbolType::TypeAlias(ty) => Some(self.eval_type(&ty)),
                SymbolType::ObjectClass(id) => self.objects.get(id).map(|o| Type::Object {
                    id,
                    name: o.name.clone(),
                    base: o.base,
                    args: Box::new([]),
                }),
                SymbolType::TypeFunction(_) => {
                    self.err(
                        fn_name,
                        format!("'{key}' is a type function, call it with ()"),
                        span,
                    );
                    None
                }
                _ => {
                    self.err(fn_name, format!("'{key}' is not a type"), span);
                    None
                }
            }
            .unwrap_or(Type::Any),
            None => {
                self.err(fn_name, format!("unknown type '{key}'"), span);
                Type::Any
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

fn literal_num(t: &Type) -> Option<f64> {
    match t {
        Type::Literal(cv) => match cv {
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
