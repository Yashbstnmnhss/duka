use std::collections::HashSet;

use duka_macros::Info;
use duka_shared::{
    ast::{BinOp, UnOp},
    constants::{cgen, cvm},
    error::{DukaCodegenError, DukaCodegenErrorKind},
    types::{LogicDatabase, SysCall},
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

///## Returned by expression, it could represent an already-allocated value or immediate operands or to-be-allocated values
#[derive(Debug, Clone, PartialEq)]
pub enum Desc {
    /// # This is *already* allocated in registers, constants pool or upvalues,
    /// which is unable to be relocated further
    NonReloc(Place),
    /// # This represents maybe several *to-be*-allocated values with their count
    /// which needs to further confirm,
    /// such as in
    /// - returning
    /// - assign
    Pending(usize),
    /// # This contains a `ConstValue` as an immediate operand
    /// which can be confirmed go whether register or constants pool or just encoding into instructions
    Immediate(ConstValue),
}

///## Things that are already allocated in registers or constants pool or upvalues
#[derive(Debug, Clone, PartialEq, Info)]
pub enum Place {
    /// this is pointing to registers index
    #[tag(store)]
    R(Reg),
    /// this is pointing to constants index
    #[tag(store)]
    K(Cst),
    /// this is pointing to index of up_vals vector in scope
    #[tag(store)]
    U(usize),
}

impl From<Vec<Reg>> for Reg {
    fn from(mut value: Vec<Reg>) -> Self {
        if value.len() == 1 {
            value.pop().unwrap()
        } else {
            let start = value
                .iter()
                .map(|i| match i {
                    Reg::Idx(u) => *u,
                    Reg::Many(u, ..) => *u,
                })
                .min()
                .expect("Already checked, this won't happened");
            let len = value.iter().fold(0usize, |mut acc, i| {
                match i {
                    Reg::Idx(..) => acc += 1,
                    Reg::Many(.., len) => acc += *len,
                }
                acc
            });
            Reg::Many(start, len)
        }
    }
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
    fn declare_upval(&mut self, name: &str, up_idx: UpIndex) {
        if let Scope::Function { up_vals, .. } = self {
            up_vals.push((name.to_string(), up_idx));
        }
    }
    fn declare_const(&mut self, name: &str, idx: usize) {
        let consts = match self {
            Scope::Block { consts, .. } => consts,
            Scope::Function { consts, .. } => consts,
        };
        consts.push((name.to_string(), idx));
    }
    fn declare_local(&mut self, name: &str, reg: usize) {
        let locals = match self {
            Scope::Block { locals, .. } => locals,
            Scope::Function { locals, .. } => locals,
        };
        locals.push((name.to_string(), reg));
    }
    fn find_upval(&self, name: &str) -> Option<usize> {
        if let Self::Function { up_vals, .. } = self {
            up_vals.iter().rposition(|(n, _)| name == n)
        } else {
            None
        }
    }
    fn find_existed(&self, name: &str) -> Option<Place> {
        match self {
            Self::Function { locals, consts, .. } => consts
                .iter()
                .rev()
                .find_map(|(n, i)| (name == n).then_some(Place::K(*i)))
                .or_else(|| {
                    locals
                        .iter()
                        .rev()
                        .find_map(|(n, i)| (name == n).then_some(Place::R(Reg::Idx(*i))))
                        .or_else(|| self.find_upval(name).map(Place::U))
                }),
            Self::Block { locals, consts } => consts
                .iter()
                .rev()
                .find_map(|(n, i)| (name == n).then_some(Place::K(*i)))
                .or_else(|| {
                    locals
                        .iter()
                        .find_map(|(n, i)| (name == n).then_some(Place::R(Reg::Idx(*i))))
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::types::{Place, Scopes};

    #[test]
    fn test_scope() {
        let mut scope = Scopes::new();
        scope.enter(true); //main

        scope.declare_const("a", 1);

        scope.enter(true); // function
        scope.enter(false); // block in function
        scope.enter(true); // inner function in block

        assert_eq!(scope.find("a"), Some(Place::K(1)));
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
    #[inline]
    fn current(&self) -> &Scope {
        assert!(self.len() >= 1);
        self.scopes.last().unwrap()
    }
    #[inline]
    fn current_mut(&mut self) -> &mut Scope {
        assert!(self.len() >= 1);
        self.scopes.last_mut().unwrap()
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    pub fn ensure_global(&mut self) -> Place {
        assert!(self.len() >= 1);
        self.find(cgen::GLOBAL).unwrap_or_else(|| {
            let main = self.scopes.first_mut().unwrap();
            main.declare_upval(
                cgen::GLOBAL,
                UpIndex {
                    name: Some(cgen::GLOBAL.to_owned()),
                    local: true,
                    index: cgen::ENV_UPVAL_IDX,
                    kind: UpValueKind::Regular,
                },
            );
            self.find(cgen::GLOBAL).expect("MUST HAVE BRO!")
        })
    }

    #[inline]
    pub fn declare_upval(&mut self, name: &str, up_idx: UpIndex) {
        self.current_mut().declare_upval(name, up_idx);
    }
    #[inline]
    pub fn declare_local(&mut self, name: &str, reg: usize) {
        self.current_mut().declare_local(name, reg);
    }
    #[inline]
    pub fn declare_const(&mut self, name: &str, idx: usize) {
        self.current_mut().declare_const(name, idx);
    }

    /// # Panic
    /// Please ensure that func is a function scope
    fn create_upval_unchecked(func: &mut Scope, name: &str, is_local: bool, idx: usize) -> usize {
        let Scope::Function { up_vals, .. } = func else {
            unreachable!();
        };
        up_vals.push((
            name.to_string(),
            UpIndex {
                name: Some(name.to_string()),
                local: is_local,
                index: idx,
                kind: UpValueKind::Regular,
            },
        ));
        return up_vals.len() - 1;
    }
    pub fn find(&mut self, name: &str) -> Option<Place> {
        assert!(self.len() >= 1);

        let mut upval_mode = false;
        let mut chain = vec![];
        for (idx, scope) in self.scopes.iter().enumerate().rev() {
            let find = scope.find_existed(name);

            if upval_mode {
                if let Some(ai) = find {
                    match ai {
                        Place::R(Reg::Idx(n)) | Place::U(n) => {
                            let mut i: usize = 0;
                            let mut idx: usize = n;
                            for func_idx in chain.into_iter().rev() {
                                let f = self.scopes.get_mut(func_idx).unwrap();
                                idx = Self::create_upval_unchecked(
                                    f,
                                    name,
                                    i == 0 && matches!(ai, Place::R(..)),
                                    idx,
                                );
                                i += 1;
                            }
                            return Some(Place::U(idx));
                        }
                        r @ Place::K(..) => return Some(r), // <const> 直接返回
                        _ => panic!("Variable cannot be vararg registers"),
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
    pub fn alloc_one(&mut self) -> usize {
        let idx = self.current.free_list.pop().unwrap_or_else(|| {
            let res = self.current.top;
            self.current.top += 1;
            res
        });
        self.current.allocated.insert(idx);
        idx
    }
    // this has infinite registers? NO!
    pub fn alloc(&mut self) -> Reg {
        Reg::Idx(self.alloc_one())
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reg {
    Idx(usize),
    Many(usize, usize),
}

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
    GetField(Reg, Place, Place),
    SetField(Place, Place, Place),
    SetArray(Place, Reg),

    GetUpVal(Reg, usize),
    SetUpVal(usize, Place),

    NewTable(Reg),

    // function-related
    Param(Reg),
    SelF(),
    Call(Reg, usize, bool),
    Closure(Reg, DukaIR),
    Return(),
    VarArg(Reg, usize),

    // coroutine
    Spawn(Reg),
    Go(Reg),
    Yield(),

    // arithmetic
    Unary(Reg, Place, UnOp),
    Binary(Reg, Place, Place, BinOp),

    // control flow
    Jump(i32),

    SysCall(SysCall),
}
