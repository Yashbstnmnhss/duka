use super::AnalyzerData;
use crate::analyzer::{Visitor, VisitorMut};
use crate::parser::ast::{
    Block, Expr, ExprKind, FieldPattern, FuncBody, If, IfClause, Linq, LinqClause, Match,
    MatchClause, Path, PathSuffix, PatternArrayTerm, PatternOp, PatternTerm, Stmt, StmtKind,
};
use duka_shared::utils::ScopesViewer;
use duka_shared::{
    constants::csugar,
    errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, SourceInfo, Spanned, UnOp},
    value::{ConstValue, DukaFloat, DukaInt},
};
use std::{mem, vec};

macro_rules! checker {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*), $($visitor: item),+) => {
        pub struct $name<'a> {
            $($var_name : $var_type),*,
            source_info: SourceInfo,
            errors: Vec<DukaSpannedError>,
            #[allow(unused)]
            data: &'a AnalyzerData
        }
        impl<'a> $name<'a> {
            pub fn new(source_info: SourceInfo, data: &'a AnalyzerData) -> Self {
                Self {
                    $($var_name: $var_val),*,
                    errors: vec![],
                    source_info,
                    data
                }
            }
            fn error<const N: usize>(&mut self, kind: impl Into<DukaErrorKind>, span: Span, related: [(Box<str>, Span); N]) {
                self.errors.push(DukaSpannedError{
                    kind: kind.into(),
                    span,
                    source_info: self.source_info.clone(),
                    related: related.into()
                })
            }
        }
        impl Visitor for $name<'_> {
            $($visitor)+
            fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
                self.errors.iter().cloned().into_iter()
            }
        }
    };
}
macro_rules! transformer {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*), $($visitor: item),+) => {
        pub struct $name {
            $($var_name : $var_type),*
        }
        impl $name {
            pub fn new() -> Self {
                Self {
                    $($var_name: $var_val),*
                }
            }
        }
        impl VisitorMut for $name {
            $($visitor)+
        }
    };
}

macro_rules! adapting {
    (<- $input: expr) => {
        mem::take($input)
    };
    ($pattern: pat in $input: expr) => {
        let $pattern = mem::take($input) else {
            unreachable!()
        };
    };
    ($src: ident <- $val: expr) => {
        let _ = mem::replace($src, $val);
    };
    ($a: ident <-> $b: ident) => {
        mem::swap($a, $b);
    };
}

macro_rules! return_ {
    ($e: expr, $s: expr) => {
        Some(Box::new(Stmt(StmtKind::Return($e), $s)))
    };
}
macro_rules! path {
    () => {};
    (:{$e: expr}) => {
        PathSuffix::Colon($e)
    };
    ([$e: expr]) => {
        PathSuffix::Index($e)
    };
    (.{$e: expr}) => {
        PathSuffix::Dot($e)
    };
    ([$e: expr]$($right: tt)+) => {
        PathSuffix::Index($e) + path!($($right: tt)*)
    };
    (.{$e: expr}$($right: tt)+) => {
        PathSuffix::Dot($e) + path!($($right: tt)*)
    };
    (($e: expr)$($right: tt)*) => {
        boxed!(Path::Expr($e) + path!($($right)*))
    };
}
macro_rules! access {
    ($e: expr, $s: expr) => {
        Expr(ExprKind::Access($e), $s)
    };
}
macro_rules! literal {
    ($e: expr, $s: expr) => {
        Expr(ExprKind::Literal($e), $s)
    };
}
macro_rules! define {
    (local {$name: expr} = {$expr: expr}) => {
        StmtKind::Define([$name].into(), [$expr].into(), false)
    };
}
macro_rules! attrname {
    ($e: expr, $s: expr) => {
        ((name!($e, $s), [].into()), $s)
    };
}
macro_rules! name {
    ($e: expr, $s: expr) => {
        ($e.to_owned(), $s)
    };
}
macro_rules! assign {
    ({$target: expr} = {$expr: expr}, $s: expr) => {
        Stmt(StmtKind::Assign([$target].into(), [$expr].into()), $s)
    };
}
macro_rules! binary {
    ({$l:expr} $op:ident {$r:expr}, $s: expr) => {
        Expr(ExprKind::Binary($l, $r, BinOp::$op), $s)
    };
}
macro_rules! boxed {
    ($e: expr) => {
        Box::new($e)
    };
}

