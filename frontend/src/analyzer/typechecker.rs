//! 静态类型检查

use std::sync::Arc;

use duka_shared::{
    dtype::Type,
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    value::ConstValue,
};

use crate::{
    analyzer::{AnalyzerData, Visit},
    parser::ast::{
        DukaChunk, Expr, ExprKind, Field, FuncBody, If, LinqClause, Match, MatchClause, Path,
        PathSuffix, PatternTerm, Stmt, StmtKind,
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
        let mut errors = Vec::new();
        if data.0.type_annotations {
            let mut ctx = TypeCheckerCtx::new(Arc::new(chunk.source_info.clone()));
            ctx.check_block(&chunk.block, &mut errors);
        }
        (data, errors.into_iter())
    }
}

struct TypeCheckerCtx {
    source: Arc<SourceInfo>,
    /// 当前函数的返回类型
    ret_stack: Vec<Option<Type>>,
}

impl TypeCheckerCtx {
    fn new(source: Arc<SourceInfo>) -> Self {
        Self {
            source,
            ret_stack: vec![None],
        }
    }

    fn err(&self, v: DukaSemanticError, span: Span, into: &mut Vec<DukaSpannedError>) {
        into.push(DukaSpannedError {
            kind: v.into(),
            span,
            related: [].into(),
            source_info: self.source.clone(),
        });
    }

    fn check_block(
        &mut self,
        block: &crate::parser::ast::Block,
        errors: &mut Vec<DukaSpannedError>,
    ) {
        for stmt in block.0.iter() {
            self.check_stmt(stmt, errors);
        }
        if let Some(ret) = &block.1 {
            self.check_stmt(ret, errors);
        }
    }

    fn check_func_body(&mut self, body: &FuncBody, errors: &mut Vec<DukaSpannedError>) {
        let FuncBody(_params, ret, block) = body;
        self.ret_stack.push(*ret);
        self.check_block(block, errors);
        self.ret_stack.pop();
    }

    fn check_stmt(&mut self, Stmt(kind, _span): &Stmt, errors: &mut Vec<DukaSpannedError>) {
        match kind {
            StmtKind::Define(names, exprs, _) => {
                let mut exprs = exprs.iter();
                for (((_, _), _, ty), _) in names.iter() {
                    if let Some(ty) = ty
                        && let Some(Expr(ek, exp_span)) = exprs.next()
                    {
                        let actual = self.infer_expr_kind(ek);
                        if !ty.accepts(&actual) {
                            self.err(
                                DukaSemanticError::TypeMismatchEqual(ty.name(), actual.name()),
                                *exp_span,
                                errors,
                            );
                        }
                    }
                }
                for e in exprs {
                    self.check_expr(e, errors);
                }
            }
            StmtKind::Return(items) => {
                let ret = self.ret_stack.last().copied().flatten();
                for e in items {
                    if let Some(ret) = ret {
                        let actual = self.infer_expr(e);
                        if !ret.accepts(&actual) {
                            self.err(
                                DukaSemanticError::TypeMismatchReturn(ret.name(), actual.name()),
                                e.1,
                                errors,
                            );
                        }
                    }
                    self.check_expr(e, errors);
                }
            }
            StmtKind::Function(_, _, body, _) => self.check_func_body(body, errors),
            StmtKind::Expr(e) => self.check_expr(e, errors),
            StmtKind::Call(callee, args) => {
                self.check_expr(callee, errors);
                for a in args {
                    self.check_expr(a, errors);
                }
            }
            StmtKind::If(if_) => {
                self.check_expr(&if_.0.1, errors);
                self.check_block(&if_.0.0, errors);
                for clause in if_.1.iter() {
                    self.check_expr(&clause.1, errors);
                    self.check_block(&clause.0, errors);
                }
                if let Some(else_) = &if_.2 {
                    self.check_block(else_, errors);
                }
            }
            StmtKind::ForNumeric(_, start, stop, step, body) => {
                self.check_expr(start, errors);
                self.check_expr(stop, errors);
                if let Some(step) = step {
                    self.check_expr(step, errors);
                }
                self.check_block(body, errors);
            }
            StmtKind::ForGeneric(names, iterables, body) => {
                for e in iterables {
                    self.check_expr(e, errors);
                }
                let _ = names;
                self.check_block(body, errors);
            }
            StmtKind::While(cond, body) => {
                self.check_expr(cond, errors);
                self.check_block(body, errors);
            }
            StmtKind::Do(body) => self.check_block(body, errors),
            StmtKind::Assign(targets, exprs) => {
                for t in targets {
                    self.check_path(t, errors);
                }
                for e in exprs {
                    self.check_expr(e, errors);
                }
            }
            StmtKind::Match(m) => self.check_match(m, errors),
            StmtKind::Object(obj) => {
                for (_, _, body) in obj.methods.iter() {
                    self.check_func_body(body, errors);
                }
                for (_, _, body) in obj.static_methods.iter() {
                    self.check_func_body(body, errors);
                }
            }
            StmtKind::Export(inner) => self.check_stmt(inner, errors),
            _ => {}
        }
    }

