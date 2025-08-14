pub mod visitors;

use duka_shared::{
    ast::{
        Block, Expr, ExprKind, FuncBody, If, IfClause, Match, MatchClause, Path, Stmt, StmtKind,
    },
    error::DukaError,
    types::{DukaAdapter, DukaAnalyzer},
};

pub struct Analyzer {
    checkers: Vec<Box<dyn Checker>>,
}
impl Analyzer {
    pub fn new() -> Self {
        Self { checkers: vec![] }
    }
    pub fn with<T: Checker + 'static>(mut self, checker: T) -> Self {
        self.checkers.push(Box::new(checker));
        self
    }
}
impl DukaAnalyzer for Analyzer {
    type InputType = Block;

    fn analyze(mut self, chunk: &Block) -> Vec<DukaError> {
        self.checkers
            .iter_mut()
            .flat_map(|checker| check(checker, chunk))
            .collect()
    }
}

pub struct Adapter {
    transfomers: Vec<Box<dyn Transformer>>,
}
impl Adapter {
    pub fn new() -> Self {
        Self {
            transfomers: vec![],
        }
    }
    pub fn with<T: Transformer + 'static>(mut self, transformer: T) -> Self {
        self.transfomers.push(Box::new(transformer));
        self
    }
}
impl DukaAdapter for Adapter {
    type InputType = Block;

    fn adapt(mut self, chunk: &mut Block) {
        self.transfomers
            .iter_mut()
            .for_each(|transformer| transform(transformer, chunk));
    }
}

pub fn check(checker: &mut Box<dyn Checker>, chunk: &Block) -> Vec<DukaError> {
    fn walk_block(checker: &mut Box<dyn Checker>, ty: &BlockType, block: &Block) {
        checker.enter_block(ty);
        block.0.iter().for_each(|stmt| walk_stmt(checker, stmt));
        checker.exit_block(ty);
    }
    fn walk_stmt(checker: &mut Box<dyn Checker>, stmt: &Stmt) {
        match stmt.0 {
            StmtKind::Expr(ref expr) => walk_expr(checker, expr),
            StmtKind::Do(ref block) => walk_block(checker, &BlockType::Stmt(stmt), block),
            StmtKind::ForGeneric(_, ref exprs, ref block) => {
                exprs.iter().for_each(|expr| walk_expr(checker, expr));
                walk_block(checker, &BlockType::Stmt(stmt), block);
            }
            StmtKind::ForNumberic(_, ref expr1, ref expr2, ref expr3, ref block) => {
                walk_expr(checker, expr1);
                walk_expr(checker, expr2);
                if let Some(expr3) = expr3 {
                    walk_expr(checker, expr3);
                }
                walk_block(checker, &BlockType::Stmt(stmt), block);
            }
            StmtKind::While(ref expr, ref block) => {
                walk_expr(checker, expr);
                walk_block(checker, &BlockType::Stmt(stmt), block);
            }

            StmtKind::Function(_, _, FuncBody(.., ref block), _) => {
                walk_block(checker, &BlockType::Stmt(stmt), block);
            }

            StmtKind::If(ref if_def) => {
                walk_if(checker, &BlockType::Stmt(stmt), if_def);
                walk_stmt(checker, stmt);
            }
            StmtKind::Match(ref match_def) => {
                walk_match(checker, &BlockType::Stmt(stmt), match_def);
                walk_stmt(checker, stmt);
            }
            StmtKind::Empty => (),
            StmtKind::Define(_, ref exprs, _) | StmtKind::Assign(_, ref exprs) => {
                exprs.iter().for_each(|expr| walk_expr(checker, expr));
                checker.visit_stmt(stmt);
            }
            StmtKind::Call(ref func, ref params) => {
                walk_expr(checker, func);
                params.iter().for_each(|param| walk_expr(checker, param));
                checker.visit_stmt(stmt);
            }
            StmtKind::Return(ref exprs) => {
                exprs.iter().for_each(|expr| walk_expr(checker, expr));
                checker.visit_stmt(stmt);
            }
            _ => checker.visit_stmt(stmt),
        }
    }
    fn walk_expr(checker: &mut Box<dyn Checker>, expr: &Expr) {
        if !checker.should_visit_expr() {
            return;
        }
        match expr.0 {
            ExprKind::Unary(ref expr, _) => walk_expr(checker, expr),
            ExprKind::Binary(ref expr1, ref expr2, _) => {
                walk_expr(checker, expr1);
                walk_expr(checker, expr2);
            }
            ExprKind::Call(ref expr1, ref expr2) => {
                walk_expr(checker, expr1);
                expr2.iter().for_each(|expr| walk_expr(checker, expr));
            }
            ExprKind::Access(ref path) => {
                do_path(checker, path);
                fn do_path(visitor: &mut Box<dyn Checker>, path: &Path) {
                    match path {
                        Path::Base(..) => (),
                        Path::Chain(path, _) => do_path(visitor, path),
                        Path::Expr(expr) => walk_expr(visitor, expr),
                    }
                }
            }
            ExprKind::Function(ref func) => {
                walk_block(checker, &BlockType::AnonymousFunc(func), &func.1);
            }
            ExprKind::Match(ref match_def) => {
                walk_match(checker, &BlockType::Expr(expr), match_def)
            }
            ExprKind::If(ref if_def) => walk_if(checker, &BlockType::Expr(expr), if_def),
            _ => (),
        }
        checker.visit_expr(expr)
    }
    fn walk_if(checker: &mut Box<dyn Checker>, ty: &BlockType, if_def: &If) {
        let If(if_head, elseif, else_tail) = if_def;
        walk_expr(checker, &if_head.1);
        walk_block(checker, ty, &if_head.0);
        elseif.iter().for_each(|IfClause(block, expr)| {
            walk_expr(checker, expr);
            walk_block(checker, ty, block);
        });
        if let Some(block) = else_tail {
            walk_block(checker, ty, block);
        }
    }
    fn walk_match(checker: &mut Box<dyn Checker>, ty: &BlockType, match_def: &Match) {
        let Match(expr, clauses, block) = match_def;
        walk_expr(checker, expr);
        clauses.iter().for_each(|MatchClause((_, guard), block)| {
            if let Some(expr) = guard {
                walk_expr(checker, expr);
            }
            walk_block(checker, ty, block);
        });
        if let Some(block) = block {
            walk_block(checker, ty, block);
        }
    }

    walk_block(checker, &BlockType::Global, chunk);
    checker.report()
}