checker! {
    LoopChecker(loop_depth: usize = 0),
    fn visit_loop_stmt_block(&mut self, _: &StmtKind, enter: bool) {
        if enter {
            self.loop_depth += 1;
        } else {
            self.loop_depth = self.loop_depth.wrapping_sub(1);
        }
    },
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if matches!(stmt.0, StmtKind::Break | StmtKind::Continue) && self.loop_depth == 0 {
            self.error(DukaSemanticError::InvalidLoopFlowControl, stmt.1, [])
        }
    }
}

pub struct LabelChecker<'a> {
    pending_goto: Vec<Vec<Spanned<Box<str>>>>,
    viewer: ScopesViewer<'a, Box<str>, Span>,
    errors: Vec<DukaSpannedError>,
    source_info: SourceInfo,
}
impl<'a> LabelChecker<'a> {
    pub fn new(source_info: SourceInfo, data: &'a AnalyzerData) -> Self {
        Self {
            pending_goto: vec![vec![]],
            errors: vec![],
            source_info,
            viewer: ScopesViewer::new(&data.1.0),
        }
    }
}
impl Visitor for LabelChecker<'_> {
    fn visit_block(&mut self, enter: bool) {
        if enter {
            self.viewer.enter();
        } else {
            self.viewer.exit();
        }
    }
    fn visit_stmt(&mut self, stmt: &Stmt) {
        match stmt.0 {
            StmtKind::Goto(ref label) => {
                self.pending_goto
                    .last_mut()
                    .expect("WTF")
                    .push((label.as_str().into(), stmt.1));
            }
            _ => (),
        }
    }
    fn after(&mut self) {
        self.check_pending_goto();
    }
    fn report(&self) -> impl Iterator<Item = DukaSpannedError> {
        self.errors.clone().into_iter()
    }
}
impl LabelChecker<'_> {
    fn error<const N: usize>(
        &mut self,
        kind: impl Into<DukaErrorKind>,
        span: Span,
        related: [(Box<str>, Span); N],
    ) {
        self.errors.push(DukaSpannedError {
            kind: kind.into(),
            span,
            source_info: self.source_info.clone(),
            related: related.into(),
        })
    }
    fn check_pending_goto(&mut self) {
        if let Some(ps) = self.pending_goto.pop() {
            ps.into_iter().for_each(|(label, span)| {
                if self.viewer.lookup_label(&label).is_none() {
                    self.error(DukaSemanticError::InvisibleGotoLabel(label), span, []);
                }
            });
        }
    }
}

enum Bool {
    True,
    False,
}

checker! {
    VarArgChecker(
        marks: Vec<Bool> = vec![],
        places: Vec<Vec<Span>> = vec![],
        collected: Vec<(Span, Option<Span>)> = vec![]
    ),
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(expr.0, ExprKind::VarArg) {
            if matches!(self.marks.last(), Some(Bool::False))
                && let Some(cur) = self.places.last_mut() {
                cur.push(expr.1)
            }
        }
        else if matches!(expr.0, ExprKind::Function(..)) {
            if let Some(spans) = self.places.pop() {
                for span in spans {
                    self.collected.push((span, None));
                }
            }
        }
    },
    fn visit_stmt(&mut self, stmt: &Stmt) {
        // its content had been visited before this is invoked
        if let StmtKind::Function(ref p, ..) = stmt.0 {
            if let Some(spans) = self.places.pop() {
                let func_span = p.get_span();
                for span in spans {
                    self.collected.push((span, Some(func_span)));
                }
            }
        }
    },
    fn visit_func_block(&mut self, func: &FuncBody, enter: bool) {
        if enter {
            self.places.push(vec![]);
            self.marks.push(func.has_var_arg().then_some(Bool::True).unwrap_or(Bool::False));
        } else {
            self.marks.pop();
        };
    },
    fn after(&mut self) {
        self.check_invalid_var_arg();
    }
}

impl VarArgChecker<'_> {
    fn check_invalid_var_arg(&mut self) {
        for (at, func) in mem::take(&mut self.collected) {
            if let Some(func) = func {
                self.error(
                    DukaSemanticError::InvalidVarArg,
                    at,
                    [("in this function without ...".into(), func)],
                );
            } else {
                self.error(DukaSemanticError::InvalidVarArg, at, []);
            }
        }
    }
}