    fn check_match(&mut self, m: &Match, errors: &mut Vec<DukaSpannedError>) {
        self.check_expr(&m.0, errors);
        for MatchClause(pat, body) in m.1.iter() {
            self.check_pattern(&pat.0, errors);
            if let Some(guard) = &pat.1 {
                self.check_expr(guard, errors);
            }
            self.check_block(body, errors);
        }
        if let Some(else_) = &m.2 {
            self.check_block(else_, errors);
        }
    }

    fn check_pattern(&mut self, pat: &PatternTerm, errors: &mut Vec<DukaSpannedError>) {
        match pat {
            PatternTerm::Constant(e) => self.check_expr(e, errors),
            PatternTerm::Call(e) => self.check_expr(e, errors),
            PatternTerm::Compare(_, e) => self.check_expr(e, errors),
            PatternTerm::Table(fields) => {
                for f in fields.iter() {
                    match f {
                        crate::parser::ast::FieldPattern::Array(t) => match t {
                            crate::parser::ast::PatternArrayTerm::Term(t) => {
                                self.check_pattern(t, errors)
                            }
                            _ => {}
                        },
                        crate::parser::ast::FieldPattern::Named(_, t) => {
                            self.check_pattern(t, errors)
                        }
                        crate::parser::ast::FieldPattern::Expr(e, t) => {
                            self.check_expr(e, errors);
                            self.check_pattern(t, errors);
                        }
                    }
                }
            }
            PatternTerm::Compound(a, b, _) => {
                self.check_pattern(a, errors);
                self.check_pattern(b, errors);
            }
            PatternTerm::Not(t) => self.check_pattern(t, errors),
            PatternTerm::Bind(_) => {}
        }
    }

    fn check_expr(&mut self, Expr(kind, _): &Expr, errors: &mut Vec<DukaSpannedError>) {
        match kind {
            ExprKind::Do(block) => self.check_block(block, errors),
            ExprKind::Function(body) => self.check_func_body(body, errors),
            ExprKind::Access(path) => self.check_path(path, errors),
            ExprKind::Call(callee, args) => {
                self.check_expr(callee, errors);
                for a in args {
                    self.check_expr(a, errors);
                }
            }
            ExprKind::Unary(e, op) => {
                if let UnOp::BitNot = op
                    && let Expr(ExprKind::Literal(ConstValue::Float(_)), span) = &**e
                {
                    self.err(
                        DukaSemanticError::TypeMismatchEqual(Type::Int.name(), Type::Float.name()),
                        *span,
                        errors,
                    );
                }
                self.check_expr(e, errors);
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
                                        Type::Int.name(),
                                        Type::Float.name(),
                                    ),
                                    operand.1,
                                    errors,
                                );
                            }
                        }
                    }
                }
                self.check_expr(a, errors);
                self.check_expr(b, errors);
            }
            ExprKind::If(if_) => self.check_if(if_, errors),
            ExprKind::Linq(linq) => {
                for clause in linq.0.iter() {
                    match clause {
                        LinqClause::Where(e) => self.check_expr(e, errors),
                        LinqClause::From(_, e) => self.check_expr(e, errors),
                    }
                }
                self.check_expr(&linq.1, errors);
            }
            ExprKind::Match(m) => self.check_match(m, errors),
            ExprKind::Table(fields) => {
                for f in fields.iter() {
                    match f {
                        Field::Value(e) | Field::KeyValue(e, _) => self.check_expr(e, errors),
                        Field::NameValue(_, e) => self.check_expr(e, errors),
                    }
                }
            }
            ExprKind::SysCall(_) | ExprKind::VarArg | ExprKind::Literal(_) | ExprKind::Empty => {}
        }
    }

    fn check_if(&mut self, if_: &If, errors: &mut Vec<DukaSpannedError>) {
        self.check_expr(&if_.0.1, errors);
        self.check_block(&if_.0.0, errors);
        for clause in if_.1.iter() {
            self.check_expr(&clause.1, errors);
            self.check_block(&clause.0, errors);
        }
        if let Some(else_) = &if_.2 {
            self.check_block(else_, errors);
        }
    }

    fn check_path(&mut self, path: &Path, errors: &mut Vec<DukaSpannedError>) {
        match path {
            Path::Expr(e) => self.check_expr(e, errors),
            Path::Base(_) => {}
            Path::Chain(p, s) => {
                self.check_path(p, errors);
                if let PathSuffix::Index(e) = s {
                    self.check_expr(e, errors);
                }
            }
        }
    }

    /// fallback `any`
    fn infer_expr(&mut self, Expr(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_kind(&mut self, kind: &ExprKind) -> Type {
        match kind {
            ExprKind::Literal(lit) => match lit {
                ConstValue::Nil => Type::Nil,
                ConstValue::Bool(_) => Type::Bool,
                ConstValue::Int(_) => Type::Int,
                ConstValue::Float(_) => Type::Float,
                ConstValue::String(_) => Type::String,
                ConstValue::ConstTable(_) => Type::Table,
            },
            ExprKind::Table(_) => Type::Table,
            ExprKind::Function(_) => Type::Function,
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
                        (Type::Num, Type::Float | Type::Int)
                        | (Type::Float | Type::Int, Type::Num) => Type::Num,
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

    use duka_shared::errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError};

    #[test]
    fn accepts_int_for_num() {
        let errors = check("local n: num = 1");
        assert!(!is_error(&errors), "{:?}", errors);
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
}
