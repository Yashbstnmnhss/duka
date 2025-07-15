use crate::shared::value::Value;

pub trait Visitor<T> {
    fn visit_expr(&mut self, expr: &Expr) -> T;
    fn visit_stmt(&mut self, stmt: &Stmt) -> T;
}

#[derive(Debug, PartialEq)]
pub enum Stmt {
    Expr(Expr),
    If {
        if_block: IfStmt,
        elseif_blocks: Option<Vec<IfStmt>>,
        else_block: Option<BlockStmt>,
    },
}

#[derive(Debug, PartialEq)]
pub struct IfStmt {
    pub block: BlockStmt,
    pub condition: Expr,
}

#[derive(Debug, PartialEq)]
pub struct BlockStmt {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, PartialEq)]
pub enum Expr {
    Vararg {},
    Literal { value: Value },
    Ident { name: String },
    Call { callee: Box<Expr>, args: Vec<Expr> },
}
