use crate::{
    frontend::{
        analyzer::{BlockType, Checker},
        ast::{Expr, ExprKind, Stmt, StmtKind},
    },
    shared::{
        error::{DukaError, DukaSemanticError},
        types::Spanned,
    },
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
        impl Checker for $name {
            $($visitor)+
            fn errors(&self) -> Vec<DukaError> {
                self.errors.clone()
            }
        }
    };
}

checker! {
    LoopChecker(loop_depth: usize = 0),
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

#[derive(Debug, PartialEq)]
enum ScopeType {
    Function,
    Do,
    Control,
    Global,
}
#[derive(Debug)]
struct LabelScope {
    labels: Vec<String>,
    scope_type: ScopeType,
}
impl LabelScope {
    pub fn new(scope_type: ScopeType) -> Self {
        LabelScope {
            labels: vec![],
            scope_type: scope_type,
        }
    }
}
struct LabelScopeManager {
    global: LabelScope,
    scopes: Vec<LabelScope>,
}
impl LabelScopeManager {
    pub fn new() -> Self {
        Self {
            global: LabelScope::new(ScopeType::Global),
            scopes: vec![],
        }
    }

    pub fn enter(&mut self, ty: ScopeType) {
        self.scopes.push(LabelScope::new(ty))
    }
    pub fn exit(&mut self) {
        self.scopes.pop();
    }

    pub fn push_label(&mut self, label: String) -> Result<(), ()> {
        let cur = self.current_mut();
        cur.labels
            .contains(&label)
            .then_some(Err(()))
            .unwrap_or_else(|| {
                cur.labels.push(label);
                Ok(())
            })
    }

    fn find_in_func(&mut self, label: &String) -> bool {
        self.scopes
            .iter()
            .rposition(|s| s.labels.contains(label) || matches!(s.scope_type, ScopeType::Function))
            .map(|i| self.scopes[i].labels.contains(label))
            .unwrap_or_else(|| self.global.labels.contains(label))
    }

    fn current_mut(&mut self) -> &mut LabelScope {
        self.scopes.last_mut().unwrap_or(&mut self.global)
    }
}

checker! {
    LabelChecker(
        scopes: LabelScopeManager = LabelScopeManager::new(),
        pending_goto: Vec<Vec<Spanned<String>>> = vec![]
    ),
    fn enter_block(&mut self, head: &BlockType) {
        if head.is_func() || head.is_global() {
            self.pending_goto.push(vec![]);
        }

        self.scopes.enter(match head {
            BlockType::Stmt(head) =>
                match head.0 {
                    StmtKind::If(..) |
                    StmtKind::ForNumberic(..) |
                    StmtKind::ForGeneric(..) |
                    StmtKind::While(..) => ScopeType::Control,

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
                if self.scopes.push_label(label.to_string()).is_err(){
                    self.errors.push(DukaError {
                        kind: DukaSemanticError::DuplicatedItem("label".to_string(), label.to_string()).into(),
                        span: stmt.1
                    });
                }
            }
            StmtKind::Goto(ref label) => {
                self.pending_goto.last_mut().unwrap().push((label.to_string(), stmt.1));
            }
            _ => ()
        }
    }
}
impl LabelChecker {
    fn check_pending_goto(&mut self) {
        self.pending_goto
            .pop()
            .unwrap()
            .into_iter()
            .for_each(|(label, span)| {
                if !self.scopes.find_in_func(&label) {
                    self.errors.push(DukaError {
                        kind: DukaSemanticError::InvisibleGotoLabel(label).into(),
                        span,
                    });
                }
            });
    }
}

checker! {
    VarArgChecker(marks: Vec<bool> = vec![]),
    fn visit_expr(&mut self, expr: &Expr) {
        if !matches!(expr.0, ExprKind::VarArg) {
            return
        }
        if matches!(self.marks.last(), Some(false)) {
            self.errors.push(DukaError {
                kind: DukaSemanticError::InvalidVarArg.into(),
                span: expr.1
            })
        }
    },
    fn enter_block(&mut self, head: &BlockType) {
        if let BlockType::Stmt(head) = head &&
            let StmtKind::Function(_, ref func, _) = head.0 {
            self.marks.push(func.has_vararg());
        }
    },
    fn exit_block(&mut self, head: &BlockType) {
        if head.is_func() {
            self.marks.pop();
        }
    }
}
