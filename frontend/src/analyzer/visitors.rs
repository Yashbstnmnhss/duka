use std::{mem, vec};

use duka_shared::{
    ast::{
        BinOp, Block, Expr, ExprKind, FieldPattern, FuncBody, If, IfClause, Linq, LinqClause,
        Match, MatchClause, Path, PathSuffix, Pattern, PatternArrayTerm, PatternOp, PatternTerm,
        Stmt, StmtKind, UnOp,
    },
    error::{DukaError, DukaSemanticError, Span},
    types::{Spanned, Visitor, VisitorMut},
    utils::{ScopeType, Scopes},
    value::{DukaFloat, DukaInt, Value},
};

macro_rules! checker {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*), $($visitor: item),+) => {
        pub struct $name {
            $($var_name : $var_type),*,
            errors: Vec<DukaError>
        }
        impl $name {
            pub fn new() -> Self {
                Self {
                    $($var_name: $var_val),*,
                    errors: vec![]
                }
            }
        }
        impl Visitor for $name {
            $($visitor)+
            fn report(&self) -> Vec<DukaError> {
                self.errors.clone()
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
        Path::Expr($e) + path!($($right)*)
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
        StmtKind::Define(vec![$name], vec![$expr], false)
    };
}
macro_rules! attrname {
    ($e: literal, $s: expr) => {
        ((name!($e, $s), vec![]), $s)
    };
}
macro_rules! name {
    ($e: literal, $s: expr) => {
        ($e.to_owned(), $s)
    };
}
macro_rules! assign {
    ({$target: expr} = {$expr: expr}, $s: expr) => {
        Stmt(StmtKind::Assign(vec![$target], vec![$expr]), $s)
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
            self.errors.push(DukaError {
                span: stmt.1,
                kind: DukaSemanticError::InvalidLoopFlowControl.into()
            })
        }
    }
}

macro_rules! label_visit_block {
    ($self: ident, $e: expr, $i: ident) => {
        if !$e {
            $self.check_pending_goto();
            $self.scopes.exit();
            return;
        }
        $self.scopes.enter(ScopeType::$i);
        $self.pending_goto.push(vec![]);
    };
}

checker! {
    LabelChecker(
        scopes: Scopes<String, ()> = {
            let mut s = Scopes::new();
            s.enter(ScopeType::Global);
            s
        },
        pending_goto: Vec<Vec<Spanned<String>>> = vec![vec![]] // this is for global
    ),
    fn visit_match_else_block(&mut self, _: &Match, enter: bool) {
        label_visit_block!(self, enter, ControlFlow);
    },
    fn visit_match_clause_block(&mut self, _: &MatchClause, enter: bool) {
        label_visit_block!(self, enter, ControlFlow);
    },
    fn visit_if_clause_block(&mut self, _: &IfClause, enter: bool) {
        label_visit_block!(self, enter, ControlFlow);
    },
    fn visit_loop_stmt_block(&mut self, _: &StmtKind, enter: bool) {
        label_visit_block!(self, enter, ControlFlow);
    },

    fn visit_func_block(&mut self, _: &FuncBody, enter: bool) {
        label_visit_block!(self, enter, Function);
    },

    fn visit_do_stmt_block(&mut self, _: &StmtKind, enter: bool) {
        label_visit_block!(self, enter, Do);
    },
    fn visit_do_expr_block(&mut self, _: &ExprKind, enter: bool) {
        label_visit_block!(self, enter, Do);
    },
    fn visit_stmt(&mut self, stmt: &Stmt)  {
        match stmt.0 {
            StmtKind::Label(ref label) => {
                if self.scopes.push(label.to_string(), ()).is_err(){
                    self.errors.push(DukaError {
                        kind: DukaSemanticError::DuplicatedItem("label".to_owned(), label.to_string()).into(),
                        span: stmt.1
                    });
                }
            }
            StmtKind::Goto(ref label) => {
                // checked, it must have the last one
                dbg!(label);
                dbg!(&self.pending_goto);
                self.pending_goto.last_mut().expect("im sure this wont happen").push((label.to_string(), stmt.1));
            }
            _ => ()
        }
    }
}
impl LabelChecker {
    fn check_pending_goto(&mut self) {
        if let Some(ps) = self.pending_goto.pop() {
            ps.into_iter().for_each(|(label, span)| {
                if !self.scopes.find_within(&label, ScopeType::Function) {
                    self.errors.push(DukaError {
                        kind: DukaSemanticError::InvisibleGotoLabel(label).into(),
                        span,
                    });
                }
            });
        }
    }
}