transformer! {
    ConstFoldTransformer(),
    fn visit_expr(&mut self, expr: &mut Expr) {
        match &mut expr.0 {
            ExprKind::Binary(l, r, op @ BinOp::Pipeline | op @ BinOp::PipelineL) => {
                if matches!(op, BinOp::PipelineL) {
                    adapting!(l <-> r);
                }
                match &mut r.0 {
                    ExprKind::Call(func, params) => {
                        let l = adapting!(<- l);
                        let func = adapting!(<- func);
                        let mut params = adapting!(<- params).into_vec();
                        params.push(*l);
                        expr.0 = ExprKind::Call(func, params.into());
                    },
                    ExprKind::Access(_) => {
                        let r = adapting!(<- r);
                        let l = adapting!(<- l);
                        expr.0 = ExprKind::Call(r, [*l].into());
                    }
                    _ => ()
                }
            },
            ExprKind::Binary(l, r, op) => {
                if let Some(new_expr) = Self::fold_binary(&mut l.0, &mut r.0, op) {
                    expr.0 = new_expr
                }
            },
            ExprKind::Unary(e, op) => {
                if let Some(new_expr) = Self::fold_unary(&e.0, op) {
                    expr.0 = new_expr
                }
            }
            _ => ()
        }
    }
}
impl ConstFoldTransformer {
    fn fold_unary(e: &ExprKind, op: &UnOp) -> Option<ExprKind> {
        fn do_unary(e: &ConstValue, op: &UnOp) -> Option<ConstValue> {
            match op {
                UnOp::BitNot => {
                    if let ConstValue::Int(i) = e {
                        Some(ConstValue::Int(!i))
                    } else {
                        None
                    }
                }
                UnOp::Minus => Some(match e {
                    ConstValue::Int(i) => ConstValue::Int(-i),
                    ConstValue::Float(f) => ConstValue::Float(-f),
                    _ => return None,
                }),
                UnOp::Not => Some(ConstValue::Bool(!e.eval_to_bool())),
                UnOp::Length => match e {
                    ConstValue::String(..) => Some(ConstValue::Int({
                        e.get_string().unwrap().len() as DukaInt
                    })),
                    ConstValue::ConstTable(table) if e.is_const() => {
                        Some(ConstValue::Int(table.len() as DukaInt))
                    }
                    _ => None,
                },
            }
        }

        match e {
            ExprKind::Literal(e) => do_unary(e, op).map(ExprKind::Literal),
            _ => None,
        }
    }

