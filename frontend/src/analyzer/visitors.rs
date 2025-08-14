use std::mem;

use crate::analyzer::{BlockType, Checker, Transformer};
use duka_shared::{
    ast::{
        BinOp, Block, Expr, ExprKind, If, IfClause, Linq, LinqClause, Path, PathSuffix, Stmt,
        StmtKind, UnOp,
    },
    error::{DukaError, DukaSemanticError, Span},
    types::Spanned,
    utils::{ScopeType, Scopes},
    value::{DukaFloat, DukaInt, Value},
};

macro_rules! checker {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*)[stmt: $s: literal, expr: $e: literal], $($visitor: item),+) => {
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
        impl Checker for $name {
            $($visitor)+
            fn report(&self) -> Vec<DukaError> {
                self.errors.clone()
            }

            fn should_visit_stmt(&self) -> bool {
                $s
            }
            fn should_visit_expr(&self) -> bool {
                $e
            }
        }
    };
}
macro_rules! transformer {
    ($name: ident ($($var_name: ident : $var_type: ty = $var_val: expr),*)[stmt: $s: literal, expr: $e: literal], $($visitor: item),+) => {
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
        impl Transformer for $name {
            $($visitor)+

            fn should_adapt_stmt(&self) -> bool {
                $s
            }
            fn should_adapt_expr(&self) -> bool {
                $e
            }
        }
    };
}

macro_rules! takeout {
    ($input: expr) => {
        mem::take($input)
    };
}
macro_rules! putback {
    ($src: ident <- $val: expr) => {
        let _ = mem::replace($src, $val);
    };
}

macro_rules! return_ {
    ($e: expr, $s: expr) => {
        Some(Box::new((StmtKind::Return($e), $s)))
    };
}
macro_rules! access {
    ($e: expr, $s: expr) => {
        (ExprKind::Access($e), $s)
    };
}
macro_rules! literal {
    ($e: expr, $s: expr) => {
        (ExprKind::Literal($e), $s)
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
        (StmtKind::Assign(vec![$target], vec![$expr]), $s)
    };
}
macro_rules! binary {
    ({$l:expr} $op:ident {$r:expr}, $s: expr) => {
        (ExprKind::Binary($l, $r, BinOp::$op), $s)
    };
}
macro_rules! boxed {
    ($e: expr) => {
        Box::new($e)
    };
}

checker! {
    LoopChecker(loop_depth: usize = 0)[stmt: true, expr: false],
    fn enter_block(&mut self, head: &BlockType) {
        if let BlockType::Stmt(head) = head && matches!
        (head.0,
            StmtKind::ForGeneric(..) |
            StmtKind::ForNumberic(..) |
            StmtKind::While(..)
        ) {
            self.loop_depth += 1;
        }
    },
    fn exit_block(&mut self, head: &BlockType) {
        if let BlockType::Stmt(head) = head && matches!
        (head.0,
            StmtKind::ForGeneric(..) |
            StmtKind::ForNumberic(..) |
            StmtKind::While(..)
        ) {
            self.loop_depth -= 1;
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

checker! {
    LabelChecker(
        scopes: Scopes<String, ()> = Scopes::new(),
        pending_goto: Vec<Vec<Spanned<String>>> = vec![]
    )[stmt: true, expr: false],
    fn enter_block(&mut self, head: &BlockType) {
        if head.is_func() || head.is_global() {
            self.pending_goto.push(vec![]);
        }

        self.scopes.enter(match head {
            BlockType::Expr(..) =>
                ScopeType::Do,
            BlockType::Stmt(head) =>
                match head.0 {
                    StmtKind::If(..) |
                    StmtKind::ForNumberic(..) |
                    StmtKind::ForGeneric(..) |
                    StmtKind::While(..) => ScopeType::ControlFlow,

                    StmtKind::Function(..) => ScopeType::Function,
                    StmtKind::Do(..) => ScopeType::Do,

                    _ => unreachable!()
                }
            BlockType::Global => ScopeType::Global,
            BlockType::AnonymousFunc(..) => ScopeType::Function,
        });
    },
    fn exit_block(&mut self, _: &BlockType) {
        self.check_pending_goto();
        self.scopes.exit();
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
                self.pending_goto.last_mut().expect("im sure this wont happen").push((label.to_string(), stmt.1));
            }
            _ => ()
        }
    }
}
impl LabelChecker {
    fn check_pending_goto(&mut self) {
        self.pending_goto
            .pop()
            .expect("im sure this wont happen")
            .into_iter()
            .for_each(|(label, span)| {
                if !self.scopes.find_within(&label, ScopeType::Function) {
                    self.errors.push(DukaError {
                        kind: DukaSemanticError::InvisibleGotoLabel(label).into(),
                        span,
                    });
                }
            });
    }
}

checker! {
    VarArgChecker(marks: Vec<u8> = vec![])[stmt: true, expr: true],
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
    fn enter_block(&mut self, head: &BlockType) {
        if let BlockType::Stmt(head) = head &&
            let StmtKind::Function(_, _, ref func, _) = head.0 {
            self.marks.push(if func.has_vararg() { 1 } else { 0 });
        }
    },
    fn exit_block(&mut self, head: &BlockType) {
        if head.is_func() {
            self.marks.pop();
        }
    }
}

transformer! {
    ConstFoldTransformer()[stmt: true, expr: true],
    fn adapt_expr(&mut self, expr: &mut Expr) {
        match &mut expr.0 {
            ExprKind::Binary(l, r, op @ BinOp::Pipeline | op @ BinOp::PipelineL) => {
                if matches!(op, BinOp::PipelineL) {
                    mem::swap(l, r);
                }
                match &mut r.0 {
                    ExprKind::Call(func, params) => {
                        let l = mem::take(l);
                        let func = mem::take(func);
                        let mut params = mem::take(params);
                        params.push(*l);
                        expr.0 = ExprKind::Call(func, params);
                    },
                    ExprKind::Access(_) => {
                        let r = mem::take(r);
                        let l = mem::take(l);
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
    MeaninglessTransformer()[stmt: true, expr: false],
    fn adapt_stmt(&mut self, stmt: &mut Stmt) {
        match stmt.0 {
            StmtKind::If(If(IfClause(ref mut b, ref expr), ref e, ref mut el)) if e.is_empty() => {
                if let ExprKind::Literal(Value::Bool(c)) = expr.0 {
                    stmt.0 = if c {
                        StmtKind::Do(mem::replace(b, Block::default()))
                    } else {
                        if let Some(block) = el.take() {
                            StmtKind::Do(block)
                        } else {
                            StmtKind::default()
                        }
                    }
                }
            },
            StmtKind::While((ExprKind::Literal(Value::Bool(false)), _), _) => {
                stmt.0 = StmtKind::default()
            }
            _ => ()
        }
    }
}

// TODO:
transformer! {
    DesugarTransformer()[stmt: true, expr: true],
    fn adapt_stmt(&mut self, stmt: &mut Stmt) {

    },
    fn adapt_expr(&mut self, expr: &mut Expr) {
        match &expr.0 {
            ExprKind::Linq(_) => {
                let (ek, span) = takeout!(expr);
                let ExprKind::Linq(linq) = ek else { unreachable!() };
                let new_ek = self.desugar_linq(linq, span);
                putback!(expr <- (new_ek, span));
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
            .next() // this wont happen, checked
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
}
