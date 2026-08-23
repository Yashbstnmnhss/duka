use super::AnalyzerData;
use crate::analyzer::{VisitMut, Visitor, VisitorMut};
use crate::parser::ast::{
    Block, DukaChunk, Expr, ExprKind, Field, FieldPattern, FuncBody, If, IfClause, Linq,
    LinqClause, Match, MatchClause, Name, ObjectDef, ObjectProperty, Param, Path, PathSuffix,
    PatternArrayTerm, PatternOp, PatternTerm, Stmt, StmtKind, TypeDescriptor, get_attr,
};
use duka_shared::constants::{MetaMethod, catt};
use duka_shared::dtype::Type;
use duka_shared::utils::SymbolTableViewer;
use duka_shared::{
    constants::{cgen, cpar, csugar, ctype},
    errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, SourceInfo, Spanned, UnOp},
    value::{ConstValue, DukaFloat, DukaInt},
};
use std::sync::Arc;
use std::{mem, vec};

macro_rules! checker {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*), $($visitor: item),+) => {
        pub struct $name<'a> {
            $($var_name : $var_type),*,
            source_info: Arc<SourceInfo>,
            errors: Vec<DukaSpannedError>,
            #[allow(unused)]
            data: &'a AnalyzerData
        }
        impl<'a> $name<'a> {
            pub fn new(source_info: impl Into<Arc<SourceInfo>>, data: &'a AnalyzerData) -> Self {
                Self {
                    $($var_name: $var_val),*,
                    errors: vec![],
                    source_info: source_info.into(),
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
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
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
        ((name!($e, $s), [].into(), None), $s)
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
    viewer: SymbolTableViewer<'a>,
    errors: Vec<DukaSpannedError>,
    source_info: Arc<SourceInfo>,
}
impl<'a> LabelChecker<'a> {
    pub fn new(source_info: impl Into<Arc<SourceInfo>>, data: &'a AnalyzerData) -> Self {
        Self {
            pending_goto: vec![vec![]],
            errors: vec![],
            source_info: source_info.into(),
            viewer: SymbolTableViewer::new(&data.1.symbols),
        }
    }
}
impl Visitor for LabelChecker<'_> {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let StmtKind::Goto(ref label) = stmt.0 {
            self.pending_goto
                .last_mut()
                .expect("WTF")
                .push((label.as_str().into(), stmt.1));
        }
    }
    fn after(&mut self) {
        self.check_pending_goto();
    }
    fn visit_func_block(&mut self, _block: &FuncBody, enter: bool) {
        if enter {
            self.pending_goto.push(vec![]);
        } else {
            self.check_pending_goto();
        }
    }
    fn visit_block(&mut self, enter: bool) {
        if enter {
            self.viewer.enter();
        } else {
            self.viewer.exit();
        }
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
                if matches!(op, BinOp::Pipeline) {
                    // `data |> f(args)` -> `f(data, args)`：数据前插
                    let ExprKind::Call(func, params) = &mut r.0 else {
                        let r = adapting!(<- r);
                        let l = adapting!(<- l);
                        expr.0 = ExprKind::Call(r, [*l].into());
                        return;
                    };
                    let l = adapting!(<- l);
                    let func = adapting!(<- func);
                    let mut params = adapting!(<- params).into_vec();
                    params.insert(0, *l);
                    expr.0 = ExprKind::Call(func, params.into());
                } else {
                    // `f(args) <| data` -> `f(args, data)`：数据后追加
                    let ExprKind::Call(func, params) = &mut l.0 else {
                        let r = adapting!(<- r);
                        let l = adapting!(<- l);
                        expr.0 = ExprKind::Call(l, [*r].into());
                        return;
                    };
                    let r = adapting!(<- r);
                    let func = adapting!(<- func);
                    let mut params = adapting!(<- params).into_vec();
                    params.push(*r);
                    expr.0 = ExprKind::Call(func, params.into());
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
                    // ConstValue::ConstTable(table) if e.is_const() => {
                    //     Some(ConstValue::Int(table.len() as DukaInt))
                    // }
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
                BinOp::Add => do_arith(lv, rv, DukaInt::wrapping_add, std::ops::Add::add),
                BinOp::Sub => do_arith(lv, rv, DukaInt::wrapping_sub, std::ops::Sub::sub),
                BinOp::Multiply => do_arith(lv, rv, DukaInt::wrapping_mul, std::ops::Mul::mul),
                BinOp::Divide => do_arith_f(lv, rv, std::ops::Div::div),
                BinOp::Pow => do_arith_f(lv, rv, |a, b| a.powf(b)),
                BinOp::IDivide => do_arith(
                    lv,
                    rv,
                    |a, b| {
                        // floor 除:向负无穷取整
                        let mut r = a / b;
                        if (a % b) != 0 && (a < 0) != (b < 0) {
                            r -= 1;
                        }
                        r
                    },
                    |a, b| (a / b).floor(),
                ),
                BinOp::Mod => do_arith(
                    lv,
                    rv,
                    |a, b| {
                        // floor 取模:满足 a == (a//b)*b + (a%b)
                        let mut r = a / b;
                        if (a % b) != 0 && (a < 0) != (b < 0) {
                            r -= 1;
                        }
                        a - r * b
                    },
                    |a, b| a - (a / b).floor() * b,
                ),

                BinOp::BitAnd => do_arith_i(lv, rv, std::ops::BitAnd::bitand),
                BinOp::BitOr => do_arith_i(lv, rv, std::ops::BitOr::bitor),
                BinOp::BitXor => do_arith_i(lv, rv, std::ops::BitXor::bitxor),
                BinOp::ShiftL => do_arith_i(lv, rv, std::ops::Shl::shl),
                BinOp::ShiftR => do_arith_i(lv, rv, std::ops::Shr::shr),

                BinOp::Greater => do_cmp(lv, rv, PartialOrd::gt, PartialOrd::gt),
                BinOp::GreaterEqual => do_cmp(lv, rv, PartialOrd::ge, PartialOrd::ge),
                BinOp::Less => do_cmp(lv, rv, PartialOrd::lt, PartialOrd::lt),
                BinOp::LessEqual => do_cmp(lv, rv, PartialOrd::le, PartialOrd::le),

                BinOp::Equal => {
                    let eq = match (lv, rv) {
                        (ConstValue::Int(i1), ConstValue::Int(i2)) => i1 == i2,
                        (ConstValue::Float(f1), ConstValue::Float(f2)) => f1 == f2,
                        (ConstValue::Int(i), ConstValue::Float(f)) => (*i as DukaFloat) == *f,
                        (ConstValue::Float(f), ConstValue::Int(i)) => *f == (*i as DukaFloat),
                        _ => lv.eq(rv),
                    };
                    Some(ConstValue::Bool(eq))
                }
                BinOp::NotEqual => {
                    let eq = match (lv, rv) {
                        (ConstValue::Int(i1), ConstValue::Int(i2)) => i1 == i2,
                        (ConstValue::Float(f1), ConstValue::Float(f2)) => f1 == f2,
                        (ConstValue::Int(i), ConstValue::Float(f)) => (*i as DukaFloat) == *f,
                        (ConstValue::Float(f), ConstValue::Int(i)) => *f == (*i as DukaFloat),
                        _ => lv.eq(rv),
                    };
                    Some(ConstValue::Bool(!eq))
                }

                BinOp::And => Some(if lv.eval_to_bool() {
                    rv.clone()
                } else {
                    lv.clone()
                }),
                BinOp::Or => Some(if lv.eval_to_bool() {
                    lv.clone()
                } else {
                    rv.clone()
                }),
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
                    AdaptedIf::If(if_) => ExprKind::If(Box::new(if_)),
                    _ => unimplemented!()
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
                    AdaptedIf::If(if_) => StmtKind::If(if_),
                    _ => unimplemented!()
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
    InsertStmts(Stmt, Stmt),
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

transformer! {
    SuperReplacer(super_name: Option<String> = None),
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        if let StmtKind::Call(callee, params) = &mut stmt.0 {
            if let ExprKind::Access(path) = &callee.0
            && let Some(name) = self.is_super_colon_call(path) {
                if let Some(sn) = self.super_name.clone() {
                    *callee = boxed!(
                        callee.1 * ExprKind::Access(boxed!(
                            Path::Base((sn, callee.1)) + PathSuffix::Dot(name)
                        ))
                    );
                    let mut vec = vec![callee.1 * ExprKind::Access(boxed!(
                        Path::Base((cgen::SELF.to_owned(), callee.1))
                    ))];
                    vec.extend(mem::take(params));
                    *params = vec.into_boxed_slice();
                }
            }
        }
    },
    fn visit_expr(&mut self, expr: &mut Expr) {
        match &mut expr.0 {
            ExprKind::Call(callee, params) => {
                if let ExprKind::Access(path) = &callee.0
                && let Some(name) = self.is_super_colon_call(path) {
                    if let Some(sn) = self.super_name.clone() {
                        *callee = boxed!(
                            callee.1 * ExprKind::Access(boxed!(
                                Path::Base((sn, callee.1)) + PathSuffix::Dot(name)
                            ))
                        );
                        let mut vec = vec![callee.1 * ExprKind::Access(boxed!(
                            Path::Base((cgen::SELF.to_owned(), callee.1))
                        ))];
                        vec.extend(mem::take(params));
                        *params = vec.into_boxed_slice();
                    }
                }
            }
            ExprKind::Access(path) => {
                if self.is_super_colon_call(path).is_none() {
                    if let Some(sn) = self.super_name.clone() {
                        rewrite_super_base(path, &sn);
                    }
                }
            }
            _ => {}
        }
    }
}
impl SuperReplacer {
    fn is_super_colon_call(&self, path: &Path) -> Option<Name> {
        let Path::Chain(a, b) = path else { return None };
        match (&**a, b) {
            (Path::Base((c, _)), PathSuffix::Colon(name)) if c == cgen::SUPER => Some(name.clone()),
            _ => None,
        }
    }
}

fn rewrite_super_base(path: &mut Path, sn: &str) {
    match path {
        Path::Base((name, _)) if name == cgen::SUPER => *name = sn.to_owned(),
        Path::Chain(p, _) => rewrite_super_base(p, sn),
        _ => {}
    }
}

transformer! {
    DesugarTransformer(),
    fn visit_block(&mut self, block: &mut Block) {
        let stmts = std::mem::take(&mut block.0);
        let mut res = vec![];
        for mut b in stmts {
            b.visit_mut(self);
            match &b.0 {
                StmtKind::Match(_) => {
                    adapting!(Stmt(StmtKind::Match(m), span) in &mut b);
                    let AdaptedIf::InsertStmts(mut a, mut b) = self.desugar_match(m, false) else {
                        unreachable!()
                    };
                    a.1 = span;
                    b.1 = span;
                    res.push(a);
                    res.push(b);
                }
                _ => res.push(b),
            };
        }
        block.0 = res.into();
        block.1.visit_mut(self);
    },
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        if !stmt.0.is_sugar() {
            return
        }
        match &stmt.0 {
            StmtKind::Object(_) => {
                adapting!(Stmt(StmtKind::Object(od), span) in stmt);
                let new_ek = self.desugar_object(*od, span);
                adapting!(stmt <- Stmt(new_ek, span));
            },
            _ => ()
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
                let r#if = self.desugar_match(m, true);
                adapting!(expr <- Expr(match r#if {
                    AdaptedIf::Do(b) => ExprKind::Do(b.into()),
                    AdaptedIf::Empty => ExprKind::Empty,
                    AdaptedIf::If(r#if) => ExprKind::If(r#if.into()),
                    _ => unimplemented!(),
                }, span));
            },
            _ => ()
        }
    }
}

