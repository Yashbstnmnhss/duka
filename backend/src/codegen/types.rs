use std::{collections::HashSet, fmt::Display};

use duka_macros::Info;
use duka_shared::{
    ast::{BinOp, UnOp},
    constants::cgen,
    types::{LogicDatabase, SysCall},
    utils::UniqueVec,
    value::{ConstValue, DukaFloat, DukaInt},
};

use crate::{
    DebugInfo,
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
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}

impl ExpDesc {
    pub(crate) fn from_regs(regs: Vec<Reg>) -> Self {
        if let [first, ..] = regs.as_slice() {
            ExpDesc::Single(Place::R(*first))
        } else {
            ExpDesc::Many(regs, None)
        }
    }
}

///## Returned by expression, it could represent an already-allocated value or immediate operands or to-be-allocated values
#[derive(Debug, Clone, PartialEq)]
pub enum ExpDesc {
    /// # This is *already* allocated in registers, constants pool or upvalues,
    /// which will be passed directly to **instruction codegen** for further relocation
    Single(Place),

    /// # This represents maybe several *to-be*-allocated values with their count
    /// ## This should be used with a __`Take` instruction__ after current instruction
    /// which needs to further confirm,
    /// such as in
    /// - `return a, b, c, ...` -> uncertain
    /// - `return a, b, c` -> exact(3)
    /// - `local a = 1, 2, 3` -> exact(3)
    /// - `local b, c = d, ...` -> uncertain
    ///
    /// # Params
    /// 1. Fixed values' register
    /// 2. VarArg value's register (optional)
    ///
    Many(Vec<Reg>, /*vararg*/ Option<Reg>),
    /// # This contains a `ConstValue` as an immediate operand (almost `DukaInt`)
    /// which can be confirmed go whether register or constants pool or just encoding into instructions.
    /// See instructions ends with letter `I`
    Immediate(ConstValue),
}

