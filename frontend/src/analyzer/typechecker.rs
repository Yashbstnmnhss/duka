//! 静态类型检查(Complile-time)

use std::collections::HashMap;
use std::sync::Arc;

use duka_shared::{
    dtype::{FunctionType, ObjectId, Type},
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::{
    analyzer::{
        AnalyzerData, Visit, Visitor,
        objects::{MethodLink, ObjectMethod, ObjectType},
    },
    parser::ast::{DukaChunk, Expr, ExprKind, FuncBody, Param, Path, PathSuffix, StmtKind},
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
        mut data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        let mut ctx = TypeCheckerCtx::new(Arc::new(chunk.source_info.clone()), &data);
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
    links: Vec<MethodLink>,
    errors: Vec<DukaSpannedError>,
    backfills: Vec<(Span, Box<str>)>,
}

impl<'a> TypeCheckerCtx<'a> {
    fn new(source: Arc<SourceInfo>, data: &'a AnalyzerData) -> Self {
        Self {
            source,
            ret_stack: vec![None],
            types: vec![HashMap::new()],
            viewer: SymbolTableViewer::new(&data.1.symbols),
            objects: &data.1.objects,
            links: vec![],
            errors: vec![],
            backfills: vec![],
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
        self.backfills.push((span, ty.to_string().into_boxed_str()));
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

    fn resolve_type(&mut self, ty: &Type, at: Option<Span>) -> Type {
        match ty {
            Type::Named(name) => self.resolve_named(name, at),
            Type::Union(ts) => Type::Union(
                ts.iter()
                    .map(|t| self.resolve_type(t, at))
                    .collect::<Vec<_>>()
                    .into(),
            ),
            Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
                params: ft.params.iter().map(|t| self.resolve_type(t, at)).collect(),
                returns: ft
                    .returns
                    .iter()
                    .map(|t| self.resolve_type(t, at))
                    .collect(),
                var_arg: ft.var_arg,
                return_var_arg: ft.return_var_arg,
            })),
            other => other.clone(),
        }
    }

    fn resolve_named(&mut self, name: &str, at: Option<Span>) -> Type {
        let kind = match self.viewer.lookup(name) {
            Some(sym) => sym.symbol_type.clone(),
            None => {
                if let Some(span) = at {
                    self.err(DukaSemanticError::UnknownType(name.into()), span);
                }
                return Type::Named(name.into());
            }
        };
        if let SymbolType::ObjectClass(id) = kind {
            if let Some(obj) = self.objects.get(id) {
                return Type::Object {
                    id,
                    name: obj.name.clone(),
                    base: obj.base,
                };
            }
        }
        if let Some(span) = at {
            self.err(DukaSemanticError::UnknownType(name.into()), span);
        }
        Type::Named(name.into())
    }
}

fn fn_type(body: &FuncBody) -> Type {
    let FuncBody(params, ret, _) = body;
    Type::Function(Some(FunctionType {
        params: params
            .iter()
            .map(|p| match p {
                Param::Typed(_, t) => t.clone(),
                _ => Type::Any,
            })
            .collect(),
        var_arg: body.has_var_arg(),
        returns: ret.clone().into_iter().collect(),
        return_var_arg: false,
    }))
}

impl<'a> Visitor for TypeCheckerCtx<'a> {
    fn before(&mut self) {}

    fn visit_block(&mut self, enter: bool) {
        if enter {
            self.types.push(HashMap::new());
            self.viewer.enter();
        } else {
            self.viewer.exit();
            self.types.pop();
        }
    }

    fn visit_func_block(&mut self, block: &FuncBody, enter: bool) {
        if enter {
            let ret = match &block.1 {
                Some(t) => Some(self.resolve_type(t, None)),
                None => None,
            };
            self.ret_stack.push(ret);
            self.declare_params(block);
        } else {
            self.ret_stack.pop();
        }
    }