// Convert type to `typeof() == ...`
fn type_to_checker(ty: Type, target: Expr) -> ExprKind {
    let span = target.1;
    fn type_name_eq(target: Expr, name: &str) -> ExprKind {
        let span = target.1;
        ExprKind::Binary(
            boxed!(
                span * ExprKind::Call(
                    boxed!(access!(
                        boxed!(Path::Base((ctype::TYPEOF.to_owned(), span))),
                        span
                    )),
                    [target].into(),
                )
            ),
            boxed!(literal!(ConstValue::String(name.as_bytes().into()), span)),
            BinOp::Equal,
        )
    }
    match ty {
        Type::Array(_) => type_name_eq(target, ctype::ARR),
        Type::Never => ExprKind::Literal(ConstValue::Bool(false)),
        Type::Any => ExprKind::Literal(ConstValue::Bool(true)),
        Type::Union(u) => {
            let mut iter = u.into_vec().into_iter();
            let Some(mut acc) = iter.next().map(|t| type_to_checker(t, target.clone())) else {
                return ExprKind::Literal(ConstValue::Bool(false));
            };
            for t in iter {
                acc = (span * acc | span * type_to_checker(t, target.clone())).0;
            }
            acc
        }
        Type::Nil => type_name_eq(target, ctype::NIL),
        Type::Bool => type_name_eq(target, ctype::BOO),
        Type::Int => type_name_eq(target, ctype::INT),
        Type::Float => ExprKind::Binary(
            boxed!(span * type_name_eq(target.clone(), ctype::FLO)),
            boxed!(span * type_name_eq(target, ctype::INT)),
            BinOp::Or,
        ),
        Type::String => type_name_eq(target, ctype::STR),
        Type::Table(..) | Type::Object { .. } => type_name_eq(target, ctype::TAB),
        Type::Function(_) => type_name_eq(target, ctype::FUN),
        // 以下类型均不支持具体值比较
        Type::Param(_) | Type::TypeTable(_) | Type::TypeTuple(_) => {
            ExprKind::Literal(ConstValue::Bool(true))
        }
        Type::Literal(lv) => ExprKind::Binary(
            //字面量类型则相当于与常量比较
            boxed!(target.clone()),
            boxed!(span * ExprKind::Literal(lv.clone())),
            BinOp::Equal,
        ),
    }
}

