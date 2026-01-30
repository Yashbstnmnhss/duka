use std::collections::HashSet;

use duka_macros::Info;
use duka_shared::{
    ast::{BinOp, UnOp},
    error::{DukaCodegenError, DukaCodegenErrorKind},
    types::LogicDatabase,
    utils::UniqueVec,
    value::{ConstValue, DukaFloat, DukaInt},
};

use crate::{
    DebugInfo,
    instructions::{Address, Instruction as I},
    value::{UpIndex, UpValueKind},
};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Constants(UniqueVec<ConstValue>);
impl Constants {
    pub fn add(&mut self, val: ConstValue) -> usize {
        self.0.push(val)
    }
    pub fn into_vec(self) -> Vec<ConstValue> {
        self.0.into_vec()
    }
}

#[derive(Debug, Clone, PartialEq, Info)]
pub enum AllocIdx {
    /// this is pointing to registers index
    #[tag(r)]
    R(Reg),
    /// this is pointing to constants index
    #[tag(k)]
    K(Cst),
    /// this is pointing to index of up_vals vector in scope
    #[tag(u)]
    U(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Function {
        locals: Vec<(String, usize)>,
        consts: Vec<(String, usize)>,
        up_vals: Vec<(String, UpIndex)>,
    },
    Block {
        locals: Vec<(String, usize)>,
        consts: Vec<(String, usize)>,
    },
}
impl Scope {
    fn find_existed(&self, name: &str) -> Option<AllocIdx> {
        match self {
            Self::Function {
                locals,
                consts,
                up_vals,
            } => consts
                .iter()
                .rev()
                .find_map(|(n, i)| (name == n).then_some(AllocIdx::K(*i)))
                .or_else(|| {
                    locals
                        .iter()
                        .rev()
                        .find_map(|(n, i)| (name == n).then_some(AllocIdx::R(*i)))
                        .or_else(|| {
                            up_vals
                                .iter()
                                .rposition(|(n, _)| name == n)
                                .map(AllocIdx::U)
                        })
                }),
            Self::Block { locals, consts } => consts
                .iter()
                .rev()
                .find_map(|(n, i)| (name == n).then_some(AllocIdx::K(*i)))
                .or_else(|| {
                    locals
                        .iter()
                        .find_map(|(n, i)| (name == n).then_some(AllocIdx::R(*i)))
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::types::{AllocIdx, Scopes};

    #[test]
    fn test_scope() {
        let mut scope = Scopes::new();
        scope.enter(true); //main

        scope.declare_const("a", 1);

        scope.enter(true); // function
        scope.enter(false); // block in function
        scope.enter(true); // inner function in block

        assert_eq!(scope.find("a"), Some(AllocIdx::K(1)));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scopes {
    scopes: Vec<Scope>,
    functions: Vec<usize>,
}
impl Scopes {
    /// Notice, there are none scopes, so for global scope, you also need to call `enter`
    pub fn new() -> Self {
        // NO, NO NEED FOR GLOBAL PLEASE
        Self {
            scopes: vec![/*Scope::default()*/],
            functions: vec![],
        }
    }
    fn current(&self) -> &Scope {
        assert!(self.len() >= 1);
        self.scopes.last().unwrap()
    }
    fn current_mut(&mut self) -> &mut Scope {
        assert!(self.len() >= 1);
        self.scopes.last_mut().unwrap()
    }
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    pub fn declare_local(&mut self, name: &str, reg: usize) {
        let locals = match self.current_mut() {
            Scope::Block { locals, .. } => locals,
            Scope::Function { locals, .. } => locals,
        };
        locals.push((name.to_string(), reg));
    }
    pub fn declare_const(&mut self, name: &str, idx: usize) {
        let consts = match self.current_mut() {
            Scope::Block { consts, .. } => consts,
            Scope::Function { consts, .. } => consts,
        };
        consts.push((name.to_string(), idx));
    }

    pub fn find(&mut self, name: &str) -> Option<AllocIdx> {
        assert!(self.len() >= 1);

        let mut upval_mode = false;
        let mut chain = vec![];
        for (idx, scope) in self.scopes.iter().enumerate().rev() {
            let find = scope.find_existed(name);

            if upval_mode {
                if let Some(ai) = find {
                    fn create_upval(f: &mut Scope, n: &str, l: bool, i: usize) -> usize {
                        let Scope::Function { up_vals, .. } = f else {
                            unreachable!();
                        };
                        up_vals.push((
                            n.to_string(),
                            UpIndex {
                                name: Some(n.to_string()),
                                local: l,
                                index: i,
                                kind: UpValueKind::Regular,
                            },
                        ));
                        return up_vals.len() - 1;
                    }

                    match ai {
                        AllocIdx::R(n) | AllocIdx::U(n) => {
                            let mut i: usize = 0;
                            let mut final_idx: usize = 0; // I mean, this wouldn't just be it
                            for idx in chain.into_iter().rev() {
                                let f = self.scopes.get_mut(idx).unwrap();
                                final_idx = create_upval(
                                    f,
                                    name,
                                    i == 0 && matches!(ai, AllocIdx::R(..)),
                                    (i == 0).then_some(n).unwrap_or(final_idx),
                                );
                                i += 1;
                            }
                            return Some(AllocIdx::U(final_idx));
                        }
                        r @ AllocIdx::K(..) => return Some(r), // <const> 直接返回
                    }
                } else if matches!(scope, Scope::Function { .. }) {
                    chain.push(idx);
                }
            } else if find.is_some() {
                return find;
            }

            // 超出父函数边界, 涉及upvalue
            if !upval_mode && matches!(scope, Scope::Function { .. }) {
                upval_mode = true;
                chain.push(idx);
            }
        }

        None
    }

    /// NOTICE, YOU MUST HAVE AT LEAST ONE FUNCTION SCOPE BEFORE ENTERING A BLOCK SCOPE!
    pub fn enter(&mut self, is_func: bool) {
        if is_func {
            self.functions.push(self.len());
        }

        self.scopes.push(
            is_func
                .then_some(Scope::Function {
                    locals: vec![],
                    consts: vec![],
                    up_vals: vec![],
                })
                .unwrap_or(Scope::Block {
                    locals: vec![],
                    consts: vec![],
                }),
        );
    }
    pub fn exit(&mut self) {
        assert!(self.len() > 1);
        if self.len() > 1 && matches!(self.scopes.pop().unwrap(), Scope::Function { .. }) {
            self.functions.pop();
        }
    }
}

#[derive(Debug)]
pub struct Allocator {
    snapshots: Vec<AllocatorSnapshot>,
    current: AllocatorSnapshot,
}

/// into a function prototype
#[derive(Debug, Default)]
pub struct AllocatorSnapshot {
    top: usize,
    free_list: Vec<usize>,
    allocated: HashSet<usize>,
}

impl Allocator {
    pub fn new() -> Self {
        Self {
            snapshots: vec![],
            current: AllocatorSnapshot::default(),
        }
    }
    pub fn enter(&mut self) {
        let snapshot = std::mem::take(&mut self.current);
        self.snapshots.push(snapshot);
    }
    pub fn exit(&mut self) {
        if let Some(cur) = self.snapshots.pop() {
            self.current = cur
        }
    }
    // this has infinite registers NO!
    pub fn alloc(&mut self) -> usize {
        let idx = self.current.free_list.pop().unwrap_or_else(|| {
            let res = self.current.top;
            self.current.top += 1;
            res
        });
        self.current.allocated.insert(idx);
        idx
    }

    pub fn used_reg_count(&self) -> usize {
        self.current.allocated.len() + self.current.free_list.len()
    }

    pub fn free(&mut self, idx: usize) {
        if self.current.allocated.remove(&idx) {
            self.current.free_list.push(idx)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaIR {
    pub irs: Vec<IR>,
    pub constants: Constants,
    pub scopes: Scopes,
    pub debug_info: DebugInfo,
    pub logic: Option<LogicDatabase>,
}
pub type Reg = usize;
pub type Cst = usize;
#[derive(Debug, Clone, PartialEq)]
pub enum IR {
    Void,

    // Constants
    /// load nil to reg
    LoadNil(Reg),
    /// load true to reg
    LoadTrue(Reg),
    /// load false to reg
    LoadFalse(Reg),
    /// load constants[cst] to reg
    LoadConst(Reg, Cst),
    /// load float to reg
    LoadFloat(Reg, DukaFloat),
    /// load int to reg
    LoadInt(Reg, DukaInt),
    /// load str to reg
    LoadString(Reg, Vec<u8>),

    // Table-related
    GetByKey(Reg, AllocIdx, AllocIdx),
    SetByKey(AllocIdx, AllocIdx, AllocIdx),

    GetUpVal(Reg, usize),
    SetUpVal(usize, AllocIdx),

    NewTable(Reg),

    // function-related
    Param(Reg),
    SelF(),
    Call(Reg, usize, bool),
    Closure(Reg, usize),
    Return(),
    VarArg(Reg, usize),

    // coroutine
    Spawn(Reg),
    Go(Reg),
    Yield(),

    // arithmetic
    Unary(Reg, AllocIdx, UnOp),
    Binary(Reg, AllocIdx, AllocIdx, BinOp),

    // control flow
    Jump(i32),
}
