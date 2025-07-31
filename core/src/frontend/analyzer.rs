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
                visitor.errors()
            })
            .collect();
        errors.is_empty().then_some(()).ok_or(errors)
    }

    pub fn transform(&mut self, chunk: &mut Block) {
        for transformer in self.transformers.iter_mut() {
            for stmt in chunk.0.iter_mut() {
                transformer.adapt_stmt(stmt);
            }
        }
    }
}

pub enum BlockType<'a> {
    Global,
    AnonymousFunc(&'a FuncBody),
    Stmt(&'a Stmt),
}
impl BlockType<'_> {
    pub const fn is_func(&self) -> bool {
        matches!(
            self,
            Self::Stmt((StmtKind::Function(..), _)) | Self::AnonymousFunc(..)
        )
    }
    pub const fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }
}

pub trait Checker {
    fn visit_stmt(&mut self, _stmt: &Stmt) {}
    fn visit_expr(&mut self, _expr: &Expr) {}
    fn enter_block(&mut self, _ty: &BlockType) {}
    fn exit_block(&mut self, _ty: &BlockType) {}
    fn errors(&self) -> Vec<DukaError>;
}

pub trait Transformer {
    fn adapt_stmt(&mut self, _stmt: &mut Stmt) {}
    fn adapt_expr(&mut self, _stmt: &mut Expr) {}
}