impl DesugarTransformer {
    fn desugar_linq(&self, linq: Linq, span: Span) -> ExprKind {
        let Linq(clauses, select) = linq;

        let target_name = attrname!(csugar::LINQ_TABLE, span);
        let target_def = span
            * define!(local { target_name.clone() } = { Expr(ExprKind::Table([].into()), span) });
        let index_name = attrname!(csugar::LINQ_INDEX, span);
        let index_def =
            span * define!(local { index_name.clone() } = { literal!(ConstValue::Int(1), span) });

        let mut iter = clauses.into_iter().rev();

        let inner = iter
            .next() // this won't happen, im sure if parser is working correctly
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
                span,
            );

        let body = iter.fold(init, |acc, clause| {
            span * make_stmt(clause, Block([acc].into(), None), span)
        });

        fn make_stmt(clause: LinqClause, block: Block, span: Span) -> StmtKind {
            match clause {
                LinqClause::From(name, src) => {
                    // `from x in arr` -> `for _, x in pairs(arr) do ... end`
                    let discard = Path::Base(name!(cpar::DISCARD, span));
                    let pairs_call = span
                        * ExprKind::Call(
                            boxed!(access!(boxed!(Path::Base(name!("pairs", span))), span)),
                            [*src].into(),
                        );
                    StmtKind::ForGeneric(
                        [discard, Path::Base(name)].into(),
                        [pairs_call].into(),
                        Box::new(block),
                    )
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

    fn desugar_object(&self, object: ObjectDef, span: Span) -> StmtKind {
        let ObjectDef {
            global,
            name,
            attrs,
            base,
            properties,
            static_methods,
            methods,
            ..
        } = object;

        // function init(...)不带冒号也是实例构造器,
        // `__` 开头的 metamethod 同样作用于实例(self 注入)
        // parser 把无冒号函数归为 static,这里把它们转成实例方法
        let mut static_methods = static_methods.into_vec();
        let mut methods = methods.into_vec();
        {
            let mut i = 0;
            while i < static_methods.len() {
                let name = &static_methods[i].0.0;
                if name == csugar::INIT_FUNC || name.starts_with("__") {
                    let func = static_methods.remove(i);
                    methods.push(func);
                } else {
                    i += 1;
                }
            }
        }

        let data_attr = get_attr(&attrs, catt::DATA);
        let is_data_object = data_attr.is_some();
        let is_data_object_frozen =
            data_attr.is_some_and(|i| i.iter().any(|v| v.0.0 == "frozen" && v.1.eval_to_bool()));
        let base_super = base.as_ref().map(|(n, _)| n.clone());

        /*
          @data(frozen = false)
          Data Object:
            - 自动init, 自动new
            - 自动 __eq, 自动__tostring

          类表构造块:
           global A = do
               local _obj = {}
               _obj.__index = _obj                       -- 实例方法委托
               set_metatable(_obj, {__index = Base})      -- 若继承:类级方法链

               _obj.property = expr                      -- 类级默认属性

               function _obj:init(...) ... end           -- 构造器(用户 function :init 或默认空)
               function _obj.static() ... end            -- 静态方法
               function _obj:method() ... end            -- 实例方法(self 注入)
               function _obj.new(...)                    -- 自动生成工厂
                   local self = set_metatable({}, _obj)
                   self:init(...)                        -- 沿 __index 链自动串联祖先构造
                   return self
               end
               return _obj
           end
        */
        let obj_name = attrname!(csugar::OBJECT_TABLE, span);
        let has_base = base.is_some();

        let mut stmts: Vec<Stmt> = vec![
            span * define!(local { obj_name.clone() } = {
                Expr(ExprKind::Table([].into()), span)
            }),
            assign!(
                {
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Dot(name!(MetaMethod::Index.name(), span))
                } = { access!(boxed!(Path::Base(obj_name.0.0.clone())), span) },
                span
            ),
        ];

        if let Some((base_name, base_span)) = base {
            let callee = access!(boxed!(Path::Base(name!("set_metatable", span))), span);
            let meta = Expr(
                ExprKind::Table(
                    [Field::NameValue(
                        name!("__index", base_span),
                        access!(boxed!(Path::Base((base_name, base_span))), base_span),
                    )]
                    .into(),
                ),
                base_span,
            );
            let args = [
                access!(boxed!(Path::Base(obj_name.0.0.clone())), span),
                meta,
            ]
            .into();
            stmts.push(Stmt(StmtKind::Call(boxed!(callee), args), span));
        }

        // 收集属性, NameValue 有名字可作为 data 的 init 参数, KeyValue 保留 key 表达式
        let props: Vec<(Option<Name>, Option<Box<Expr>>, Expr)> = properties
            .into_iter()
            .map(|prop| match prop {
                ObjectProperty::NameValue((pname, pspan), val, _) => (
                    Some((pname, pspan)),
                    None,
                    val.map(|e| *e).unwrap_or(literal!(ConstValue::Nil, pspan)),
                ),
                ObjectProperty::KeyValue(key, val, _) => (
                    None,
                    Some(key),
                    val.map(|e| *e).unwrap_or(literal!(ConstValue::Nil, span)),
                ),
            })
            .collect();

        for (name, key, val) in &props {
            let path = match (name, key) {
                (Some((n, ns)), _) => {
                    Path::Base(obj_name.0.0.clone()) + PathSuffix::Dot((n.clone(), *ns))
                }
                (None, Some(k)) => Path::Base(obj_name.0.0.clone()) + PathSuffix::Index(k.clone()),
                (None, None) => unreachable!(),
            };
            stmts.push(assign!({ path } = { val.clone() }, span));
        }

        let has_init = methods
            .iter()
            .any(|(func_name, _, _)| func_name.0 == csugar::INIT_FUNC);
        let has_new = static_methods
            .iter()
            .any(|(func_name, _, _)| func_name.0 == csugar::NEW_FUNC);
        let has_eq = methods
            .iter()
            .any(|(func_name, _, _)| func_name.0 == MetaMethod::Eq.name());
        let has_to_string = methods
            .iter()
            .any(|(func_name, _, _)| func_name.0 == MetaMethod::ToString.name());

        // 无:init 时生成兜底构造器; data object 则以属性名为参数逐个赋值
        if !has_init && (is_data_object || !has_base) {
            let (params, body_stmts): (Vec<Param>, Vec<Stmt>) = if is_data_object {
                let params = props
                    .iter()
                    .filter_map(|(name, _, _)| name.clone().map(Param::Name))
                    .collect();
                let stmts = props
                    .iter()
                    .filter_map(|(name, _, _)| {
                        let (n, ns) = name.clone()?;
                        let target =
                            Path::Base(name!(cgen::SELF, span)) + PathSuffix::Dot((n.clone(), ns));
                        let value = access!(boxed!(Path::Base((n, ns))), span);
                        Some(assign!({ target } = { value }, span))
                    })
                    .collect();
                (params, stmts)
            } else {
                (vec![], vec![])
            };
            let body = FuncBody(
                params.into(),
                [].into(),
                None,
                Box::new(Block(body_stmts.into(), None)),
            );
            stmts.push(Stmt(
                StmtKind::Function(
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Colon(name!(csugar::INIT_FUNC, span)),
                    [].into(),
                    Box::new(body),
                    false,
                ),
                span,
            ));
        }

        // data object: 未定义 __eq 时自动比较所有属性
        if is_data_object && !has_eq {
            let other_name = name!("other", span);
            let mut cond: Option<Expr> = None;
            for (name, key, _) in &props {
                let member = |base: &(String, Span)| match (name, key) {
                    (Some((n, ns)), _) => {
                        Path::Base(base.clone()) + PathSuffix::Dot((n.clone(), *ns))
                    }
                    (None, Some(k)) => Path::Base(base.clone()) + PathSuffix::Index(k.clone()),
                    (None, None) => unreachable!(),
                };
                let lhs = access!(boxed!(member(&name!(cgen::SELF, span))), span);
                let rhs = access!(boxed!(member(&other_name)), span);
                let cmp = Expr(
                    ExprKind::Binary(boxed!(lhs), boxed!(rhs), BinOp::Equal),
                    span,
                );
                cond = Some(match cond {
                    None => cmp,
                    Some(prev) => Expr(
                        ExprKind::Binary(boxed!(prev), boxed!(cmp), BinOp::And),
                        span,
                    ),
                });
            }
            let ret = cond.map(|c| return_!([c].into(), span)).flatten();
            let body = FuncBody(
                [Param::Name(other_name)].into(),
                [].into(),
                None,
                Box::new(Block([].into(), ret)),
            );
            stmts.push(Stmt(
                StmtKind::Function(
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Colon(name!(MetaMethod::Eq.name(), span)),
                    [].into(),
                    Box::new(body),
                    false,
                ),
                span,
            ));
        }

        // data object: 未定义 __tostring 时自动拼接所有属性
        if is_data_object && !has_to_string {
            let concat = |left: Expr, right: Expr| {
                Expr(
                    ExprKind::Binary(boxed!(left), boxed!(right), BinOp::Concat),
                    span,
                )
            };
            let to_string_call = |target: Expr| {
                Expr(
                    ExprKind::Call(
                        boxed!(access!(boxed!(Path::Base(name!("to_string", span))), span)),
                        [target].into(),
                    ),
                    span,
                )
            };
            let lit = |s: &str| literal!(ConstValue::String(s.as_bytes().to_vec().into()), span);

            let mut chain = lit(&format!("{}{{", name.0));
            let prop_count = props.len();
            for (idx, (name, key, _)) in props.iter().enumerate() {
                let self_path = match (name, key) {
                    (Some((n, ns)), _) => {
                        Path::Base(name!(cgen::SELF, span)) + PathSuffix::Dot((n.clone(), *ns))
                    }
                    (None, Some(k)) => {
                        Path::Base(name!(cgen::SELF, span)) + PathSuffix::Index(k.clone())
                    }
                    (None, None) => unreachable!(),
                };
                let value_part = to_string_call(access!(boxed!(self_path), span));
                let prefix = match name {
                    Some((n, _)) => lit(&format!("{n}=")),
                    None => {
                        let key_expr = match key {
                            Some(k) => (**k).clone(),
                            None => unreachable!(),
                        };
                        concat(lit("["), concat(to_string_call(key_expr), lit("]=")))
                    }
                };
                chain = concat(chain, concat(prefix, value_part));
                if idx + 1 < prop_count {
                    chain = concat(chain, lit(", "));
                }
            }
            chain = concat(chain, lit("}"));
            let body = FuncBody(
                [].into(),
                [].into(),
                None,
                Box::new(Block([].into(), return_!([chain].into(), span))),
            );
            stmts.push(Stmt(
                StmtKind::Function(
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Colon(name!(MetaMethod::ToString.name(), span)),
                    [].into(),
                    Box::new(body),
                    false,
                ),
                span,
            ));
        }

        if is_data_object_frozen {
            let body = FuncBody([].into(), [].into(), None, Box::new(Block([].into(), None)));
            stmts.push(Stmt(
                StmtKind::Function(
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Colon(name!(MetaMethod::NewIndex.name(), span)),
                    [].into(),
                    Box::new(body),
                    false,
                ),
                span,
            ))
        }

        for (func_name, attrs, mut body) in static_methods {
            if let Some(sn) = &base_super {
                let mut replacer = SuperReplacer::new();
                replacer.super_name = Some(sn.clone());
                body.visit_mut(&mut replacer);
            }
            let path = Path::Base(obj_name.0.0.clone()) + PathSuffix::Dot(func_name);
            stmts.push(Stmt(
                StmtKind::Function(path, attrs, Box::new(body), false),
                span,
            ));
        }

        for (func_name, attrs, mut body) in methods {
            if let Some(sn) = &base_super {
                let mut replacer = SuperReplacer::new();
                replacer.super_name = Some(sn.clone());
                body.visit_mut(&mut replacer);
            }
            let path = Path::Base(obj_name.0.0.clone()) + PathSuffix::Colon(func_name);
            stmts.push(Stmt(
                StmtKind::Function(path, attrs, Box::new(body), false),
                span,
            ));
        }

        // 自动工厂:`function _obj.new(...) local self = set_metatable({}, _obj); self:init(...); return self end`
        if !has_new {
            let self_name = attrname!(cgen::SELF, span);
            let new_body = FuncBody(
                [Param::Var(span)].into(),
                [].into(),
                None,
                Box::new(Block(
                    [
                        span * define!(local { self_name.clone() } = {
                            Expr(
                                ExprKind::Call(
                                    boxed!(access!(
                                        boxed!(Path::Base(name!("set_metatable", span))),
                                        span
                                    )),
                                    [
                                        Expr(ExprKind::Table([].into()), span),
                                        access!(boxed!(Path::Base(obj_name.0.0.clone())), span),
                                    ]
                                    .into(),
                                ),
                                span,
                            )
                        }),
                        span * StmtKind::Call(
                            boxed!(access!(
                                boxed!(
                                    Path::Base(name!(cgen::SELF, span))
                                        + PathSuffix::Colon(name!(csugar::INIT_FUNC, span))
                                ),
                                span
                            )),
                            [Expr(ExprKind::VarArg, span)].into(),
                        ),
                    ]
                    .into(),
                    return_!(
                        [access!(boxed!(Path::Base(name!(cgen::SELF, span))), span)].into(),
                        span
                    ),
                )),
            );
            stmts.push(Stmt(
                StmtKind::Function(
                    Path::Base(obj_name.0.0.clone())
                        + PathSuffix::Dot(name!(csugar::NEW_FUNC, span)),
                    [].into(),
                    Box::new(new_body),
                    false,
                ),
                span,
            ));
        }

        let block = Block(
            stmts.into(),
            return_!(
                [access!(boxed!(Path::Base(obj_name.0.0.clone())), span)].into(),
                span
            ),
        );

        StmtKind::Define(
            [attrname!(name.0, name.1)].into(),
            [span * ExprKind::Do(Box::new(block))].into(),
            global,
        )
    }

    fn desugar_match(&self, r#match: Match, for_expr: bool) -> AdaptedIf {
        fn desugar_clause(target: Expr, clause: MatchClause) -> IfClause {
            let MatchClause((term, guard), block) = clause;

            fn desugar_term(
                target: Expr,
                term: PatternTerm,
                binds: &mut Vec<(Name, Expr)>,
            ) -> Expr {
                use PatternTerm::*;
                let span = target.1;
                Expr(
                    match term {
                        Constant(expr) => ExprKind::Binary(Box::new(target), expr, BinOp::Equal),
                        Type(..) => ExprKind::Literal(ConstValue::Bool(true)),
                        Bind(name, ty) => {
                            binds.push((name, target.clone()));
                            if let Some(ty) = ty {
                                if let Some(t) = ty.base_type() {
                                    type_to_checker(t.clone(), target)
                                } else {
                                    ExprKind::Literal(ConstValue::Bool(true))
                                }
                            } else {
                                ExprKind::Literal(ConstValue::Bool(true))
                            }
                        }
                        Call(expr) => ExprKind::Call(expr, [target].into()),
                        Compare(op, expr) => ExprKind::Binary(Box::new(target), expr, op),
                        Array(items) => {
                            let mut first_discard_many: Option<usize> = None;
                            let mut array_suffix_offset: usize = 0;
                            let mut array_index: usize = 0;
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

                            let len = items.len();
                            for term in items {
                                let target = access!(
                                    path!(
                                        (boxed!(target.clone()))[boxed!(if let Some(i) =
                                            first_discard_many
                                        {
                                            (span
                                                * ExprKind::Unary(
                                                    boxed!(target.clone()),
                                                    UnOp::Length,
                                                ))
                                                - (span
                                                    * ExprKind::Literal(ConstValue::Int(
                                                        ((len - i) - array_suffix_offset)
                                                            as DukaInt,
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
                                        if first_discard_many.is_some() {
                                            array_suffix_offset += n;
                                        } else {
                                            array_index += n;
                                        }
                                    }
                                    PatternArrayTerm::DiscardMany => {
                                        first_discard_many = Some(array_index);
                                    }
                                    PatternArrayTerm::Term(term) => {
                                        exprs.push(desugar_term(target, term, binds));
                                        if first_discard_many.is_some() {
                                            array_suffix_offset += 1;
                                        } else {
                                            array_index += 1;
                                        }
                                    }
                                }
                            }

                            let final_len = array_index + array_suffix_offset;

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
                        Table(fields) => {
                            let mut first_discard_many: Option<usize> = None;
                            let mut array_index: usize = 0;
                            let mut item_count: usize = 0;
                            let mut array_suffix_offset: usize = 0;
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
                                        exprs.push(desugar_term(target, term, binds));
                                    }
                                    FieldPattern::Expr(key, term) => {
                                        let key_span = key.1;
                                        let target = access!(
                                            path!((boxed!(target.clone()))[boxed!(key)]),
                                            key_span
                                        );
                                        item_count += 1;
                                        exprs.push(desugar_term(target, term, binds));
                                    }
                                    FieldPattern::Array(term) => {
                                        let target = access!(
                                            path!(
                                                (boxed!(target.clone()))[boxed!(if let Some(i) =
                                                    first_discard_many
                                                {
                                                    (span
                                                        * ExprKind::Unary(
                                                            boxed!(target.clone()),
                                                            UnOp::Length,
                                                        ))
                                                        - (span
                                                            * ExprKind::Literal(ConstValue::Int(
                                                                ((len - i) - array_suffix_offset)
                                                                    as DukaInt,
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
                                                if first_discard_many.is_some() {
                                                    array_suffix_offset += n;
                                                } else {
                                                    array_index += n;
                                                }
                                            }
                                            PatternArrayTerm::DiscardMany => {
                                                first_discard_many = Some(array_index);
                                            }
                                            PatternArrayTerm::Term(term) => {
                                                exprs.push(desugar_term(target, term, binds));
                                                if first_discard_many.is_some() {
                                                    array_suffix_offset += 1;
                                                } else {
                                                    array_index += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            let final_len = array_index + item_count + array_suffix_offset;

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
                            Box::new(desugar_term(target.clone(), *left, binds)),
                            Box::new(desugar_term(target, *right, binds)),
                            match op {
                                PatternOp::And => BinOp::And,
                                PatternOp::Or => BinOp::Or,
                                PatternOp::Xor => BinOp::Xor,
                            },
                        ),
                        Not(term) => {
                            ExprKind::Unary(Box::new(desugar_term(target, *term, binds)), UnOp::Not)
                        }
                    },
                    span,
                )
            }

            let mut binds = vec![];
            let cond = desugar_term(target, term, &mut binds);

            let if_block = if binds.is_empty() {
                block
            } else {
                let mut new_stmts: Vec<Stmt> = binds
                    .into_iter()
                    .map(|(name, value)| {
                        let span = name.1;
                        Stmt(
                            // local name = value
                            StmtKind::Define(
                                [attrname!(name.0, span)].into(),
                                [value].into(),
                                false,
                            ),
                            span,
                        )
                    })
                    .collect();
                let Block(stmts, tail) = *block;
                new_stmts.extend(stmts.to_vec());
                Box::new(Block(new_stmts.into(), tail))
            };

            IfClause(
                if_block,
                Box::new(if let Some(guard) = guard {
                    cond & *guard
                } else {
                    cond
                }),
            )
        }

        let Match(target, clauses, else_block) = r#match;

        let span = target.1;
        let def = span * define!(local {attrname!(csugar::MATCHEE, span)} = {*target.clone()});

        let mut desugareds = clauses.into_iter().map(|clause| {
            desugar_clause(
                access!(boxed!(Path::Base((csugar::MATCHEE.to_owned(), span))), span),
                clause,
            )
        });

        let Some(head) = desugareds.next() else {
            return else_block
                .map(|e| AdaptedIf::Do(e))
                .unwrap_or(AdaptedIf::Empty);
        };

        if for_expr {
            AdaptedIf::Do(boxed!(Block(
                [def].into(),
                Some(boxed!(Stmt(
                    StmtKind::Return(
                        [span * ExprKind::If(boxed!(If(head, desugareds.collect(), else_block)))]
                            .into(),
                    ),
                    span,
                )))
            )))
        } else {
            AdaptedIf::InsertStmts(
                def,
                Stmt(
                    StmtKind::If(If(head, desugareds.collect(), else_block)),
                    span,
                ),
            )
        }
    }
}

/// Desugar `export ...` sugar into writes against a synthetic module table.
///
/// Every `export` statement collects its declared value into a (module-local
/// global) table and a trailing `return <table>` is appended to the root block,
/// unless the module already returns explicitly.
pub struct ExportDesugarer {
    exports: Option<Name>,
    has_export: bool,
}
impl ExportDesugarer {
    pub fn new() -> Self {
        Self {
            exports: None,
            has_export: false,
        }
    }

    pub fn run(&mut self, chunk: &mut DukaChunk) {
        self.desugar_block(&mut chunk.block);
        if self.has_export && chunk.block.1.is_none() {
            let span = chunk.span;
            let exports = Path::Base(self.exports.as_ref().unwrap().clone());
            chunk.block.1 = Some(Box::new(Stmt(
                StmtKind::Return([access!(boxed!(exports), span)].into()),
                span,
            )));
        }
    }

    fn desugar_block(&mut self, block: &mut Block) {
        let stmts = mem::take(&mut block.0);
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.desugar_stmt(&mut out, stmt);
        }
        block.0 = out.into();
    }

    fn desugar_stmt(&mut self, out: &mut Vec<Stmt>, mut stmt: Stmt) {
        match &mut stmt.0 {
            StmtKind::Export(inner) => {
                let span = stmt.1;
                let inner = mem::take(inner);
                let export_name = (csugar::EXPORT_TABLE.to_owned(), span);
                let exports = self.exports.get_or_insert(export_name).clone();
                self.has_export = true;
                let ispan = inner.1;
                let mut collected = vec![];
                match &inner.0 {
                    StmtKind::Define(names, _, _) => {
                        for (((key, kspan), _, _), _) in names.iter() {
                            collected.push((key.clone(), *kspan));
                        }
                    }
                    StmtKind::Function(name, _, _, _) => {
                        collected.push((name.to_string(), name.get_span()));
                    }
                    StmtKind::Assign(names, _) => {
                        for name in names.iter() {
                            collected.push((name.to_string(), name.get_span()));
                        }
                    }
                    _ => (), // ignored,
                }
                out.push(*inner);
                if !collected.is_empty() {
                    out.push(Self::ensure_init(exports.clone(), ispan));
                    for (key, kspan) in collected {
                        out.push(Self::write_export(exports.clone(), key, kspan, ispan));
                    }
                }
            }
            _ => {
                match &mut stmt.0 {
                    StmtKind::If(if_) => {
                        let If(first, rest, els) = if_;
                        self.desugar_block(&mut first.0);
                        for clause in rest.iter_mut() {
                            self.desugar_block(&mut clause.0);
                        }
                        if let Some(els) = els {
                            self.desugar_block(els);
                        }
                    }
                    StmtKind::Do(b) => self.desugar_block(b),
                    StmtKind::While(_, b) => self.desugar_block(b),
                    StmtKind::ForNumeric(_, _, _, _, b) => self.desugar_block(b),
                    StmtKind::ForGeneric(_, _, b) => self.desugar_block(b),
                    StmtKind::Function(_, _, body, _) => self.desugar_block(&mut body.3),
                    _ => (),
                }
                out.push(stmt);
            }
        }
    }

    fn ensure_init(exports: Name, span: Span) -> Stmt {
        let target = Path::Base(exports);
        let cond = Expr(
            ExprKind::Unary(boxed!(access!(boxed!(target.clone()), span)), UnOp::Not),
            span,
        );
        let block = Block(
            [assign!(
                { target } = { Expr(ExprKind::Table([].into()), span) },
                span
            )]
            .into(),
            None,
        );
        Stmt(
            StmtKind::If(If(
                IfClause(Box::new(block), Box::new(cond)),
                Box::new([]),
                None,
            )),
            span,
        )
    }

    fn write_export(exports: Name, key: String, kspan: Span, span: Span) -> Stmt {
        let target = Path::Base(exports) + PathSuffix::Dot((key.clone(), kspan));
        let value = access!(boxed!(Path::Base((key, kspan))), span);
        assign!({ target } = { value }, span)
    }
}
