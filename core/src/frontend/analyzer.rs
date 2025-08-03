use crate::{
    frontend::{
        ast::{Block, Expr, ExprKind, FuncBody, IfClause, Path, Stmt, StmtKind},
        visitors::{LabelChecker, LoopChecker, VarArgChecker},
    },
    shared::{error::DukaError, types::DukaAnalyzer},
};

pub struct Analyzer {
    walker: Walker,
}
impl Analyzer {
    pub fn new() -> Self {
        Self {
            walker: Walker::new()
                .add_checker(LoopChecker::new())
                .add_checker(LabelChecker::new())
                .add_checker(VarArgChecker::new()),
        }
    }
}
impl DukaAnalyzer for Analyzer {
    fn analyze(&mut self, chunk: &Block) -> Result<(), Vec<DukaError>> {
        self.walker.check(chunk)
    }
}

pub struct Walker {
    checkers: Vec<Box<dyn Checker>>,
    transformers: Vec<Box<dyn Transformer>>,
}

impl Walker {
    pub fn new() -> Self {
        Self {
            checkers: vec![],
            transformers: vec![],
        }
    }

    pub fn add_checker<T: Checker + 'static>(mut self, checker: T) -> Self {
        self.checkers.push(Box::new(checker));
        self
    }
    pub fn add_transformer<T: Transformer + 'static>(mut self, transformer: T) -> Self {
        self.transformers.push(Box::new(transformer));
        self
    }

    pub fn check(&mut self, chunk: &Block) -> Result<(), Vec<DukaError>> {
        fn walk_block(visitor: &mut Box<dyn Checker>, ty: &BlockType, block: &Block) {
            visitor.enter_block(ty);
            block.0.iter().for_each(|stmt| walk_stmt(visitor, stmt));
            visitor.exit_block(ty);
        }
        fn walk_stmt(visitor: &mut Box<dyn Checker>, stmt: &Stmt) {
            match stmt.0 {
                StmtKind::Expr(ref expr) => walk_expr(visitor, expr),
                StmtKind::Do(ref block) => walk_block(visitor, &BlockType::Stmt(stmt), block),
                StmtKind::ForGeneric(_, ref exprs, ref block) => {
                    exprs.iter().for_each(|expr| walk_expr(visitor, expr));
                    walk_block(visitor, &BlockType::Stmt(stmt), block);
                }
                StmtKind::ForNumberic(_, ref expr1, ref expr2, ref expr3, ref block) => {
                    walk_expr(visitor, expr1);
                    walk_expr(visitor, expr2);
                    if let Some(expr3) = expr3 {
                        walk_expr(visitor, expr3);
                    }
                    walk_block(visitor, &BlockType::Stmt(stmt), block);
                }
                StmtKind::While(ref expr, ref block) => {
                    walk_expr(visitor, expr);
                    walk_block(visitor, &BlockType::Stmt(stmt), block);
                }

                StmtKind::Function(_, FuncBody(.., ref block), _) => {
                    walk_block(visitor, &BlockType::Stmt(stmt), block);
                }

                StmtKind::If(ref if_head, ref elseif, ref else_tail) => {
                    walk_expr(visitor, &if_head.1);
                    walk_block(visitor, &BlockType::Stmt(stmt), &if_head.0);
                    elseif.iter().for_each(|IfClause(block, expr)| {
                        walk_expr(visitor, expr);
                        walk_block(visitor, &BlockType::Stmt(stmt), block);
                    });
                    if let Some(block) = else_tail {
                        walk_block(visitor, &BlockType::Stmt(stmt), block);
                    }
                }
                StmtKind::Empty => (),
                StmtKind::Assign(_, ref exprs) => {
                    exprs.iter().for_each(|expr| walk_expr(visitor, expr));
                    visitor.visit_stmt(stmt);
                }
                StmtKind::Call(ref func, ref params) => {
                    walk_expr(visitor, func);
                    params.iter().for_each(|param| walk_expr(visitor, param));
                    visitor.visit_stmt(stmt);
                }
                StmtKind::Return(ref exprs) => {
                    exprs.iter().for_each(|expr| walk_expr(visitor, expr));
                    visitor.visit_stmt(stmt);
                }
                _ => visitor.visit_stmt(stmt),
            }
        }
        fn walk_expr(visitor: &mut Box<dyn Checker>, expr: &Expr) {
            if !visitor.should_visit_expr() {
                return;
            }
            match expr.0 {
                ExprKind::Unary(ref expr, _) => walk_expr(visitor, expr),
                ExprKind::Binary(ref expr1, ref expr2, _) => {
                    walk_expr(visitor, expr1);
                    walk_expr(visitor, expr2);
                }
                ExprKind::Call(ref expr1, ref expr2) => {
                    walk_expr(visitor, expr1);
                    expr2.iter().for_each(|expr| walk_expr(visitor, expr));
                }
                ExprKind::Access(ref path) => {
                    do_path(visitor, path);
                    fn do_path(visitor: &mut Box<dyn Checker>, path: &Path) {
                        match path {
                            Path::Base(..) => (),
                            Path::Chain(path, _) => do_path(visitor, path),
                            Path::Expr(expr) => walk_expr(visitor, expr),
                        }
                    }
                }
                ExprKind::Function(ref func) => {
                    walk_block(visitor, &BlockType::AnonymousFunc(func), &func.1);
                }
                _ => (),
            }
            visitor.visit_expr(expr)
        }

        let errors: Vec<DukaError> = self
            .checkers
            .iter_mut()
            .flat_map(|visitor| {
                walk_block(visitor, &BlockType::Global, chunk);
                visitor.report()
            })
            .collect();
        errors.is_empty().then_some(()).ok_or(errors)
    }

    pub fn transform(&mut self, chunk: &mut Block) {
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

                StmtKind::Function(_, FuncBody(.., ref mut block), _) => {
                    walk_block(transformer, block);
                    transformer.adapt_stmt(stmt);
                }

                StmtKind::If(ref mut if_head, ref mut elseif, ref mut else_tail) => {
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
                StmtKind::Assign(_, ref mut exprs) => {
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

        for transformer in self.transformers.iter_mut() {
            walk_block(transformer, chunk);
        }
    }
}

pub enum BlockType<'a> {
    Global,
    AnonymousFunc(&'a FuncBody),
    Stmt(&'a Stmt),
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
    fn adapt_expr(&mut self, _stmt: &mut Expr) {}

    #[inline]
    fn should_adapt_stmt(&self) -> bool {
        true
    }
    #[inline]
    fn should_adapt_expr(&self) -> bool {
        true
    }
}
