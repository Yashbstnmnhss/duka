use std::collections::HashMap;

use duka_frontend::{
    analyzer::{Visit, Visitor},
    parser::ast::{DukaChunk, Expr, ExprKind, Field, Path, PathSuffix, Stmt, StmtKind},
};
use duka_shared::{constants::MetaMethod, errors::Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    MethodCall,
    FieldAccess,
}

struct RoleCollector {
    roles: HashMap<Span, Role>,
}

pub fn collect(chunk: &DukaChunk) -> HashMap<Span, Role> {
    let mut collector = RoleCollector {
        roles: HashMap::new(),
    };
    chunk.visit(&mut collector);
    collector.roles
}

impl RoleCollector {
    fn mark_chain(&mut self, path: &Path, role: Role) {
        match path {
            Path::Chain(rest, suffix) => {
                if let PathSuffix::Dot((_, span)) | PathSuffix::Colon((_, span)) = suffix {
                    let entry = self.roles.entry(*span).or_insert(role);
                    if *entry == Role::FieldAccess && role == Role::MethodCall {
                        *entry = Role::MethodCall;
                    }
                }
                self.mark_chain(rest, Role::FieldAccess); // a.b.c() a.b()
            }
            _ => {}
        }
    }
}

impl Visitor for RoleCollector {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if let StmtKind::Call(func, _) = &stmt.0 {
            if let ExprKind::Access(path) = &func.0 {
                self.mark_chain(path, Role::MethodCall);
            }
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.0 {
            ExprKind::Call(target, _) => {
                if let ExprKind::Access(path) = &target.0 {
                    self.mark_chain(path, Role::MethodCall);
                }
            }
            ExprKind::Access(path) => self.mark_chain(path, Role::FieldAccess),
            ExprKind::Table(fields) => {
                for field in fields.iter() {
                    if let Field::NameValue((_, span), _) = field {
                        self.roles.entry(*span).or_insert(Role::FieldAccess);
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn is_metamethod(name: &str) -> bool {
    name == MetaMethod::Index.name()
        || name == MetaMethod::NewIndex.name()
        || name == MetaMethod::Gc.name()
        || name == MetaMethod::Mode.name()
        || name == MetaMethod::Len.name()
        || name == MetaMethod::Eq.name()
        || name == MetaMethod::Add.name()
        || name == MetaMethod::Sub.name()
        || name == MetaMethod::Mul.name()
        || name == MetaMethod::Mod.name()
        || name == MetaMethod::Pow.name()
        || name == MetaMethod::Div.name()
        || name == MetaMethod::IDiv.name()
        || name == MetaMethod::BAnd.name()
        || name == MetaMethod::BOr.name()
        || name == MetaMethod::BXor.name()
        || name == MetaMethod::ShL.name()
        || name == MetaMethod::ShR.name()
        || name == MetaMethod::Unm.name()
        || name == MetaMethod::BNot.name()
        || name == MetaMethod::LT.name()
        || name == MetaMethod::LE.name()
        || name == MetaMethod::Concat.name()
        || name == MetaMethod::Call.name()
        || name == MetaMethod::Close.name()
        || name == MetaMethod::ToString.name()
}