    fn visit_stmt(&mut self, stmt: &crate::parser::ast::Stmt) {
        match &stmt.0 {
            StmtKind::Define(names, exprs, _) => {
                for (idx, (((name, span), _attrs, ty), _)) in names.iter().enumerate() {
                    let actual = exprs.get(idx).map(|e| self.infer_expr(e));
                    let declared = ty.as_ref().map(|t| self.resolve_type(t, Some(*span)));
                    if let Some(declared) = &declared
                        && let Some(actual) = &actual
                        && !declared.accepts(actual)
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                declared.to_string(),
                                actual.to_string(),
                            ),
                            exprs[idx].1,
                        );
                    }
                    self.declare(name, *span, declared.or(actual).unwrap_or(Type::Any));
                }
            }
            StmtKind::Assign(targets, exprs) => {
                for (idx, target) in targets.iter().enumerate() {
                    let some_expr = exprs.get(idx);
                    if let Path::Base((name, span)) = target
                        && let Some(expr) = some_expr
                        && self.lookup_type(name).is_none()
                    {
                        let actual = self.infer_expr(expr);
                        self.declare(name, *span, actual);
                        continue;
                    }
                    if let Path::Base((name, _)) = target
                        && let Some(expr) = some_expr
                        && let Some(declared) = self.lookup_type(name)
                    {
                        let actual = self.infer_expr(expr);
                        if !declared.accepts(&actual) && actual != Type::Any {
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
                let ret = self.ret_stack.last().cloned().flatten();
                if let Some(ret) = ret {
                    for e in items {
                        let actual = self.infer_expr(e);
                        if !ret.accepts(&actual) && actual != Type::Any {
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
                    self.declare(name, *span, fn_type(body));
                }
            }
            StmtKind::Expr(expr) => {
                let _ = self.infer_expr(expr);
            }
            StmtKind::Call(callee, _) => {
                if let Expr(ExprKind::Access(path), span) = &**callee {
                    let _ = self.infer_call(path, *span);
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
}

impl TypeCheckerCtx<'_> {
    fn infer_expr(&mut self, Expr(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_kind(&mut self, kind: &ExprKind) -> Type {
        match kind {
            ExprKind::Literal(lit) => lit.type_of(),
            ExprKind::Table(_) => Type::Table,
            ExprKind::Function(body) => {
                let Type::Function(Some(ft)) = fn_type(body) else {
                    return Type::Any;
                };
                Type::Function(Some(FunctionType {
                    params: ft
                        .params
                        .iter()
                        .map(|t| self.resolve_type(t, None))
                        .collect(),
                    returns: ft
                        .returns
                        .iter()
                        .map(|t| self.resolve_type(t, None))
                        .collect(),
                    var_arg: ft.var_arg,
                    return_var_arg: ft.return_var_arg,
                }))
            }
            ExprKind::Access(path) => self.infer_access(path),
            ExprKind::Call(callee, _) => match &**callee {
                Expr(ExprKind::Access(path), span) => self.infer_call(path, *span),
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
        }
    }

    fn return_type_of(&self, sig: &FunctionType) -> Type {
        match sig.returns.len() {
            0 => Type::Any,
            1 => sig.returns[0].clone(),
            _ => Type::Union(sig.returns.to_vec().into()),
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
                            .map(|m| m.ty.clone())
                            .unwrap_or(Type::Any)
                    }
                    None => Type::Any,
                }
            }
            _ => match path {
                Path::Base((name, _)) => self.lookup_type(name.as_str()).unwrap_or(Type::Any),
                _ => Type::Any,
            },
        }
    }

    fn infer_call(&mut self, path: &Path, call_span: Span) -> Type {
        match path {
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
                if name == "new" {
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
        // 变量/调用可能被 metatable 覆写 BAnd,不应静态报错
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
        // `int!` 非空:赋 nil 报错
        let errors = check_with("local a: int! = nil", false);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with("local a: int! = 5", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn union_bang_keeps_other_nil() {
        // int | string!: !仅管string, 但 int 仍可空 整体可空
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
            errors.iter().any(|e| matches!(
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