///## Things that are already allocated in registers or constants pool or upvalues
#[derive(Debug, Clone, PartialEq, Info)]
#[shy]
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
impl Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::R(r) => write!(f, "R[{r}]"),
            Place::K(k) => write!(f, "Consts[{k}]"),
            Place::U(u) => write!(f, "UpVals[{u}]"),
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
                        .find_map(|(n, i)| (name == n).then_some(Place::R(*i)))
                        .or_else(|| self.find_upval(name).map(Place::U))
                }),
            Self::Block { locals, consts } => consts
                .iter()
                .rev()
                .find_map(|(n, i)| (name == n).then_some(Place::K(*i)))
                .or_else(|| {
                    locals
                        .iter()
                        .find_map(|(n, i)| (name == n).then_some(Place::R(*i)))
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
                        Place::R(n) | Place::U(n) => {
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
                                                            // _ => panic!("Variable cannot be vararg registers"),
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
        assert!(self.len() >= 1);
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
    top: Reg,
    free_list: Vec<Reg>,
    allocated: HashSet<Reg>,
}

impl Allocator {
    pub fn new() -> Self {
        Self {
            snapshots: vec![],
            current: AllocatorSnapshot::default(),
        }
    }
    pub const fn top(&self) -> Reg {
        self.current.top
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

    /// Free registers
    pub fn free_many(&mut self, regs: impl Iterator<Item = Reg>) {
        for who in regs {
            if who >= self.top() {
                break;
            }
            self.free(who);
        }
    }
    // /// Ensure the register is already allocated
    // pub fn ensure(&mut self, who: Reg) {
    //     if !self.current.allocated.contains(&who) {
    //         self.alloc_to(who + 1).all(|_| false);
    //     }
    // }
    /// Allocate some registers range to a certain register(exclusive), returns them
    pub fn alloc_to(&mut self, to_top: Reg) -> impl Iterator<Item = Reg> {
        (0..to_top - self.current.top).map(|_| self.alloc())
    }
    /// this has infinite registers? NO!
    pub fn alloc(&mut self) -> Reg {
        let idx = self.current.free_list.pop().unwrap_or_else(|| {
            let res = self.current.top;
            self.current.top += 1;
            res
        });
        self.current.allocated.insert(idx);
        idx
    }

    /// # For those who needs intermediate storage
    pub fn alloc_temp(&mut self) -> Reg {
        // allocate it, its life ends at next allocation
        self.current.top
    }

    pub fn used_reg_count(&self) -> usize {
        self.current.allocated.len() + self.current.free_list.len()
    }

    pub fn free(&mut self, who: Reg) {
        if self.current.allocated.remove(&who) {
            self.current.free_list.push(who)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaIR {
    pub param_count: usize,
    pub has_var_arg: bool,

    pub instructions: Vec<IR>,
    pub nesteds: Vec<DukaIR>,
    pub constants: Constants,
    pub scopes: Scopes,
    pub debug_info: DebugInfo,
    pub logic: Option<LogicDatabase>,
}

impl Display for DukaIR {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{} input.duka:({}) [{} instructions]",
            self.debug_info
                .debug_name
                .clone()
                .unwrap_or("...".to_owned()),
            self.debug_info.all_span,
            self.instructions.len()
        )?;
        writeln!(
            f,
            "{} params (vararg: {}), {} consts, {} nesteds",
            self.param_count,
            self.has_var_arg,
            self.constants.len(),
            self.nesteds.len()
        )?;
        for (i, ins) in self.instructions.iter().enumerate() {
            let span = self
                .debug_info
                .inst_spans
                .iter()
                .find_map(|(r, s)| r.contains(&i).then_some(*s))
                .unwrap_or_default();
            write!(f, "[{i:0>2}]:({span})  ")?;
            write!(f, ".{} ", ins)?;
            match ins {
                IR::Void => (),

                IR::Move(to, from) => writeln!(f, "R[{to}] <- R[{from}]")?,
                IR::LoadNil(to) => writeln!(f, "R[{to}] <- nil")?,
                IR::LoadTrue(to) => writeln!(f, "R[{to}] <- true")?,
                IR::LoadFalse(to) => writeln!(f, "R[{to}] <- false")?,
                IR::LoadConst(to, k) => writeln!(f, "R[{to}] <- Consts[{k}]")?,
                IR::LoadFloat(to, fv) => writeln!(f, "R[{to}] <- {fv}f")?,
                IR::LoadInt(to, i) => writeln!(f, "R[{to}] <- {i}i")?,
                IR::LoadString(to, str) => writeln!(f, "R[{to}] <- {str:?}str")?,

                IR::GetField(to, tab, key) => writeln!(f, "R[{to}] <- {tab}.get({key})")?,
                IR::SetField(to, key, val) => writeln!(f, "{to}.set({key} := {val})")?,
                IR::SetIndexed(to, idx, val) => writeln!(f, "{to}.set([{idx}] := {val})")?,
                IR::NewTable(n) => writeln!(f, "R[{n}] <- {{}}")?,
                IR::Array(place, items) => writeln!(f, "{place}.pushes({items:?})")?,

                IR::GetUpVal(to, who) => writeln!(f, "R[{to}] <- UpVals[{who}]")?,
                IR::SetUpVal(who, place) => writeln!(f, "UpVals[{who}] <- {place}")?,

                IR::SelfParam() => writeln!(f, "%next call%")?,
                IR::Call(who, params) => writeln!(
                    f,
                    "R[{who}](params: {params}) %from R[{}] to R[{}]%",
                    *who + 1,
                    who + params
                )?,
                IR::TailCall(who, params) => writeln!(
                    f,
                    "R[{who}](params: {params}) %from R[{}] to R[{}]%, tailcall",
                    *who + 1,
                    who + params
                )?,
                IR::Closure(to, cls) => writeln!(
                    f,
                    "R[{to}] <- nesteds#{cls} %{}%",
                    self.nesteds
                        .get(*cls)
                        .unwrap()
                        .debug_info
                        .debug_name
                        .clone()
                        .unwrap_or("...".to_owned())
                )?,
                IR::Return(_) => writeln!(f, "")?,
                IR::VarArg(to) => writeln!(f, "R[{to}] <- ...")?,
                IR::Spawn(c) => writeln!(f, "coroutine#{c}")?,
                IR::Go(c) => writeln!(f, "coroutine#{c}")?,
                IR::Yield() => writeln!(f, "")?,
                IR::Unary(to, place, un_op) => writeln!(f, "R[{to}] <- |{un_op}| {place}")?,
                IR::Binary(to, place, place1, bin_op) => {
                    writeln!(f, "R[{to}] <- {place} |{bin_op}| {place1}")?
                }
                IR::BinaryI(to, place, int, bin_op) => {
                    writeln!(f, "R[{to}] <- {place} |{bin_op}| {int}i")?
                }
                IR::BinaryI2(to, int, place, bin_op) => {
                    writeln!(f, "R[{to}] <- {int}i |{bin_op}| {place}")?
                }
                IR::Jump(to) => writeln!(f, "to [{:0>2}]", to + (i as i32))?,
                IR::SkipNext(cond, to) => writeln!(f, "R[{cond}] is {to} ?: to [{:0>2}]", i + 2)?,
                IR::Dead(who) => writeln!(f, "R[{who}]")?,
                IR::Take(num) => writeln!(f, "{num} %for [{:0>2}]%", i - 1)?,
                IR::TakeAll => writeln!(f, "all %for [{:0>2}]%", i - 1)?,
                IR::SysCall(sys_call) => writeln!(f, "@{sys_call:?}")?,
            }
        }

        writeln!(f)?;
        writeln!(f, "- Consts:")?;
        for (i, val) in self.constants.clone().into_vec().into_iter().enumerate() {
            writeln!(f, ".[{i:0>2}] {val}")?;
        }
        writeln!(f, "{:=>9}", "=")?;
        writeln!(f)?;
        writeln!(f, "- Nesteds:")?;
        for (i, nested) in self.nesteds.iter().enumerate() {
            writeln!(f, "{:->9}", "-")?;
            writeln!(f, "#{i:0>2}:")?;
            writeln!(f, "{nested}")?;
        }

        Ok(())
    }
}

pub type Reg = usize;
pub type Cst = usize;

#[derive(Debug, Clone, PartialEq, Info)]
pub enum IR {
    Void,

    Move(Reg, Reg),

    // Constants
    /// load nil to reg
    #[tag(load)]
    LoadNil(Reg),
    /// load true to reg
    #[tag(load)]
    LoadTrue(Reg),
    /// load false to reg
    #[tag(load)]
    LoadFalse(Reg),
    /// load constants[cst] to reg
    #[tag(load)]
    LoadConst(Reg, Cst),
    /// load float to reg
    #[tag(load)]
    LoadFloat(Reg, DukaFloat),
    /// load int to reg
    #[tag(load)]
    LoadInt(Reg, DukaInt),
    /// load str to reg
    #[tag(load)]
    LoadString(Reg, Vec<u8>),

    // Table-related
    #[tag(table)]
    GetField(Reg, Place, Place),
    #[tag(table)]
    SetField(Place, Place, Place),
    #[tag(table)]
    SetIndexed(Place, usize, Place),
    #[tag(table)]
    NewTable(Reg),
    #[tag(table)]
    Array(Place, Vec<Reg>),

    #[tag(upval)]
    GetUpVal(Reg, usize),
    #[tag(upval)]
    SetUpVal(usize, Place),

    #[tag(param)]
    SelfParam(),
    #[tag(call)]
    Call(Reg, usize), //Along with Take
    #[tag(call)]
    TailCall(Reg, usize),
    Closure(Reg, usize),
    Return(usize),
    VarArg(Reg), //Along with Take

    // coroutine
    #[tag(cor)]
    Spawn(Reg),
    #[tag(cor)]
    Go(Reg),
    #[tag(cor)]
    Yield(),

    // arithmetic
    #[tag(ari)]
    Unary(Reg, Place, UnOp),
    #[tag(ari)]
    Binary(Reg, Place, Place, BinOp),
    #[tag(ari)]
    BinaryI(Reg, Place, DukaInt, BinOp),
    #[tag(ari)]
    BinaryI2(Reg, DukaInt, Place, BinOp),

    // control flow
    Jump(i32),
    #[tag(cond)]
    /// skip next when `R[reg]` is matched
    SkipNext(Reg, bool),

    // Special
    #[tag(lifetime)]
    Dead(usize),
    #[tag(pending)]
    Take(usize),
    #[tag(pending)]
    TakeAll,
    SysCall(SysCall),
}