pub fn transform(transformer: &mut Box<dyn Transformer>, chunk: &mut Block) {
    fn walk_block(transformer: &mut Box<dyn Transformer>, block: &mut Block) {
        block
            .0
            .iter_mut()
            .for_each(|stmt| walk_stmt(transformer, stmt));
    }
    fn walk_stmt(transformer: &mut Box<dyn Transformer>, stmt: &mut Stmt) {
        match stmt.0 {
            StmtKind::Expr(ref mut expr) => {
                walk_expr(transformer, expr);
                transformer.adapt_stmt(stmt);
            }
            StmtKind::Do(ref mut block) => {
                walk_block(transformer, block);
                transformer.adapt_stmt(stmt);
            }
            StmtKind::ForGeneric(_, ref mut exprs, ref mut block) => {
                exprs
                    .iter_mut()
                    .for_each(|expr| walk_expr(transformer, expr));
                walk_block(transformer, block);
                transformer.adapt_stmt(stmt);
            }
            StmtKind::ForNumberic(
                _,
                ref mut expr1,
                ref mut expr2,
                ref mut expr3,
                ref mut block,
            ) => {
                walk_expr(transformer, expr1);
                walk_expr(transformer, expr2);
                if let Some(expr3) = expr3 {
                    walk_expr(transformer, expr3);
                }
                walk_block(transformer, block);
                transformer.adapt_stmt(stmt);
            }
            StmtKind::While(ref mut expr, ref mut block) => {
                walk_expr(transformer, expr);
                walk_block(transformer, block);
                transformer.adapt_stmt(stmt);
            }

            StmtKind::Function(_, _, FuncBody(.., ref mut block), _) => {
                walk_block(transformer, block);
                transformer.adapt_stmt(stmt);
            }

            StmtKind::If(If(ref mut if_head, ref mut elseif, ref mut else_tail)) => {
                walk_expr(transformer, &mut if_head.1);
                walk_block(transformer, &mut if_head.0);
                elseif.iter_mut().for_each(|IfClause(block, expr)| {
                    walk_expr(transformer, expr);
                    walk_block(transformer, block);
                });
                if let Some(block) = else_tail {
                    walk_block(transformer, block);
                }
                transformer.adapt_stmt(stmt);
            }
            StmtKind::Empty => (),
            StmtKind::Define(_, ref mut exprs, _) | StmtKind::Assign(_, ref mut exprs) => {
                exprs
                    .iter_mut()
                    .for_each(|expr| walk_expr(transformer, expr));
                transformer.adapt_stmt(stmt);
            }
            StmtKind::Call(ref mut func, ref mut params) => {
                walk_expr(transformer, func);
                params
                    .iter_mut()
                    .for_each(|param| walk_expr(transformer, param));
                transformer.adapt_stmt(stmt);
            }
            StmtKind::Return(ref mut exprs) => {
                exprs
                    .iter_mut()
                    .for_each(|expr| walk_expr(transformer, expr));
                transformer.adapt_stmt(stmt);
            }
            _ => transformer.adapt_stmt(stmt),
        }
    }
    fn walk_expr(transformer: &mut Box<dyn Transformer>, expr: &mut Expr) {
        if !transformer.should_adapt_expr() {
            return;
        }

        match expr.0 {
            ExprKind::Unary(ref mut expr, _) => walk_expr(transformer, expr),
            ExprKind::Binary(ref mut expr1, ref mut expr2, _) => {
                walk_expr(transformer, expr1);
                walk_expr(transformer, expr2);
            }
            ExprKind::Call(ref mut expr1, ref mut expr2) => {
                walk_expr(transformer, expr1);
                expr2
                    .iter_mut()
                    .for_each(|expr| walk_expr(transformer, expr));
            }
            ExprKind::Access(ref mut path) => {
                do_path(transformer, path);
                fn do_path(transformer: &mut Box<dyn Transformer>, path: &mut Path) {
                    match path {
                        Path::Base(..) => (),
                        Path::Chain(path, _) => do_path(transformer, path),
                        Path::Expr(expr) => walk_expr(transformer, expr),
                    }
                }
            }
            ExprKind::Function(FuncBody(.., ref mut body)) => {
                walk_block(transformer, body);
            }
            _ => (),
        }
        transformer.adapt_expr(expr)
    }

    walk_block(transformer, chunk);
}