    fn fold_binary(l: &mut ExprKind, r: &mut ExprKind, op: &BinOp) -> Option<ExprKind> {
        fn do_binary(lv: &ConstValue, rv: &ConstValue, op: &BinOp) -> Option<ConstValue> {
            fn do_cmp(
                lv: &ConstValue,
                rv: &ConstValue,
                fi: fn(&DukaInt, &DukaInt) -> bool,
                ff: fn(&DukaFloat, &DukaFloat) -> bool,
            ) -> Option<ConstValue> {
                Some(ConstValue::Bool(match (lv, rv) {
                    (ConstValue::Int(i1), ConstValue::Int(i2)) => fi(i1, i2),
                    (ConstValue::Float(f1), ConstValue::Float(f2)) => ff(f1, f2),
                    (ConstValue::Int(i), ConstValue::Float(f)) => ff(&(*i as DukaFloat), f),
                    (ConstValue::Float(f), ConstValue::Int(i)) => ff(f, &(*i as DukaFloat)),
                    _ => return None,
                }))
            }
            fn do_arith(
                lv: &ConstValue,
                rv: &ConstValue,
                fi: fn(DukaInt, DukaInt) -> DukaInt,
                ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
            ) -> Option<ConstValue> {
                Some(match (lv, rv) {
                    (ConstValue::Int(i1), ConstValue::Int(i2)) => ConstValue::Int(fi(*i1, *i2)),
                    (ConstValue::Float(f1), ConstValue::Float(f2)) => {
                        ConstValue::Float(ff(*f1, *f2))
                    }
                    (ConstValue::Int(i), ConstValue::Float(f)) => {
                        ConstValue::Float(ff(*i as DukaFloat, *f))
                    }
                    (ConstValue::Float(f), ConstValue::Int(i)) => {
                        ConstValue::Float(ff(*f, *i as DukaFloat))
                    }
                    _ => return None,
                })
            }

            fn do_arith_i(
                lv: &ConstValue,
                rv: &ConstValue,
                fi: fn(DukaInt, DukaInt) -> DukaInt,
            ) -> Option<ConstValue> {
                let (a, b) = match (lv, rv) {
                    (ConstValue::Int(i1), ConstValue::Int(i2)) => (*i1, *i2),
                    (ConstValue::Float(f1), ConstValue::Float(f2)) => {
                        (*f1 as DukaInt, *f2 as DukaInt)
                    }
                    (ConstValue::Int(i), ConstValue::Float(f)) => (*i, *f as DukaInt),
                    (ConstValue::Float(f), ConstValue::Int(i)) => (*f as DukaInt, *i),
                    _ => return None,
                };
                Some(ConstValue::Int(fi(a, b)))
            }

            fn do_arith_f(
                lv: &ConstValue,
                rv: &ConstValue,
                ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
            ) -> Option<ConstValue> {
                let (a, b) = match (lv, rv) {
                    (ConstValue::Int(i1), ConstValue::Int(i2)) => {
                        (*i1 as DukaFloat, *i2 as DukaFloat)
                    }
                    (ConstValue::Float(f1), ConstValue::Float(f2)) => (*f1, *f2),
                    (ConstValue::Int(i), ConstValue::Float(f)) => (*i as DukaFloat, *f),
                    (ConstValue::Float(f), ConstValue::Int(i)) => (*f, *i as DukaFloat),
                    _ => return None,
                };
                Some(ConstValue::Float(ff(a, b)))
            }

            match op {
                BinOp::Add => do_arith(lv, rv, std::ops::Add::add, std::ops::Add::add),
                BinOp::Sub => do_arith(lv, rv, std::ops::Sub::sub, std::ops::Sub::sub),
                BinOp::Multiply => do_arith(lv, rv, std::ops::Mul::mul, std::ops::Mul::mul),
                BinOp::Divide => do_arith_f(lv, rv, std::ops::Div::div),
                BinOp::Pow => do_arith_f(lv, rv, |a, b| a.powf(b)),
                BinOp::IDivide => do_arith(lv, rv, std::ops::Div::div, std::ops::Div::div),
                BinOp::Mod => do_arith_i(lv, rv, |a, b| a % b),

                BinOp::BitAnd => do_arith_i(lv, rv, std::ops::BitAnd::bitand),
                BinOp::BitOr => do_arith_i(lv, rv, std::ops::BitOr::bitor),
                BinOp::BitXor => do_arith_i(lv, rv, std::ops::BitXor::bitxor),
                BinOp::ShiftL => do_arith_i(lv, rv, std::ops::Shl::shl),
                BinOp::ShiftR => do_arith_i(lv, rv, std::ops::Shr::shr),

                BinOp::Greater => {
                    do_cmp(lv, rv, std::cmp::PartialOrd::gt, std::cmp::PartialOrd::gt)
                }
                BinOp::GreaterEqual => {
                    do_cmp(lv, rv, std::cmp::PartialOrd::ge, std::cmp::PartialOrd::ge)
                }
                BinOp::Less => do_cmp(lv, rv, std::cmp::PartialOrd::lt, std::cmp::PartialOrd::lt),
                BinOp::LessEqual => {
                    do_cmp(lv, rv, std::cmp::PartialOrd::le, std::cmp::PartialOrd::le)
                }

                BinOp::Equal => Some(ConstValue::Bool(lv.eq(rv))),
                BinOp::NotEqual => Some(ConstValue::Bool(lv.ne(rv))),

                BinOp::And => Some(ConstValue::Bool(lv.eval_to_bool() && rv.eval_to_bool())),
                BinOp::Or => Some(ConstValue::Bool(lv.eval_to_bool() || rv.eval_to_bool())),
                BinOp::Xor => Some(ConstValue::Bool(lv.eval_to_bool() ^ rv.eval_to_bool())),

                BinOp::Concat => {
                    if let (Some(a), Some(b)) = (lv.get_string(), rv.get_string()) {
                        let mut result = Vec::with_capacity(a.len() + b.len());
                        result.extend_from_slice(a.as_bytes());
                        result.extend_from_slice(b.as_bytes());
                        Some(ConstValue::String(result.as_slice().into()))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        match (l, r) {
            (ExprKind::Literal(lv), ExprKind::Literal(rv)) => {
                do_binary(lv, rv, op).map(ExprKind::Literal)
            }
            _ => None,
        }
    }
}

transformer! {
    MeaninglessTransformer(),
    fn visit_expr(&mut self, expr: &mut Expr) {
        match expr.0 {
            ExprKind::If(ref mut if_) => {
                let target = adapting!(<- if_);
                let result = match self.adapt_if(*target) {
                    AdaptedIf::Empty => ExprKind::Empty,
                    AdaptedIf::Do(block) => ExprKind::Do(block),
                    AdaptedIf::If(if_) => ExprKind::If(Box::new(if_))
                };
                expr.0 = result
            },
            ExprKind::Do(ref v) if v.is_empty() => {
                expr.0 = ExprKind::Literal(ConstValue::Nil);
            }
            _ => ()
        }
    },
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        match stmt.0 {
            StmtKind::If(ref mut if_) => {
                let target = adapting!(<- if_);
                let result = match self.adapt_if(target) {
                    AdaptedIf::Empty => StmtKind::Empty,
                    AdaptedIf::Do(block) => StmtKind::Do(block.into()),
                    AdaptedIf::If(if_) => StmtKind::If(if_)
                };
                stmt.0 = result
            },
            StmtKind::While(ref cond, _) if matches!(**cond, Expr(ExprKind::Literal(ConstValue::Bool(false)), _)) => {
                stmt.0 = StmtKind::default()
            },
            StmtKind::Do(ref v) if v.is_empty() => {
                stmt.0 = StmtKind::Empty;
            },
            StmtKind::Assign(..) => {
                let StmtKind::Assign(left, right) = adapting!(<- &mut stmt.0) else {
                    unreachable!()
                };

                let l_len = left.len();
                let r_len = right.len();
                let mut iter_a = left.into_iter();
                let mut iter_b = right.into_iter();

                let mut left = Vec::with_capacity(l_len);
                let mut right = Vec::with_capacity(r_len);

                loop {
                    match (iter_a.next(), iter_b.next()) {
                        (Some(a), Some(b)) => {
                            if let Expr(ExprKind::Access(ref path2), _) = b && a == **path2 {
                                continue
                            }

                            left.push(a);
                            right.push(b);
                        }
                        (Some(a), None) => {
                            left.push(a);
                        },
                        (None, Some(b)) => {
                            right.push(b);
                        },
                        (None, None) => break,
                    }
                }

                stmt.0 = StmtKind::Assign(left.into(), right.into());
            },
            _ => ()
        }
    }
}

enum AdaptedIf {
    If(If),
    Do(Box<Block>),
    Empty,
}
impl MeaninglessTransformer {
    fn adapt_if(&self, target: If) -> AdaptedIf {
        fn adapt_if_inner(
            if_clause: IfClause,
            else_if_clauses: Box<[IfClause]>,
            else_clause: Option<Box<Block>>,
        ) -> AdaptedIf {
            enum AdaptedClause {
                Never,
                Always,
                Keep,
            }
            fn adapt_if_clause(clause: &IfClause) -> AdaptedClause {
                let IfClause(block, cond) = clause;
                match cond.0 {
                    ExprKind::Literal(ref cv) => cv
                        .eval_to_bool()
                        .then_some(if block.is_empty() {
                            AdaptedClause::Never
                        } else {
                            AdaptedClause::Always
                        })
                        .unwrap_or(AdaptedClause::Never),
                    _ => AdaptedClause::Keep,
                }
            }

            match adapt_if_clause(&if_clause) {
                AdaptedClause::Always => AdaptedIf::Do(if_clause.0),
                AdaptedClause::Never => {
                    if else_if_clauses.is_empty() {
                        else_clause
                            .map(|e| AdaptedIf::Do(e))
                            .unwrap_or(AdaptedIf::Empty)
                    } else {
                        let mut iter = else_if_clauses.into_iter();
                        let if_clause = iter.next().unwrap();
                        let else_if_clauses = iter.collect();
                        adapt_if_inner(if_clause, else_if_clauses, else_clause)
                    }
                }
                AdaptedClause::Keep => {
                    let mut new_else_if = vec![];
                    let mut has_always = false;

                    for else_if in else_if_clauses {
                        match adapt_if_clause(&else_if) {
                            AdaptedClause::Never => continue,
                            AdaptedClause::Always if has_always => continue,
                            AdaptedClause::Always => {
                                new_else_if.push(else_if);
                                has_always = true;
                            }
                            _ => new_else_if.push(else_if),
                        }
                    }

                    AdaptedIf::If(If(if_clause, new_else_if.into(), else_clause))
                }
            }
        }

        let If(if_clause, else_if_clauses, else_clause) = target;
        adapt_if_inner(if_clause, else_if_clauses, else_clause)
    }
}

// TODO:
transformer! {
    DesugarTransformer(),
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        if !stmt.0.is_sugar() {
        }
    },
    fn visit_expr(&mut self, expr: &mut Expr) {
        if !expr.0.is_sugar() {
            return
        }
        match &expr.0 {
            ExprKind::Linq(_) => {
                adapting!(Expr(ExprKind::Linq(linq), span) in expr);
                let new_ek = self.desugar_linq(linq, span);
                adapting!(expr <- Expr(new_ek, span));
            },
            ExprKind::Match(_) => {
                adapting!(Expr(ExprKind::Match(m), span) in expr);
                let r#if = self.desugar_match(m);
                adapting!(expr <- Expr(match r#if {
                    AdaptedIf::Do(b) => ExprKind::Do(b.into()),
                    AdaptedIf::Empty => ExprKind::Empty,
                    AdaptedIf::If(r#if) => ExprKind::If(r#if.into())
                }, span));
            },
            _ => ()
        }
    }
}
impl DesugarTransformer {
    fn desugar_linq(&self, linq: Linq, span: Span) -> ExprKind {
        let Linq(clauses, select) = linq;

        let target_name = attrname!(csugar::LINQ_TABLE, span);
        let target_def = span
            * define!(local { target_name.clone() } = { literal!(ConstValue::new_table(), span) });
        let index_name = attrname!(csugar::LINQ_INDEX, span);
        let index_def =
            span * define!(local { index_name.clone() } = { literal!(ConstValue::Int(0), span) });

        let mut iter = clauses.into_iter().rev();

        let inner = iter
            .next() // this wont happen, im sure if parser is working correctly
            .expect("NO, it must contain at lease one clause!");
        let init = span
            * make_stmt(
                inner,
                Block(
                    [
                        assign!(
                            {
                                Path::Base(target_name.0.0.clone())
                                    + PathSuffix::Index(boxed!(access!(
                                        boxed!(Path::Base(index_name.0.0.clone())),
                                        span
                                    )))
                            } = { *select },
                            span
                        ),
                        assign!(
                            { Path::Base(index_name.0.0.clone()) } = {
                                access!(boxed!(Path::Base(index_name.0.0)), span)
                                    + literal!(ConstValue::Int(1), span)
                            },
                            span
                        ),
                    ]
                    .into(),
                    None,
                ),
            );

        let body = iter.fold(init, |acc, clause| {
            span * make_stmt(clause, Block([acc].into(), None))
        });

        fn make_stmt(clause: LinqClause, block: Block) -> StmtKind {
            match clause {
                LinqClause::From(name, src) => {
                    StmtKind::ForGeneric([Path::Base(name)].into(), [*src].into(), Box::new(block))
                }
                LinqClause::Where(cond) => {
                    StmtKind::If(If(IfClause(Box::new(block), cond), Box::new([]), None))
                }
            }
        }

        ExprKind::Do(Box::new(Block(
            [target_def, index_def, body].into(),
            return_!(
                [access!(Box::new(Path::Base(target_name.0.0)), span)].into(),
                span
            ),
        )))
    }

    fn desugar_match(&self, r#match: Match) -> AdaptedIf {
        fn desugar_clause(target: Expr, clause: MatchClause) -> IfClause {
            let MatchClause((term, guard), block) = clause;

            fn desugar_term(target: Expr, term: PatternTerm) -> Expr {
                use PatternTerm::*;
                let span = target.1;
                Expr(
                    match term {
                        Constant(expr) => ExprKind::Binary(Box::new(target), expr, BinOp::Equal),
                        Bind(_) => ExprKind::Literal(ConstValue::Bool(true)), // deal it in block
                        Call(expr) => ExprKind::Call(expr, [target].into()),
                        Compare(op, expr) => ExprKind::Binary(Box::new(target), expr, op),
                        Table(fields) => {
                            let mut first_discord_many: Option<usize> = None;
                            let mut array_index: usize = 0;
                            let mut item_count: usize = 0;
                            let mut exprs = vec![
                                span * ExprKind::Call(
                                    boxed!(access!(
                                        Box::new(Path::Base((
                                            csugar::TYPE_IS_TABLE.to_owned(),
                                            span
                                        ))),
                                        span
                                    )),
                                    [target.clone()].into(),
                                ),
                            ];

                            let len = fields.len();
                            for field in fields {
                                match field {
                                    FieldPattern::Named((key, key_span), term) => {
                                        let target = access!(
                                            path!((boxed!(target.clone())).{(key, key_span)}),
                                            key_span
                                        );
                                        item_count += 1;
                                        exprs.push(desugar_term(target, term));
                                    }
                                    FieldPattern::Expr(key, term) => {
                                        let key_span = key.1;
                                        let target = access!(
                                            path!((boxed!(target.clone()))[boxed!(key)]),
                                            key_span
                                        );
                                        item_count += 1;
                                        exprs.push(desugar_term(target, term));
                                    }
                                    FieldPattern::Array(term) => {
                                        let target = access!(
                                            path!(
                                                (boxed!(target.clone()))[boxed!(if let Some(i) =
                                                    first_discord_many
                                                {
                                                    (span
                                                        * ExprKind::Unary(
                                                            boxed!(target.clone()),
                                                            UnOp::Length,
                                                        ))
                                                        - (span
                                                            * ExprKind::Literal(ConstValue::Int(
                                                                (len - i - array_index) as DukaInt,
                                                            )))
                                                } else {
                                                    span * ExprKind::Literal(ConstValue::Int(
                                                        array_index as DukaInt,
                                                    ))
                                                })]
                                            ),
                                            span
                                        );
                                        match term {
                                            PatternArrayTerm::Discard(n) => {
                                                array_index += n;
                                            }
                                            PatternArrayTerm::DiscardMany => {
                                                first_discord_many = Some(array_index);
                                            }
                                            PatternArrayTerm::Term(term) => {
                                                exprs.push(desugar_term(target, term));
                                                array_index += 1;
                                            }
                                        }
                                    }
                                }
                            }

                            let final_len = array_index + item_count;

                            exprs.push(binary!(
                                {boxed!(span * ExprKind::Unary(boxed!(target), UnOp::Length))}
                                GreaterEqual
                                {boxed!(
                                    span * ExprKind::Literal(ConstValue::Int(final_len as DukaInt))
                                )},
                                span
                            ));

                            // checked
                            return exprs.into_iter().reduce(|acc, item| acc & item).unwrap();
                        }
                        Compound(left, right, op) => ExprKind::Binary(
                            Box::new(desugar_term(target.clone(), *left)),
                            Box::new(desugar_term(target, *right)),
                            match op {
                                PatternOp::And => BinOp::And,
                                PatternOp::Or => BinOp::Or,
                                PatternOp::Xor => BinOp::Xor,
                            },
                        ),
                        Not(term) => {
                            ExprKind::Unary(Box::new(desugar_term(target, *term)), UnOp::Not)
                        }
                    },
                    span,
                )
            }

            let cond = desugar_term(target, term);

            IfClause(
                block,
                Box::new(if let Some(guard) = guard {
                    cond & *guard
                } else {
                    cond
                }),
            )
        }

        let Match(target, clauses, else_block) = r#match;

        let mut desugareds = clauses
            .into_iter()
            .map(|clause| desugar_clause(*target.clone(), clause));

        let Some(head) = desugareds.next() else {
            return else_block
                .map(|e| AdaptedIf::Do(e))
                .unwrap_or(AdaptedIf::Empty);
        };

        AdaptedIf::If(If(head, desugareds.collect(), else_block))
    }
}
