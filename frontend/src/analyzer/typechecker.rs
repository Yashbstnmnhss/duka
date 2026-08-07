//! 静态类型检查

use std::collections::HashMap;
use std::sync::Arc;

use duka_shared::{
    dtype::FunctionType,
    dtype::Type,
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::{
    analyzer::{AnalyzerData, Visit, Visitor},
    parser::ast::{DukaChunk, Expr, ExprKind, FuncBody, Param, Path, StmtKind},
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
        let (errors, backfills) = ctx.finish();
        for (span, ty) in backfills {
            data.1 .0.set_type_at_span(span, ty);
        }
        (data, errors)
    }
}

struct TypeCheckerCtx<'a> {
    source: Arc<SourceInfo>,
    /// 当前函数的返回类型
    ret_stack: Vec<Option<Type>>,
    /// 每个 block 一层的变量/类型名表
    types: Vec<HashMap<Box<str>, Type>>,
    viewer: SymbolTableViewer<'a>,
    errors: Vec<DukaSpannedError>,
    /// 声明的 (span, 类型 string),分析后回填到 SymbolTable
    backfills: Vec<(Span, Box<str>)>,
}

impl<'a> TypeCheckerCtx<'a> {
    fn new(source: Arc<SourceInfo>, data: &'a AnalyzerData) -> Self {
        Self {
            source,
            ret_stack: vec![None],
            types: vec![HashMap::new()],
            viewer: SymbolTableViewer::new(&data.1.0),
            errors: vec![],
            backfills: vec![],
        }
    }

    fn finish(self) -> (impl Iterator<Item = DukaSpannedError> + use<>, Vec<(Span, Box<str>)>) {
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
                return Some(type_of(cv));
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
                Param::Typed((name, span), ty) => self.declare(name, *span, ty.clone()),
                Param::Name((name, span)) => self.declare(name, *span, Type::Any),
                Param::Var(_) => {}
            }
        }
    }
}

fn type_of(cv: &ConstValue) -> Type {
    match cv {
        ConstValue::Nil => Type::Nil,
        ConstValue::Bool(_) => Type::Bool,
        ConstValue::Int(_) => Type::Int,
        ConstValue::Float(_) => Type::Float,
        ConstValue::String(_) => Type::String,
        ConstValue::ConstTable(_) => Type::Table,
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
            self.ret_stack.push(block.1.clone());
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
                    if let Some(ty) = ty
                        && let Some(actual) = &actual
                        && !ty.accepts(actual)
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                ty.to_string(),
                                actual.to_string(),
                            ),
                            exprs[idx].1,
                        );
                    }
                    self.declare(name, *span, ty.clone().or(actual.clone()).unwrap_or(Type::Any));
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
            _ => {}
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
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
    fn infer_expr(&self, Expr(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_kind(&self, kind: &ExprKind) -> Type {
        match kind {
            ExprKind::Literal(lit) => type_of(lit),
            ExprKind::Table(_) => Type::Table,
            ExprKind::Function(body) => fn_type(body),
            ExprKind::Access(path) => match &**path {
                Path::Base((name, _)) => self.lookup_type(name).unwrap_or(Type::Any),
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
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser};

    use crate::{
        analyzer::ScopeAnalyzer, analyzer::TypeChecker, lexer::LexerWithMacro, parser::Parser,
    };

    fn check(source: &str) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(Cursor::new(source), Some("test".into()));
        let stream = lexer.tokenize().unwrap();
        let chunk =
            Parser::parse(stream, duka_shared::config::DukaParserConfig::default()).unwrap();
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
        let lexer = LexerWithMacro::new(Cursor::new(source), Some("test".into()));
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
        // 无注解变量由字面量推断,int 重赋值字符串应报错
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
        // 常量参与推断,重赋值应被判定
        let errors = check("local const N = 3");
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
        let errors = check("local cb: fn(int, ...) -> (int, ...) = function(a, ...) return 1, ... end");
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

    #[test]
    fn display_fn_vararg_returns_terminates() {
        // regression: Display used from_fn + then_some, an infinite iterator
        // when var_arg / return_var_arg is true (only hit on the error path).
        let errors = check("global b: fn(...) -> ... = 1");
        assert!(is_error(&errors), "{:?}", errors);
    }

    fn check_with(source: &str, nonnilable: bool) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(Cursor::new(source), Some("test".into()));
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
        // int | string!: ! 吃紧邻 string,但 int 仍可空 → 整体可空
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
}
