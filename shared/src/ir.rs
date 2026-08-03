use std::fmt::Display;

use crate::{
    constants::cgen::{self, MAX_LOCAL_COUNT, MAX_REGISTER_COUNT},
    errors::{DukaIRError, DukaIRErrorKind},
    types::{BinOp, LogicDatabase, SysCall, UnOp},
    utils::{OrError, UniqueVec},
    value::{ConstValue, DukaFloat, DukaInt},
};
use duka_macros::Info;
use serde::{Deserialize, Serialize};

use crate::types::{DebugInfo, ValueCount};

#[derive(Debug, Default)]
struct LabelScopes {
    scopes: Vec<(Vec<Lab>, bool)>,
}
impl LabelScopes {
    pub fn with_global() -> Self {
        Self {
            scopes: vec![(vec![], true)],
        }
    }
    pub fn enter(&mut self, is_func: bool) {
        self.scopes.push((vec![], is_func))
    }
    pub fn exit(&mut self) {
        self.scopes.pop();
    }
    pub fn declare_label(&mut self, lab: Lab) -> Result<(), ()> {
        if let Some((last, _)) = self.scopes.last_mut() {
            if last.contains(&lab) {
                return Err(());
            }
            last.push(lab);
            return Ok(());
        }
        Ok(())
    }
    pub fn lookup_label(&self, lab: &Lab) -> bool {
        for (labels, is_func) in self.scopes.iter().rev() {
            if labels.contains(lab) {
                return true;
            }
            if *is_func {
                break;
            }
        }
        false
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct NameMapper<K>(pub Vec<(K, Box<str>)>);
impl<K: PartialEq> NameMapper<K> {
    pub fn add(&mut self, key: K, name: impl Into<Box<str>>) {
        self.0.push((key, name.into()))
    }
    pub fn get(&self, key: &K) -> Option<&str> {
        self.0
            .iter()
            .find_map(|(k, v)| (k == key).then_some(v.as_ref()))
    }
    pub fn get_by_name(&self, name: &str) -> Option<&K> {
        self.0
            .iter()
            .find_map(|(k, v)| (v.as_ref() == name).then_some(k))
    }
}
impl<K: Display + PartialEq> NameMapper<K> {
    pub fn format(&self, key: &K) -> String {
        if let Some(v) = self.get(key) {
            v.to_string()
        } else {
            format!("{}", key)
        }
    }
}

/// `instack`: `true`则在parent的栈中, `false`则也是parent的up_value
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpIndex {
    /// For debug
    pub name: Option<String>,
    /// Whether this is a local variable or another up_value in parent closure
    pub local: bool,
    pub index: usize,
    pub kind: UpValueKind,
}
#[derive(Debug, Clone, PartialEq, Default, Info, Serialize, Deserialize)]
#[idcard(u8)]
pub enum UpValueKind {
    #[default]
    Regular,
    ToBeClosed,
}

#[derive(Debug)]
pub struct Labels {
    label_names: NameMapper<Lab>,
    pending_gotos: Vec<(usize, String)>,
    loops: Vec<(Lab, Lab)>,
    scopes: LabelScopes,
    label_top: Lab,
}
impl Default for Labels {
    fn default() -> Self {
        Self::new()
    }
}

impl Labels {
    pub fn into_names(self) -> NameMapper<Lab> {
        self.label_names
    }
    pub fn new() -> Self {
        Self {
            label_names: NameMapper::default(),
            scopes: LabelScopes::with_global(),
            label_top: Lab::default(),
            loops: vec![],
            pending_gotos: vec![],
        }
    }
    pub fn enter(&mut self, is_func: bool) {
        assert!(self.pending_gotos.is_empty());
        self.scopes.enter(is_func);
    }
    pub fn new_goto(&mut self, at: usize, to: String) {
        self.pending_gotos.push((at, to))
    }
    pub fn new_label(&mut self, name: Option<String>) -> Result<Lab, DukaIRErrorKind> {
        let lab = self.label_top;
        self.label_top += 1;

        if let Some(name) = name {
            self.label_names.add(lab, name);
        }
        self.scopes
            .declare_label(lab)
            .map(|_| lab)
            .map_err(|_| DukaIRErrorKind::DuplicatedLabel(lab))
    }
    pub fn new_loop(&mut self, start: Lab, end: Lab) -> usize {
        self.loops.push((start, end));
        self.loops.len() - 1
    }
    pub fn exit_loop(&mut self) {
        self.loops.pop();
    }
    pub fn get_loop(&self) -> Option<(Lab, Lab)> {
        self.loops.last().cloned()
    }
    pub fn resolve_and_exit(&mut self) -> Result<Vec<(usize, Lab)>, DukaIRError> {
        let gotos = std::mem::take(&mut self.pending_gotos);
        let res = gotos
            .into_iter()
            .map(|(at, label)| {
                self.label_names
                    .get_by_name(&label)
                    .filter(|lab| self.scopes.lookup_label(lab))
                    .ok_or_else(|| DukaIRError::from(DukaIRErrorKind::UnsolvedGoto(label.into())))
                    .map(|&lab| (at, lab))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.scopes.exit();
        Ok(res)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constants(UniqueVec<ConstValue>);
impl Constants {
    pub fn push(&mut self, val: ConstValue) -> usize {
        self.0.push(val)
    }
    pub fn into_vec(self) -> Vec<ConstValue> {
        self.0.into_vec()
    }
    pub const fn len(&self) -> usize {
        self.0.len()
    }
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

///## Returned by expression, it could represent an already-allocated value or immediate operands or to-be-allocated values
#[derive(Debug, Clone, PartialEq)]
pub enum ExpDesc {
    /// # This is *already* allocated in registers, constants pool or up_values,
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
    Many(Vec<Reg>, /*var_arg*/ Option<Reg>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TablePlace {
    R(Reg),
    U(usize),
}

impl Display for TablePlace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TablePlace::R(r) => write!(f, "R[{r}]"),
            TablePlace::U(u) => write!(f, "UpVals[{u}]"),
        }
    }
}

impl From<TablePlace> for Place {
    fn from(value: TablePlace) -> Self {
        match value {
            TablePlace::U(u) => Self::U(u),
            TablePlace::R(r) => Self::R(r),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValuePlace {
    R(Reg),
    K(Cst),
    I(DukaInt),
}

impl Display for ValuePlace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValuePlace::R(r) => write!(f, "R[{r}]"),
            ValuePlace::K(k) => write!(f, "Consts[{k}]"),
            ValuePlace::I(i) => write!(f, "[{i}i]"),
        }
    }
}

impl From<ValuePlace> for Place {
    fn from(value: ValuePlace) -> Self {
        match value {
            ValuePlace::I(i) => Self::I(i),
            ValuePlace::K(k) => Self::K(k),
            ValuePlace::R(r) => Self::R(r),
        }
    }
}

///## Things that are already allocated in registers or constants pool or up_values
#[derive(Debug, Clone, PartialEq, Info, Serialize, Deserialize)]
#[shy]
pub enum Place {
    /// this is pointing to registers index
    #[tag(stored)]
    R(Reg),
    /// this is pointing to constants index
    #[tag(stored)]
    K(Cst),
    /// this is pointing to index of up_vals vector in scope
    #[tag(stored)]
    U(usize),
    /// this is an immediate number
    I(DukaInt),
}
impl Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Place::R(r) => write!(f, "R[{r}]"),
            Place::K(k) => write!(f, "Consts[{k}]"),
            Place::U(u) => write!(f, "UpVals[{u}]"),
            Place::I(i) => write!(f, "[{i}i]"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    fn declare_up_val(&mut self, name: &str, up_idx: UpIndex) {
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
    fn declare_local(&mut self, name: &str, reg: usize) -> Result<(), DukaIRError> {
        let locals = match self {
            Scope::Block { locals, .. } => locals,
            Scope::Function { locals, .. } => locals,
        };
        (locals.len() > MAX_LOCAL_COUNT).then_error(|| DukaIRError {
            kind: DukaIRErrorKind::TooManyLocals {
                got: locals.len(),
                limit: MAX_LOCAL_COUNT,
            },
        })?;
        locals.push((name.to_string(), reg));
        Ok(())
    }
    fn find_up_val(&self, name: &str) -> Option<usize> {
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
                        .or_else(|| self.find_up_val(name).map(Place::U))
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
    use super::{Place, Scopes};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scopes {
    scopes: Vec<Scope>,
    functions: Vec<usize>,
}
#[allow(unused)]
impl Default for Scopes {
    fn default() -> Self {
        Self::new()
    }
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
    // #[inline]
    // fn current(&self) -> &Scope {
    //     assert!(!self.is_empty());
    //     self.scopes.last().unwrap()
    // }
    #[inline]
    fn current_mut(&mut self) -> &mut Scope {
        assert!(!self.is_empty());
        self.scopes.last_mut().unwrap()
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.scopes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    pub fn ensure_global(&mut self) -> Place {
        assert!(!self.is_empty());
        self.find(cgen::GLOBAL).unwrap_or_else(|| {
            let main = self.scopes.first_mut().unwrap();
            main.declare_up_val(
                cgen::GLOBAL,
                UpIndex {
                    name: Some(cgen::GLOBAL.to_owned()),
                    local: true,
                    index: cgen::ENV_UPVAL_IDX,
                    kind: UpValueKind::Regular,
                },
            );
            self.find(cgen::GLOBAL).unwrap()
        })
    }

    #[inline]
    pub fn declare_up_val(&mut self, name: &str, up_idx: UpIndex) {
        self.current_mut().declare_up_val(name, up_idx);
    }
    #[inline]
    pub fn declare_local(&mut self, name: &str, reg: usize) -> Result<(), DukaIRError> {
        self.current_mut().declare_local(name, reg)
    }
    #[inline]
    pub fn declare_const(&mut self, name: &str, idx: usize) {
        self.current_mut().declare_const(name, idx);
    }

    /// # Panic
    /// Please ensure that func is a function scope
    fn create_up_val_unchecked(func: &mut Scope, name: &str, is_local: bool, idx: usize) -> usize {
        let Scope::Function { up_vals, .. } = func else {
            panic!("WHY YOU DONT FOLLOW THE RULE");
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
        up_vals.len() - 1
    }
    pub fn find(&mut self, name: &str) -> Option<Place> {
        assert!(!self.is_empty());

        let mut up_val_mode = false;
        let mut chain = vec![];
        for (idx, scope) in self.scopes.iter().enumerate().rev() {
            let find = scope.find_existed(name);

            if up_val_mode {
                if let Some(ai) = find {
                    match ai {
                        Place::R(n) | Place::U(n) => {
                            let mut idx: usize = n;
                            for (i, func_idx) in chain.into_iter().rev().enumerate() {
                                // Ensured
                                let f = self.scopes.get_mut(func_idx).unwrap();
                                idx = Self::create_up_val_unchecked(
                                    f,
                                    name,
                                    i == 0 && matches!(ai, Place::R(..)),
                                    idx,
                                );
                            }
                            return Some(Place::U(idx));
                        }
                        r @ Place::K(..) => return Some(r), // <const> 直接返回
                        _ => panic!("Variable cannot be an immediate value"),
                    }
                } else if matches!(scope, Scope::Function { .. }) {
                    chain.push(idx);
                }
            } else if find.is_some() {
                return find;
            }

            // 超出父函数边界, 涉及up_value
            if !up_val_mode && matches!(scope, Scope::Function { .. }) {
                up_val_mode = true;
                chain.push(idx);
            }
        }

        None
    }

    /// Whether a register is bound to a currently-visible local (i.e. the
    /// register is still "owned" by a scoped variable). A local register must
    /// NOT be handed to the allocator's free list while its scope is alive,
    /// otherwise a later allocation can clobber the variable's value.
    pub fn is_local_reg(&self, reg: usize) -> bool {
        self.scopes
            .iter()
            .any(|s| match s {
                Scope::Function { locals, .. } | Scope::Block { locals, .. } => {
                    locals.iter().any(|(_, r)| *r == reg)
                }
            })
    }

    /// NOTICE, YOU MUST HAVE AT LEAST ONE FUNCTION SCOPE BEFORE ENTERING A BLOCK SCOPE!
    pub fn enter(&mut self, is_func: bool) {
        if is_func {
            self.functions.push(self.len());
        }
        self.scopes.push(if is_func {
            Scope::Function {
                locals: vec![],
                consts: vec![],
                up_vals: vec![],
            }
        } else {
            Scope::Block {
                locals: vec![],
                consts: vec![],
            }
        });
    }
    pub fn exit(&mut self) -> Scope {
        assert!(!self.is_empty());
        // Ensured
        let scope = self.scopes.pop().unwrap();
        if self.len() > 1 && matches!(scope, Scope::Function { .. }) {
            self.functions.pop();
        }
        scope
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
    allocated: Vec<Reg>,
}
#[allow(unused)]
impl Default for Allocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Allocator {
    pub fn new() -> Self {
        Self {
            snapshots: vec![],
            current: AllocatorSnapshot::default(),
        }
    }
    pub fn get_allocated_regs(&self) -> &[Reg] {
        &self.current.allocated
    }
    /// Literally the newest unallocated register number
    pub const fn top(&self) -> Reg {
        self.current.top
    }
    /// The most top available register number, taking freed registers into consideration
    pub fn available_top(&self) -> Reg {
        self.current
            .free_list
            .iter()
            .min()
            .copied()
            .unwrap_or_else(|| self.top())
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

    pub fn ensure_allocated(&mut self, reg: Reg) -> Result<(), DukaIRError> {
        if reg >= self.top() {
            self.alloc_consecutive_from(self.top(), reg - self.top() + 1)?
                .count();
        }
        Ok(())
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
    /// Allocate some registers range to a certain register(exclusive), returns them
    pub fn alloc_consecutive_from(
        &mut self,
        start: Reg,
        count: usize,
    ) -> Result<impl Iterator<Item = Reg>, DukaIRError> {
        let end = start + count;
        (end > MAX_REGISTER_COUNT).then_error(|| DukaIRError {
            kind: DukaIRErrorKind::TooManyRegisters {
                got: end,
                limit: MAX_REGISTER_COUNT,
            },
        })?;

        // Reserve the exact range `start..end`: drop it from the free list and
        // extend the top to cover it (do NOT pop free registers, they may not
        // belong to the requested range).
        if end > self.current.top {
            self.current.top = end;
        }
        self.current
            .free_list
            .retain(|reg| !(*reg >= start && *reg < end));
        for reg in start..end {
            if !self.current.allocated.contains(&reg) {
                self.current.allocated.push(reg);
            }
        }

        Ok((start..end).into_iter())
    }
    pub fn alloc_consecutive(
        &mut self,
        count: usize,
    ) -> Result<impl Iterator<Item = Reg>, DukaIRError> {
        let free_list = &self.current.free_list;
        if free_list.len() == count {
            let free_list_min = *free_list.iter().max().unwrap();
            if (free_list_min - free_list.iter().min().unwrap()) == count - 1 {
                return self.alloc_consecutive_from(free_list_min, count);
            }
        } else if free_list.len() > count {
            'outer: for freed in free_list {
                if *freed != 0 && !free_list.contains(&(*freed - 1)) {
                    for i in 1..count {
                        if !free_list.contains(&(i + *freed)) {
                            break 'outer;
                        }
                    }
                    return self.alloc_consecutive_from(*freed, count);
                }
            }
        }
        self.alloc_consecutive_from(self.available_top(), count)
    }
    /// this has infinite registers? NO!
    pub fn alloc(&mut self) -> Result<Reg, DukaIRError> {
        let idx = if !self.current.free_list.is_empty() {
            self.current.free_list.sort();
            self.current.free_list.remove(0)
        } else {
            let res = self.current.top;

            (res > MAX_REGISTER_COUNT).then_error(|| DukaIRError {
                kind: DukaIRErrorKind::TooManyRegisters {
                    got: res,
                    limit: MAX_REGISTER_COUNT,
                },
            })?;

            self.current.top += 1;
            res
        };
        assert!(!self.current.allocated.contains(&idx));
        self.current.allocated.push(idx);
        Ok(idx)
    }

    /// # For those who needs intermediate storage
    pub fn alloc_temp(&mut self) -> Result<Reg, DukaIRError> {
        // allocate it, its life ends at next allocation
        (MAX_REGISTER_COUNT > self.current.top)
            .then_some(self.current.top)
            .ok_or(DukaIRError {
                kind: DukaIRErrorKind::TooManyRegisters {
                    got: self.current.top,
                    limit: MAX_REGISTER_COUNT,
                },
            })
    }

    /// Allocate a brand-new register above everything currently live, ignoring
    /// the free list. Used for call frames, which must sit above every live
    /// register so the VM can resolve arguments from `func+1..` to the top.
    pub fn alloc_fresh(&mut self) -> Result<Reg, DukaIRError> {
        let res = self.current.top;
        (res > MAX_REGISTER_COUNT).then_error(|| DukaIRError {
            kind: DukaIRErrorKind::TooManyRegisters {
                got: res,
                limit: MAX_REGISTER_COUNT,
            },
        })?;
        self.current.top += 1;
        assert!(!self.current.allocated.contains(&res));
        self.current.allocated.push(res);
        Ok(res)
    }

    pub fn used_reg_count(&self) -> usize {
        self.current.allocated.len() + self.current.free_list.len()
    }

    pub fn free(&mut self, who: Reg) {
        if !self.current.free_list.contains(&who)
            && let Some(idx) = &self
                .current
                .allocated
                .iter()
                .enumerate()
                .find_map(|(i, v)| (*v == who).then_some(i))
        {
            self.current.allocated.remove(*idx);
            self.current.free_list.push(who);
        }
    }
}

pub type RegUsingMap = Box<[Box<[Reg]>]>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RegLifetime {
    pub count: usize,
    pub using: RegUsingMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DukaIR {
    pub param_count: usize,
    pub has_var_arg: bool,
    pub reg_lifetime: RegLifetime,

    pub instructions: Box<[IR]>,
    pub nesteds: Box<[DukaIR]>,
    pub constants: Box<Constants>,
    pub up_indexes: Box<[UpIndex]>,
    pub debug_info: Box<DebugInfo>,
    pub label_names: Box<NameMapper<Lab>>,
    pub logic: Option<Box<LogicDatabase>>,
}

impl Display for DukaIR {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "{} input.duka:({}) [{} instructions]",
            self.debug_info.debug_name.as_deref().unwrap_or("..."),
            self.debug_info.all_span,
            self.instructions.len()
        )?;
        writeln!(
            f,
            "{} params (var_arg: {}), {} consts, {} nesteds",
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
            if !matches!(ins, IR::Label(..)) {
                write!(f, ".{} ", ins)?;
            }

            fn get_label(mapper: &NameMapper<Lab>, lab: &Lab) -> String {
                mapper.format(lab)
            }

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
                //IR::SetFieldI(to, idx, val) => writeln!(f, "{to}.set([{idx}] := {val})")?,
                IR::NewTable(to) => writeln!(f, "R[{to}] <- {{}} %dynamic table%")?,
                IR::Array(place, items) => writeln!(f, "{place}.pushes({items:?})")?,
                IR::GetUpVal(to, who) => writeln!(f, "R[{to}] <- UpVals[{who}]")?,
                IR::SetUpVal(who, place) => writeln!(f, "UpVals[{who}] <- {place}")?,
                IR::SelfParam() => writeln!(f, "%next call%")?,
                IR::Call(who, params) => writeln!(
                    f,
                    "R[{who}](params: {params}) %{}%",
                    params.format_register(*who + 1)
                )?,
                IR::TailCall(who, params) => writeln!(
                    f,
                    "R[{who}](params: {params}) %{}%, tailcall",
                    params.format_register(*who + 1)
                )?,
                IR::Closure(to, cls) => writeln!(
                    f,
                    "R[{to}] <- nesteds#{cls} %{}%",
                    self.nesteds
                        .get(*cls)
                        .unwrap()
                        .debug_info
                        .debug_name
                        .as_deref()
                        .unwrap_or("...")
                )?,
                IR::Return(from, n) => writeln!(f, "{}", n.format_register(*from))?,
                IR::VarArg(to) => writeln!(f, "R[{to}] <- ...")?,
                IR::Spawn(to, who) => writeln!(f, "R[{to}] <- coroutine({who})")?,
                IR::Go(who, params) => writeln!(
                    f,
                    "coroutine(R[{who}])(params: {params}) %{}%",
                    params.format_register(*who + 1)
                )?,
                IR::Yield(from, params) => writeln!(f, "{}", params.format_register(*from))?,
                IR::Unary(to, place, un_op) => writeln!(f, "R[{to}] <- |{un_op}| {place}")?,
                IR::Binary(to, left, right, bin_op) => {
                    writeln!(f, "R[{to}] <- {left} |{bin_op}| {right}")?
                }
                IR::Concat(to, from, count) => {
                    writeln!(f, "R[{to}] <- concat(R[{from}] to R[{}])", from + count)?
                }
                IR::Jump(to) => writeln!(f, "to ::{}::", get_label(&self.label_names, to))?,
                IR::SkipNext(cond, to) => writeln!(f, "R[{cond}] is {to} ?: to [{:0>2}]", i + 2)?,
                IR::Take(num) => writeln!(f, "{num} %for [{:0>2}]%", i - 1)?,
                IR::TakeAll => writeln!(f, "%for [{:0>2}]%", i - 1)?,
                IR::SysCall(sys_call) => writeln!(f, "@{sys_call:?}")?,

                IR::ForPrep(from, to) => writeln!(
                    f,
                    "R[{from}] %prepare numeric forloop, with ::{}::%",
                    get_label(&self.label_names, to)
                )?,
                IR::ForLoop(from, to) => writeln!(
                    f,
                    "R[{from}] %check numeric forloop, with ::{}::%",
                    get_label(&self.label_names, to)
                )?,
                IR::TForPrep(from, to) => writeln!(
                    f,
                    "R[{from}] %prepare generic forloop, with ::{}::%",
                    get_label(&self.label_names, to)
                )?,
                IR::TForCall(callee, n) => writeln!(f, "iterator(R[{callee}]) take({n})")?,
                IR::TForLoop(from, to) => writeln!(
                    f,
                    "R[{from}] %prepare generic forloop, with ::{}::%",
                    get_label(&self.label_names, to)
                )?,
                IR::Label(l) => writeln!(f, "::{}::", get_label(&self.label_names, l))?,
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
pub type Lab = usize;

#[derive(Debug, Clone, PartialEq, Info, Default, Serialize, Deserialize)]
pub enum IR {
    #[default]
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
    LoadString(Reg, Box<[u8]>),

    // Table-related
    #[tag(table)]
    GetField(Reg, TablePlace, ValuePlace),
    #[tag(table)]
    SetField(TablePlace, ValuePlace, ValuePlace),
    #[tag(table)]
    NewTable(Reg),
    #[tag(table)]
    Array(Reg, ValueCount),

    #[tag(up_val)]
    GetUpVal(Reg, usize),
    #[tag(up_val)]
    SetUpVal(usize, Reg),

    #[tag(param)]
    SelfParam(),
    #[tag(call)]
    Call(Reg, ValueCount), //Along with Take
    #[tag(call)]
    TailCall(Reg, ValueCount),
    Closure(Reg, usize),
    Return(Reg, ValueCount),
    VarArg(Reg), //Along with Take

    // coroutine
    #[tag(cor)]
    Spawn(Reg, Reg),
    #[tag(cor)]
    Go(Reg, ValueCount),
    #[tag(cor)]
    Yield(Reg, ValueCount),

    // arithmetic
    #[tag(ari)]
    Unary(Reg, ValuePlace, UnOp),
    #[tag(ari)]
    Binary(Reg, ValuePlace, ValuePlace, BinOp),

    Concat(Reg, Reg, usize),

    // control flow
    Label(Lab),
    Jump(Lab),
    #[tag(for_loop)]
    ForPrep(Reg, Lab),
    #[tag(for_loop)]
    ForLoop(Reg, Lab),
    #[tag(tfor_loop)]
    TForPrep(Reg, Lab),
    #[tag(tfor_loop)]
    TForCall(Reg, usize),
    #[tag(tfor_loop)]
    TForLoop(Reg, Lab),
    /// skip next when `R[reg]` is matched
    #[tag(cond)]
    SkipNext(Reg, bool),

    // Not Special Now
    #[tag(pending)]
    Take(usize),
    #[tag(pending)]
    TakeAll,
    SysCall(SysCall),
}