checker! {
    VarArgChecker(marks: Vec<u8> = vec![]),
    fn visit_expr(&mut self, expr: &Expr) {
        if !matches!(expr.0, ExprKind::VarArg) {
            return
        }
        if matches!(self.marks.last(), Some(0)) {
            self.errors.push(DukaError {
                kind: DukaSemanticError::InvalidVarArg.into(),
                span: expr.1
            })
        }
    },
    fn visit_func_block(&mut self, func: &FuncBody, enter: bool) {
        if enter {
            self.marks.push(if func.has_vararg() { 1 } else {0});
        } else {
            self.marks.pop();
        };
    }
}

transformer! {
    AttributeTransformer(),
    fn visit_stmt(&mut self, stmt: &mut Stmt) {
        match stmt.0 {
            _ => ()
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
                        let mut params = adapting!(<- params);
                        params.push(*l);
                        expr.0 = ExprKind::Call(func, params);
                    },
                    ExprKind::Access(_) => {
                        let r = adapting!(<- r);
                        let l = adapting!(<- l);
                        expr.0 = ExprKind::Call(r, vec![*l]);
                    }
                    _ => ()
                }
            },
            ExprKind::Binary(l, r, op) => {
                if let Some(new_expr) = Self::fold_binary(&l.0, &r.0, op) {
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
        fn do_unary(e: &Value, op: &UnOp) -> Option<Value> {
            match op {
                UnOp::BitNot => {
                    if let Value::Int(i) = e {
                        Some(Value::Int(!i))
                    } else {
                        None
                    }
                }
                UnOp::Minus => Some(match e {
                    Value::Int(i) => Value::Int(-i),
                    Value::Float(f) => Value::Float(-f),
                    _ => return None,
                }),
                UnOp::Not => Some(match e {
                    Value::Bool(b) => Value::Bool(!b),
                    Value::Nil => Value::Bool(true),
                    val if val.is_string() => Value::Bool(false),
                    Value::Int(..) | Value::Float(..) => Value::Bool(false),
                    _ => return None,
                }),
                UnOp::Length => match e {
                    str if str.is_string() => Some(Value::Int({
                        let string: &str = str.into();
                        string.len() as DukaInt
                    })),
                    Value::Table(table) if e.is_const() => {
                        let bt = table.borrow();
                        Some(Value::Int((bt.array.len() + bt.map.len()) as DukaInt))
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

    fn fold_binary(l: &ExprKind, r: &ExprKind, op: &BinOp) -> Option<ExprKind> {
        fn do_binary(lv: &Value, rv: &Value, op: &BinOp) -> Option<Value> {
            fn do_arith(
                lv: &Value,
                rv: &Value,
                fi: fn(DukaInt, DukaInt) -> DukaInt,
                ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
            ) -> Option<Value> {
                Some(match (lv, rv) {
                    (Value::Int(i1), Value::Int(i2)) => Value::Int(fi(*i1, *i2)),
                    (Value::Float(f1), Value::Float(f2)) => Value::Float(ff(*f1, *f2)),
                    (Value::Int(i), Value::Float(f)) => Value::Float(ff(*i as DukaFloat, *f)),
                    (Value::Float(f), Value::Int(i)) => Value::Float(ff(*f, *i as DukaFloat)),
                    _ => return None,
                })
            }

            fn do_arith_i(
                lv: &Value,
                rv: &Value,
                fi: fn(DukaInt, DukaInt) -> DukaInt,
            ) -> Option<Value> {
                let (a, b) = match (lv, rv) {
                    (Value::Int(i1), Value::Int(i2)) => (*i1, *i2),
                    (Value::Float(f1), Value::Float(f2)) => (*f1 as DukaInt, *f2 as DukaInt),
                    (Value::Int(i), Value::Float(f)) => (*i, *f as DukaInt),
                    (Value::Float(f), Value::Int(i)) => (*f as DukaInt, *i),
                    _ => return None,
                };
                Some(Value::Int(fi(a, b)))
            }

            fn do_arith_f(
                lv: &Value,
                rv: &Value,
                ff: fn(DukaFloat, DukaFloat) -> DukaFloat,
            ) -> Option<Value> {
                let (a, b) = match (lv, rv) {
                    (Value::Int(i1), Value::Int(i2)) => (*i1 as DukaFloat, *i2 as DukaFloat),
                    (Value::Float(f1), Value::Float(f2)) => (*f1, *f2),
                    (Value::Int(i), Value::Float(f)) => (*i as DukaFloat, *f),
                    (Value::Float(f), Value::Int(i)) => (*f, *i as DukaFloat),
                    _ => return None,
                };
                Some(Value::Float(ff(a, b)))
            }

            match op {
                BinOp::Add => do_arith(lv, rv, |a, b| a + b, |a, b| a + b),
                BinOp::Sub => do_arith(lv, rv, |a, b| a - b, |a, b| a - b),
                BinOp::Multiply => do_arith(lv, rv, |a, b| a * b, |a, b| a * b),
                BinOp::Divide => do_arith_f(lv, rv, |a, b| a / b),
                BinOp::Pow => do_arith_f(lv, rv, |a, b| a.powf(b)),
                BinOp::IDivide => do_arith(lv, rv, |a, b| a / b, |a, b| a / b),
                BinOp::Mod => do_arith_i(lv, rv, |a, b| a % b),

                BinOp::BitAnd => do_arith_i(lv, rv, |a, b| a & b),
                BinOp::BitOr => do_arith_i(lv, rv, |a, b| a | b),
                BinOp::BitXor => do_arith_i(lv, rv, |a, b| a ^ b),
                BinOp::ShiftL => do_arith_i(lv, rv, |a, b| a << b),
                BinOp::ShiftR => do_arith_i(lv, rv, |a, b| a >> b),

                BinOp::Concat => {
                    if lv.is_string() && rv.is_string() {
                        let a: &str = lv.into();
                        let b: &str = rv.into();
                        Some(format!("{}{}", a, b).into())
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
                let result = match self.adapt_if(target) {
                    AdaptedIf::Empty => ExprKind::Empty,
                    AdaptedIf::Do(block) => ExprKind::Do(block),
                    AdaptedIf::If(if_) => ExprKind::If(if_)
                };
                expr.0 = result
            },
            ExprKind::Do(ref v) if v.is_empty() => {
                expr.0 = ExprKind::Literal(Value::Nil);
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
                    AdaptedIf::Do(block) => StmtKind::Do(block),
                    AdaptedIf::If(if_) => StmtKind::If(if_)
                };
                stmt.0 = result
            },
            StmtKind::While(Expr(ExprKind::Literal(Value::Bool(false)), _), _) => {
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
                            if let Expr(ExprKind::Access(ref path2), _) = b && a == *path2 {
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

                stmt.0 = StmtKind::Assign(left, right);
            },
            _ => ()
        }
    }
}

enum AdaptedIf {
    If(If),
    Do(Block),
    Empty,
}
impl MeaninglessTransformer {
    fn adapt_if(&self, target: If) -> AdaptedIf {
        fn adapt_if_inner(
            if_clause: IfClause,
            else_if_clauses: Vec<IfClause>,
            else_clause: Option<Block>,
        ) -> AdaptedIf {
            enum AdaptedClause {
                Never,
                Always,
                Keep,
            }
            fn adapt_if_clause(clause: &IfClause) -> AdaptedClause {
                let IfClause(block, cond) = clause;
                match cond.0 {
                    ExprKind::Literal(Value::Bool(b)) => b
                        .then_some(
                            block
                                .is_empty()
                                .then_some(AdaptedClause::Never)
                                .unwrap_or(AdaptedClause::Always),
                        )
                        .unwrap_or(AdaptedClause::Never),
                    _ => AdaptedClause::Keep,
                }
            }

            match adapt_if_clause(&if_clause) {
                AdaptedClause::Always => AdaptedIf::Do(if_clause.0),
                AdaptedClause::Never => {
                    if else_if_clauses.is_empty() {
                        else_clause.map(AdaptedIf::Do).unwrap_or(AdaptedIf::Empty)
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

                    AdaptedIf::If(If(if_clause, new_else_if, else_clause))
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
            return
        }
    },
    fn visit_expr(&mut self, expr: &mut Expr) {
        if !expr.0.is_sugar() {
            return
        }
        match &expr.0 {
            ExprKind::Linq(_) => {
                let Expr(ek, span) = adapting!(<- expr);
                let ExprKind::Linq(linq) = ek else { unreachable!() };
                let new_ek = self.desugar_linq(linq, span);
                adapting!(expr <- Expr(new_ek, span));
            },
            ExprKind::Match(_) => {
                let Expr(ek, span) = adapting!(<- expr);
                let ExprKind::Match(m) = ek else { unreachable!() };
                let r#if = self.desugar_match(m, span);
                adapting!(expr <- Expr(match r#if {
                    AdaptedIf::Do(b) => ExprKind::Do(b),
                    AdaptedIf::Empty => ExprKind::Empty,
                    AdaptedIf::If(r#if) => ExprKind::If(r#if)
                }, span));
            },
            _ => ()
        }
    }
}
impl DesugarTransformer {
    fn desugar_linq(&self, linq: Linq, span: Span) -> ExprKind {
        let Linq(clauses, select) = linq;

        let target_name = attrname!("_s_リスト", span);
        let target_def =
            span * define!(local { target_name.clone() } = { literal!(Value::new_table(), span) });
        let index_name = attrname!("_s_イダクス", span);
        let index_def =
            span * define!(local { index_name.clone() } = { literal!(Value::Int(0), span) });

        let mut iter = clauses.into_iter().rev();

        let inner = iter
            .next() // this wont happen, im sure if parser is working correctly
            .expect("NO, it must contain at lease one clause!");
        let init = span
            * make_stmt(
                inner,
                Block(
                    vec![
                        assign!(
                            {
                                Path::Base(target_name.0.0.clone())
                                    + PathSuffix::Index(boxed!(access!(
                                        Path::Base(index_name.0.0.clone()),
                                        span
                                    )))
                            } = { *select },
                            span
                        ),
                        assign!(
                            { Path::Base(index_name.0.0.clone()) } = {
                                binary!(
                                    {boxed!(access!(Path::Base(index_name.0.0), span))}
                                    Add
                                    {boxed!(literal!(Value::Int(0), span))},
                                    span
                                )
                            },
                            span
                        ),
                    ],
                    None,
                ),
            );

        let body = iter.fold(init, |acc, clause| {
            span * make_stmt(clause, Block(vec![acc], None))
        });

        fn make_stmt(clause: LinqClause, block: Block) -> StmtKind {
            match clause {
                LinqClause::From(name, src) => {
                    StmtKind::ForGeneric(vec![Path::Base(name)], vec![*src], block)
                }
                LinqClause::Where(cond) => StmtKind::If(If(IfClause(block, cond), vec![], None)),
            }
        }

        ExprKind::Do(Block(
            vec![target_def, index_def, body],
            return_!(vec![access!(Path::Base(target_name.0.0), span)], span),
        ))
    }

    fn desugar_match(&self, r#match: Match, span: Span) -> AdaptedIf {
        fn desugar_clause(target: Expr, clause: MatchClause) -> IfClause {
            let MatchClause((term, guard), block) = clause;

            fn desugar_term(target: Expr, term: PatternTerm) -> Expr {
                use PatternTerm::*;
                let span = target.1;
                Expr(
                    match term {
                        Constant(expr) => ExprKind::Binary(Box::new(target), expr, BinOp::Equal),
                        Bind(_) => ExprKind::Literal(Value::Bool(true)), // deal it in block
                        Call(expr) => ExprKind::Call(expr, vec![target]),
                        Compare(op, expr) => ExprKind::Binary(Box::new(target), expr, op),
                        Table(fields) => {
                            let mut first_discord_many: Option<usize> = None;
                            let mut array_index: usize = 0;
                            let mut item_count: usize = 0;
                            let mut terms = vec![
                                span * ExprKind::Call(
                                    boxed!(access!(
                                        Path::Base(("_s_タイプ_イズ".to_owned(), span)),
                                        span
                                    )),
                                    vec![target.clone()],
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
                                        terms.push(desugar_term(target, term));
                                    }
                                    FieldPattern::Expr(key, term) => {
                                        let key_span = key.1;
                                        let target = access!(
                                            path!((boxed!(target.clone()))[boxed!(key)]),
                                            key_span
                                        );
                                        item_count += 1;
                                        terms.push(desugar_term(target, term));
                                    }
                                    FieldPattern::Array(term) => {
                                        let target = access!(
                                            path!(
                                                (boxed!(target.clone()))[boxed!(
                                                    span * (if let Some(i) = first_discord_many {
                                                        ExprKind::Binary(
                                                            boxed!(
                                                                span * ExprKind::Unary(
                                                                    boxed!(target.clone()),
                                                                    UnOp::Length
                                                                )
                                                            ),
                                                            boxed!(
                                                                span * ExprKind::Literal(
                                                                    Value::Int(
                                                                        (len - i - array_index)
                                                                            as DukaInt
                                                                    )
                                                                )
                                                            ),
                                                            BinOp::Sub,
                                                        )
                                                    } else {
                                                        ExprKind::Literal(Value::Int(
                                                            array_index as DukaInt,
                                                        ))
                                                    })
                                                )]
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
                                                terms.push(desugar_term(target, term));
                                                array_index += 1;
                                            }
                                        }
                                    }
                                }
                            }

                            let final_len = array_index + item_count;
                            terms.push(
                                span * ExprKind::Binary(
                                    boxed!(span * ExprKind::Unary(boxed!(target), UnOp::Length)),
                                    boxed!(
                                        span * ExprKind::Literal(Value::Int(final_len as DukaInt))
                                    ),
                                    BinOp::GreaterEqual,
                                ),
                            );
                            todo!()
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
            todo!()
        }

        let Match(target, clauses, else_block) = r#match;

        let desugared = clauses
            .into_iter()
            .map(|clause| desugar_clause(*target.clone(), clause));

        if desugared.len() == 0 {
            return else_block.map(AdaptedIf::Do).unwrap_or(AdaptedIf::Empty);
        }

        AdaptedIf::If(If(todo!(), vec![], else_block))
    }
}
