use crate::{
    frontend::ast::{Block, FuncBody, IfClause, Stmt, StmtKind},
    shared::{
        error::{DukaError, DukaSemanticError},
        types::Spanned,
    },
};

pub struct Walker {
    visitors: Vec<Box<dyn Visitor>>,
}

impl Walker {
    pub fn new() -> Self {
        Self { visitors: vec![] }
    }

    pub fn add<T: Visitor + 'static>(&mut self, visitor: T) -> &mut Self {
        self.visitors.push(Box::new(visitor));
        self
    }

    pub fn walk_chunk(&mut self, chunk: &Block) -> Result<(), Vec<DukaError>> {
        fn walk_block(
            visitor: &mut Box<dyn Visitor>,
            head: &Stmt,
            block: &Block,
        ) -> Vec<Result<(), DukaError>> {
            let mut result = vec![visitor.enter_block(head)];
            result.extend(block.0.iter().flat_map(|stmt| walk(visitor, stmt)));
            result.push(visitor.exit_block(head));
            result
        }
        fn walk(visitor: &mut Box<dyn Visitor>, stmt: &Stmt) -> Vec<Result<(), DukaError>> {
            match stmt.0 {
                StmtKind::Do(ref block)
                | StmtKind::ForGeneric(.., ref block)
                | StmtKind::ForNumberic(.., ref block)
                | StmtKind::While(.., ref block)
                | StmtKind::Function(_, FuncBody(.., ref block), _) => {
                    walk_block(visitor, stmt, block)
                }

                StmtKind::If(ref if_head, ref elseif, ref else_tail) => {
                    let mut result = walk_block(visitor, stmt, &if_head.0);
                    result.extend(
                        elseif
                            .iter()
                            .flat_map(|IfClause(block, ..)| walk_block(visitor, stmt, block)),
                    );
                    if let Some(block) = else_tail {
                        result.extend(walk_block(visitor, stmt, block))
                    }
                    result
                }
                _ => vec![visitor.visit_stmt(stmt)],
            }
        }

        let errors = self
            .visitors
            .iter_mut()
            .flat_map(|visitor| {
                let mut result = vec![visitor.enter()];
                result.extend(chunk.0.iter().flat_map(|stmt| walk(visitor, stmt)));
                result.push(visitor.exit());
                result
            })
            .fold(vec![], |mut errs, item| {
                if let Err(e) = item {
                    errs.push(e)
                }
                errs
            });
        errors.is_empty().then_some(()).ok_or(errors)
    }
}

pub trait Visitor {
    fn visit_stmt(&mut self, _stmt: &Stmt) -> Result<(), DukaError> {
        Ok(())
    }
    fn enter(&mut self) -> Result<(), DukaError> {
        Ok(())
    }
    fn exit(&mut self) -> Result<(), DukaError> {
        Ok(())
    }
    fn enter_block(&mut self, _head: &Stmt) -> Result<(), DukaError> {
        Ok(())
    }
    fn exit_block(&mut self, _head: &Stmt) -> Result<(), DukaError> {
        Ok(())
    }
}

macro_rules! visitor {
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
        impl Visitor for $name {
            $($visitor)+
        }
    };
}

visitor! {
    LoopVisitor(loop_depth: usize = 0),
    fn enter_block(&mut self, head: &Stmt) -> Result<(), DukaError> {
        if matches!(head.0,
            StmtKind::ForGeneric(..) |
            StmtKind::ForNumberic(..) |
            StmtKind::While(..)
        ) {
            self.loop_depth += 1;
        }
        Ok(())
    },
    fn exit_block(&mut self, head: &Stmt) -> Result<(), DukaError> {
        if matches!(head.0,
            StmtKind::ForGeneric(..) |
            StmtKind::ForNumberic(..) |
            StmtKind::While(..)
        ) {
            self.loop_depth -= 1;
        }
        Ok(())
    },
    fn visit_stmt(&mut self, stmt: &Stmt) -> Result<(), DukaError> {
        if matches!(stmt.0, StmtKind::Break | StmtKind::Continue) && self.loop_depth == 0 {
            Err(DukaError {
                span: stmt.1,
                kind: DukaSemanticError::InvalidLoopFlowControl.into()
            })
        } else {
            Ok(())
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

visitor! {
    LabelVisitor(
        scopes: LabelScopeManager = LabelScopeManager::new(),
        pending_goto: Vec<Vec<Spanned<String>>> = vec![]
    ),
    fn enter(&mut self) -> Result<(), DukaError> {
        self.pending_goto.push(vec![]);
        Ok(())
    },
    fn enter_block(&mut self, head: &Stmt) -> Result<(), DukaError> {
        if matches!(head.0, StmtKind::Function(..)) {
            self.pending_goto.push(vec![]);
        }

        self.scopes.enter(match head.0 {
            StmtKind::If(..) |
            StmtKind::ForNumberic(..) |
            StmtKind::ForGeneric(..) |
            StmtKind::While(..) => ScopeType::Control,

            StmtKind::Function(..) => ScopeType::Function,
            StmtKind::Do(..) => ScopeType::Do,

            _ => unreachable!()
        });
        Ok(())
    },
    fn exit(&mut self) -> Result<(), DukaError> {
        self.check_pending_goto()
    },
    fn exit_block(&mut self, head: &Stmt) -> Result<(), DukaError> {
        if matches!(head.0, StmtKind::Function(..)) {
            self.check_pending_goto()?;
        }
        self.scopes.exit();

        Ok(())
    },
    fn visit_stmt(&mut self, stmt: &Stmt) -> Result<(), DukaError> {
        match stmt.0 {
            StmtKind::Label(ref label) =>
                self.scopes.push_label(label.to_string())
                    .map_err(|_| DukaSemanticError::DuplicatedItem("label".to_string(), label.to_string())),
            StmtKind::Goto(ref label) => {
                self.pending_goto.last_mut().unwrap().push((label.to_string(), stmt.1));
                Ok(())
            }

            _ => Ok(())
        }
        .map_err(|kind| DukaError {
            kind: kind.into(),
            span: stmt.1
        })
    }
}
impl LabelVisitor {
    fn check_pending_goto(&mut self) -> Result<(), DukaError> {
        self.pending_goto
            .pop()
            .unwrap()
            .into_iter()
            .try_for_each(|(label, span)| {
                self.scopes
                    .find_in_func(&label)
                    .then_some(())
                    .ok_or_else(|| DukaError {
                        kind: DukaSemanticError::InvisibleGotoLabel(label).into(),
                        span,
                    })
            })
    }
}