pub enum BlockType<'a> {
    Global,
    AnonymousFunc(&'a FuncBody),
    Stmt(&'a Stmt),
    Expr(&'a Expr),
}
impl BlockType<'_> {
    #[inline]
    pub const fn is_func(&self) -> bool {
        matches!(
            self,
            Self::Stmt((StmtKind::Function(..), _)) | Self::AnonymousFunc(..)
        )
    }
    #[inline]
    pub const fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }
}

pub trait Checker {
    /// ## this will not contain Do, While, If, For and Function
    fn visit_stmt(&mut self, _stmt: &Stmt) {}
    /// ## this will behave like DFS
    fn visit_expr(&mut self, _expr: &Expr) {}
    fn enter_block(&mut self, _ty: &BlockType) {}
    fn exit_block(&mut self, _ty: &BlockType) {}
    /// must be implemented, this is used for errors collecting
    fn report(&self) -> Vec<DukaError>;

    #[inline]
    fn should_visit_stmt(&self) -> bool {
        true
    }
    #[inline]
    fn should_visit_expr(&self) -> bool {
        true
    }
}

pub trait Transformer {
    /// ## this contains Do, While, If, For and Function
    fn adapt_stmt(&mut self, _stmt: &mut Stmt) {}
    /// ## this will reach deeper
    fn adapt_expr(&mut self, _expr: &mut Expr) {}

    #[inline]
    fn should_adapt_stmt(&self) -> bool {
        true
    }
    #[inline]
    fn should_adapt_expr(&self) -> bool {
        true
    }
}
