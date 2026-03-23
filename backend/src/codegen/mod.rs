//! # Code Generator for Duka

pub mod binary;
pub mod errors;
pub mod logic;
use std::{collections::HashMap, fmt::Debug};

use duka_shared::{
    constants::MetaMethodAction,
    ir::{Constants, Cst, DukaIR, IR, Lab, Reg, RegUsingMap, TablePlace, ValuePlace},
    types::{BinOp, DebugInfo, DukaGenerator, UnOp, ValueCount},
    value::{ConstValue, DukaInt},
};

use crate::{
    codegen::errors::DukaDefaultError,
    instructions::{
        Address, Bits9, Bits17, Bits25, Instruction as I, SignedBits8, SignedBits9, SignedBits17,
        SignedBits25,
    },
    value::DukaProto,
};

struct JumpPending {
    label: Lab,
    at: usize,
    constructor: Box<dyn FnOnce(usize) -> Result<I, DukaDefaultError>>,
}
impl Debug for JumpPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JumpPending")
            .field("lab", &self.label)
            .field("at", &self.at)
            .finish()
    }
}

#[derive(Debug, Default)]
pub struct DefaultGenerator {
    constants: Constants,
    debug_info: DebugInfo,
    instructions: Vec<I>,

    self_params: Vec<usize>,
    labels: HashMap<Lab, usize>,
    pending: Vec<JumpPending>,
}

#[inline]
fn addr(n: usize) -> Result<Address, DukaDefaultError> {
    I::MakeAddress(n).ok_or(DukaDefaultError::InvalidAddress(n))
}

#[inline]
fn offset_jump(from: usize, to: usize) -> Result<SignedBits25, DukaDefaultError> {
    let val = to as isize - from as isize;
    I::MakeSignedBits25(val).ok_or(DukaDefaultError::InvalidJumpPosition { from, to })
}
#[inline]
fn offset_for(from: usize, to: usize) -> Result<Bits17, DukaDefaultError> {
    let val = to.saturating_sub(from);
    I::MakeBits17(val).ok_or(DukaDefaultError::InvalidJumpPosition { from, to })
}

enum MM {
    D(Address, Address),
    I(Address, DukaInt, bool),
    K(Address, Cst, bool),
    N,
}
enum RI {
    R(Address),
    I(DukaInt),
}
enum RK {
    R(Address),
    K(usize),
}

impl DefaultGenerator {
    #[inline]
    fn emit_loadi(&mut self, to: Address, what: DukaInt) {
        if let Some(res) = I::MakeSignedBits17(what as isize) {
            self.emit(I::LoadI(to, res))
        } else {
            let k = self.constants.push(ConstValue::Int(what));
            self.emit_loadk(to, k);
        }
    }
    #[inline]
    fn emit_loadk(&mut self, to: Address, k: usize) {
        if let Some(k) = I::MakeBits17(k) {
            self.emit(I::LoadK(to, k))
        } else {
            self.emit(I::ExtraArg(k as u32));
            self.emit(I::LoadKX(to));
        }
    }
    #[inline]
    fn emit(&mut self, i: I) {
        self.instructions.push(i)
    }
    #[inline]
    fn emit_placeholder(&mut self) -> usize {
        self.instructions.push(I::Return0());
        self.instructions.len() - 1
    }
    #[inline]
    fn emit_fixup(&mut self, who: usize, i: I) {
        assert!(who < self.instructions.len());
        self.instructions[who] = i;
    }

    /// Whether constant or not
    fn imm_to_k(&mut self, val: ValuePlace) -> Result<(Address, bool), DukaDefaultError> {
        Ok(match val {
            ValuePlace::R(r) => (addr(r)?, false),
            ValuePlace::K(k) => (addr(k)?, true),
            ValuePlace::I(i) => {
                let k: usize = self.constants.push(ConstValue::Int(i));
                (addr(k)?, true)
            }
        })
    }

    fn emit_jump(&mut self, label: Lab) -> Result<(), DukaDefaultError> {
        if let Some(to) = self.labels.get(&label) {
            self.emit(I::Jump(offset_jump(self.instructions.len(), *to)?));
        } else {
            let at = self.emit_placeholder();
            self.pending.push(JumpPending {
                label,
                at,
                constructor: Box::new(move |to| {
                    let offset = offset_jump(at, to)?;
                    Ok(I::Jump(offset))
                }),
            });
        }
        Ok(())
    }

    fn gen_irs(&mut self, irs: Box<[IR]>, using_map: RegUsingMap) -> Result<(), DukaDefaultError> {
        let mut iter = irs.into_iter().zip(using_map).peekable();

        macro_rules! take {
            ($ir: expr) => {{
                let Some((el @ IR::TakeAll | el @ IR::Take(..), _)) = iter.next() else {
                    return Err(DukaDefaultError::ExpectedTake($ir.into()));
                };
                match el {
                    IR::TakeAll => ValueCount::VarArg,
                    IR::Take(n) => ValueCount::Exact(n),
                    _ => unreachable!(),
                }
            }};
        }

        while let Some((ir, using_regs)) = iter.next() {
            match ir {
                IR::Void => continue,
                IR::Move(to, from) => {
                    self.emit(I::Move(addr(to)?, addr(from)?));
                }
                IR::LoadNil(to) => {
                    let count = iter
                        .by_ref()
                        .enumerate()
                        .take_while(|(i, (ir, _))| matches!(ir, IR::LoadNil(t) if *t == i + to + 1))
                        .count()
                        + 1;
                    self.emit(I::LoadNil(addr(to)?, count as Bits17))
                }
                IR::LoadTrue(to) => self.emit(I::LoadTrue(addr(to)?)),
                IR::LoadFalse(to) => self.emit(I::LoadFalse(addr(to)?)),
                IR::LoadConst(to, k) => self.emit_loadk(addr(to)?, k),
                IR::LoadFloat(to, fl) => {
                    let k = self.constants.push(ConstValue::Float(fl));
                    self.emit_loadk(addr(to)?, k);
                }
                IR::LoadInt(to, what) => {
                    self.emit_loadi(addr(to)?, what);
                }
                IR::LoadString(to, str) => {
                    let k = self.constants.push(ConstValue::String(str));
                    self.emit_loadk(addr(to)?, k);
                }
                IR::GetField(to, from, who) => self.gen_get_field(to, from, who)?,
                IR::SetField(tab, key, val) => self.gen_set_field(tab, key, val)?,
                IR::NewTable(to) => self.emit(I::NewTable(addr(to)?)),
                IR::Array(tab, count) => self.emit(I::SetList(addr(tab)?, 0, count.into())),
                IR::GetUpVal(to, who) => self.emit(I::GetUpVal(addr(to)?, who as Bits17)),
                IR::SetUpVal(who, what) => self.emit(I::SetUpVal(addr(what)?, who as Bits17)),
                IR::SelfParam() => {
                    let who = self.emit_placeholder();
                    self.self_params.push(who);
                }
                IR::Call(callee, params) => {
                    if let Some(at) = self.self_params.pop() {
                        self.emit_fixup(at, I::Self_(addr(callee)?, 0, 0, true));
                    }
                    let returns = take!("Call");
                    self.emit(I::Call(addr(callee)?, params.into(), returns.into()))
                }
                IR::TailCall(callee, params) => {
                    if let Some(at) = self.self_params.pop() {
                        self.emit_fixup(at, I::Self_(addr(callee)?, 0, 0, true));
                    }
                    let returns = take!("TailCall");
                    self.emit(I::TailCall(addr(callee)?, params.into(), returns.into()))
                }
                IR::Closure(to, idx) => self.emit(I::Closure(addr(to)?, idx as Bits17)),
                IR::Return(from, value_count) => self.emit(if value_count.is_empty() {
                    I::Return0()
                } else {
                    I::Return(addr(from)?, value_count.into())
                }),
                IR::VarArg(to) => {
                    let count = take!("VarArg");
                    self.emit(I::VarArg(addr(to)?, count.into()));
                }
                IR::Spawn(to, from) => self.emit(I::Spawn(addr(to)?, addr(from)?)),
                IR::Go(callee, params) => {
                    let returns = take!("Go");
                    self.emit(I::Go(addr(callee)?, params.into(), returns.into()))
                }
                IR::Yield(from, count) => {
                    let returns = take!("Yield");
                    self.emit(I::Yield(addr(from)?, count.into(), returns.into()))
                }
                IR::Unary(to, place, un_op) => {
                    let to = addr(to)?;
                    if let ValuePlace::I(i) = place {
                        self.emit(match un_op {
                            UnOp::Length => I::LoadI(to, 0),
                            UnOp::Not => I::LoadTrue(to),
                            UnOp::BitNot => I::LoadI(to, !i as SignedBits17),
                            UnOp::Minus => I::LoadI(to, -i as SignedBits17),
                        })
                    } else {
                        let from = match place {
                            ValuePlace::R(r) => addr(r)?,
                            ValuePlace::K(k) => {
                                self.emit_loadk(to, k);
                                to
                            }
                            _ => unreachable!(),
                        };
                        self.emit(match un_op {
                            UnOp::Length => I::Length(to, from),
                            UnOp::Not => I::Not(to, from),
                            UnOp::BitNot => I::BitNot(to, from),
                            UnOp::Minus => I::Minus(to, from),
                        });
                    }
                }
                IR::Binary(to, left, right, bin_op) => {
                    self.gen_binary(to, left, right, bin_op, using_regs)?
                }
                IR::Concat(to, from, count) => {
                    let (to, from) = (addr(to)?, addr(from)?);
                    self.emit(I::Concat(from, count as Bits17));
                    self.move_if_need(to, from);
                }
                IR::Label(label) => {
                    self.labels.insert(label, self.instructions.len());
                }
                IR::Jump(label) => self.emit_jump(label)?,
                IR::ForPrep(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pending.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::ForPrepare(a, offset_for(at, to)?))),
                    })
                }
                IR::ForLoop(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pending.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::ForLoop(a, offset_for(at, to)?))),
                    })
                }
                IR::TForPrep(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pending.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::TForPrepare(a, offset_for(at, to)?))),
                    })
                }
                IR::TForCall(a, needs) => {
                    let a = addr(a)?;
                    self.emit(I::TForCall(a, needs as Address))
                }
                IR::TForLoop(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pending.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::TForLoop(a, offset_for(at, to)?))),
                    })
                }
                IR::SkipNext(cond, what) => self.emit(I::Test(addr(cond)?, what)),
                IR::Take(_) | IR::TakeAll => return Err(DukaDefaultError::AloneTake),
                IR::SysCall(_) => unimplemented!(),
            }
        }

        while let Some(JumpPending {
            label,
            at,
            constructor,
        }) = self.pending.pop()
        {
            let to = *self
                .labels
                .get(&label)
                .ok_or(DukaDefaultError::UnsolvedLabel)?;
            self.emit_fixup(at, constructor(to)?);
        }

        Ok(())
    }

    fn check_imm9(&mut self, vp: ValuePlace) -> ValuePlace {
        if let ValuePlace::I(i) = vp
            && I::MakeSignedBits9(i as isize).is_none()
        {
            let k = self.constants.push(ConstValue::Int(i));
            ValuePlace::K(k)
        } else {
            vp
        }
    }

    fn move_if_need(&mut self, to: Address, from: Address) {
        if to != from {
            self.emit(I::Move(to, from));
        }
    }

    fn alloc_safely(&self, using_regs: &[Reg]) -> Reg {
        using_regs.iter().max().map(|v| *v + 1).unwrap_or(0)
    }

    fn bin_rki_left(
        &mut self,
        left: ValuePlace,
        using_regs: &[Reg],
    ) -> Result<RI, DukaDefaultError> {
        Ok(match left {
            ValuePlace::R(r) => RI::R(addr(r)?),
            ValuePlace::K(k) => {
                let place = addr(self.alloc_safely(using_regs))?;
                self.emit_loadk(place, k);
                RI::R(place)
            }
            ValuePlace::I(i) => {
                RI::I(i)
                // let place = addr(self.alloc_safely(using_regs))?;
                // self.emit_loadi(place, i);
                // place
            }
        })
    }

    fn bin_rk(
        &mut self,
        left: ValuePlace,
        right: ValuePlace,
        using_regs: &[Reg],
    ) -> Result<(RI, RK), DukaDefaultError> {
        let mut allocated: bool = false;
        let left = match left {
            ValuePlace::R(r) => RI::R(addr(r)?),
            ValuePlace::K(k) => {
                let place = addr(self.alloc_safely(using_regs))?;
                self.emit_loadk(place, k);
                allocated = true;
                RI::R(place)
            }
            ValuePlace::I(i) => {
                RI::I(i)
                // let place = addr(self.alloc_safely(using_regs))?;
                // self.emit_loadi(place, i);
                // allocated = true;
                // place
            }
        };
        Ok(match right {
            ValuePlace::R(r) => (left, RK::R(addr(r)?)),
            ValuePlace::K(k) => (left, RK::K(k)),
            ValuePlace::I(i) => {
                let place = addr(self.alloc_safely(using_regs) + if allocated { 1 } else { 0 })?;
                self.emit_loadi(place, i);
                (left, RK::R(place))
            }
        })
    }

    fn without_k(
        &mut self,
        left: ValuePlace,
        right: ValuePlace,
        using_regs: &[Reg],
    ) -> Result<(RI, RI), DukaDefaultError> {
        let mut allocated: bool = false;
        let left = match left {
            ValuePlace::R(r) => RI::R(addr(r)?),
            ValuePlace::K(k) => {
                let place = addr(self.alloc_safely(using_regs))?;
                self.emit_loadk(place, k);
                allocated = true;
                RI::R(place)
            }
            ValuePlace::I(i) => RI::I(i),
        };
        Ok(match right {
            ValuePlace::R(r) => (left, RI::R(addr(r)?)),
            ValuePlace::I(i) => (left, RI::I(i)),
            ValuePlace::K(k) => {
                let place = addr(self.alloc_safely(using_regs) + if allocated { 1 } else { 0 })?;
                self.emit_loadk(place, k);
                (left, RI::R(place))
            }
        })
    }

    fn gen_binary(
        &mut self,
        to: usize,
        left: ValuePlace,
        right: ValuePlace,
        bin_op: BinOp,
        using_regs: Box<[Reg]>,
    ) -> Result<(), DukaDefaultError> {
        let to = addr(to)?;
        let (left, right) = (self.check_imm9(left), self.check_imm9(right));

        let mm = match bin_op {
            BinOp::Add => match (left, right) {
                (ValuePlace::I(i), ValuePlace::I(i2)) => {
                    self.emit_loadi(to, i + i2);
                    MM::N
                }
                (ValuePlace::R(r), ValuePlace::I(i)) => {
                    let r = addr(r)?;
                    self.emit(I::AddI(to, r, i as SignedBits9));
                    MM::I(r, i, false)
                }
                (ValuePlace::I(i), ValuePlace::R(r)) => {
                    let r = addr(r)?;
                    self.emit(I::AddI(to, r, i as SignedBits9));
                    MM::I(r, i, true)
                }
                (ValuePlace::R(r), ValuePlace::K(k)) => {
                    let r = addr(r)?;
                    self.emit(I::AddK(to, r, k as Address));
                    MM::K(r, k, false)
                }
                (ValuePlace::K(k), ValuePlace::R(r)) => {
                    let r = addr(r)?;
                    self.emit(I::AddK(to, r, k as Address));
                    MM::K(r, k, true)
                }
                (ValuePlace::R(l), ValuePlace::R(r)) => {
                    let (l, r) = (addr(l)?, addr(r)?);
                    self.emit(I::Add(to, l, r));
                    MM::D(l, r)
                }
                (ValuePlace::K(k), ValuePlace::I(i)) => {
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::AddI(to, to, i as SignedBits9));
                    MM::I(to, i, false)
                }
                (ValuePlace::I(i), ValuePlace::K(k)) => {
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::AddI(to, to, i as SignedBits9));
                    MM::I(to, i, true)
                }
                (ValuePlace::K(k), ValuePlace::K(k2)) => {
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::AddK(to, to, k2 as Address));
                    MM::K(to, k2, false)
                }
            },
            BinOp::Sub => match (left, right) {
                (ValuePlace::I(i), ValuePlace::I(i2)) => {
                    self.emit_loadi(to, i - i2);
                    MM::N
                }
                (ValuePlace::R(r), ValuePlace::I(i)) => {
                    let r = addr(r)?;
                    self.emit(I::AddI(to, r, -i as SignedBits9));
                    MM::I(r, i, false)
                }
                (ValuePlace::R(r), ValuePlace::K(k)) => {
                    let r = addr(r)?;
                    self.emit(I::SubK(to, r, k as Address));
                    MM::K(r, k, false)
                }
                (ValuePlace::K(k), ValuePlace::R(r)) => {
                    let r = addr(r)?;
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::Sub(to, to, r));
                    MM::D(to, r)
                }
                (ValuePlace::I(i), ValuePlace::R(r)) => {
                    let r = addr(r)?;
                    self.emit_loadi(to, i);
                    self.emit(I::Sub(to, to, r));
                    MM::I(r, i, true)
                }
                (ValuePlace::R(l), ValuePlace::R(r)) => {
                    let (l, r) = (addr(l)?, addr(r)?);
                    self.emit(I::Sub(to, l, r));
                    MM::D(l, r)
                }
                (ValuePlace::K(k), ValuePlace::I(i)) => {
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::AddI(to, to, -i as SignedBits9));
                    MM::I(to, i, false)
                }
                (ValuePlace::I(i), ValuePlace::K(k)) => {
                    self.emit_loadi(to, i);
                    self.emit(I::SubK(to, to, k as Address));
                    MM::K(to, k, true)
                }
                (ValuePlace::K(k), ValuePlace::K(k2)) => {
                    self.emit(I::LoadK(to, k as Bits17));
                    self.emit(I::SubK(to, to, k2 as Address));
                    MM::K(to, k2, false)
                }
            },
            BinOp::Multiply => todo!(),
            BinOp::Divide => todo!(),
            BinOp::IDivide => todo!(),
            BinOp::Mod => todo!(),
            BinOp::Pow => todo!(),
            BinOp::Xor => todo!(),
            BinOp::Equal => match (left, right) {
                (ValuePlace::R(l), ValuePlace::R(r)) => {
                    let (l, r) = (addr(l)?, addr(r)?);
                    self.emit(I::Equal(to, l, r, true));
                    MM::D(l, r)
                }
                (ValuePlace::R(r), ValuePlace::K(k)) => {
                    let r = addr(r)?;
                    self.emit(I::EqualK(to, r, k as Address, true));
                    MM::K(r, k, false)
                }
                (ValuePlace::R(_), ValuePlace::I(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::R(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::K(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::I(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::R(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::K(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::I(_)) => todo!(),
            },
            BinOp::NotEqual => match (left, right) {
                (ValuePlace::R(l), ValuePlace::R(r)) => {
                    let (l, r) = (addr(l)?, addr(r)?);
                    self.emit(I::Equal(to, l, r, false));
                    MM::D(l, r)
                }
                (ValuePlace::R(_), ValuePlace::K(_)) => todo!(),
                (ValuePlace::R(_), ValuePlace::I(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::R(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::K(_)) => todo!(),
                (ValuePlace::K(_), ValuePlace::I(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::R(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::K(_)) => todo!(),
                (ValuePlace::I(_), ValuePlace::I(_)) => todo!(),
            },
            BinOp::Greater => match self.without_k(left, right, &using_regs)? {
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::Less(to, r, l));
                    MM::D(r, l)
                }
                (RI::R(r), RI::I(i)) => {
                    self.emit(I::GreaterI(to, r, i as SignedBits9));
                    MM::I(r, i, false)
                }
                (RI::I(i), RI::R(r)) => {
                    self.emit(I::LessEqualI(to, r, i as SignedBits9));
                    MM::I(r, i, true)
                }
                (RI::I(i), RI::I(i2)) => {
                    self.emit(if i > i2 {
                        I::LoadTrue(to)
                    } else {
                        I::LoadFalse(to)
                    });
                    MM::N
                }
            },
            BinOp::Less => match self.without_k(left, right, &using_regs)? {
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::Less(to, l, r));
                    MM::D(l, r)
                }
                (RI::R(r), RI::I(i)) => {
                    self.emit(I::LessI(to, r, i as SignedBits9));
                    MM::I(r, i, false)
                }
                (RI::I(i), RI::R(r)) => {
                    self.emit(I::LessI(to, r, i as SignedBits9));
                    MM::I(r, i, true)
                }
                (RI::I(i), RI::I(i2)) => {
                    self.emit(if i < i2 {
                        I::LoadTrue(to)
                    } else {
                        I::LoadFalse(to)
                    });
                    MM::N
                }
            },
            BinOp::GreaterEqual => match self.without_k(left, right, &using_regs)? {
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::LessEqual(to, r, l));
                    MM::D(r, l)
                }
                (RI::R(r), RI::I(i)) => {
                    self.emit(I::GreaterEqualI(to, r, i as SignedBits9));
                    MM::I(r, i, false)
                }
                (RI::I(i), RI::R(r)) => {
                    self.emit(I::GreaterEqualI(to, r, i as SignedBits9));
                    MM::I(r, i, true)
                }
                (RI::I(i), RI::I(i2)) => {
                    self.emit(if i >= i2 {
                        I::LoadTrue(to)
                    } else {
                        I::LoadFalse(to)
                    });
                    MM::N
                }
            },
            BinOp::LessEqual => match self.without_k(left, right, &using_regs)? {
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::LessEqual(to, l, r));
                    MM::D(l, r)
                }
                (RI::R(r), RI::I(i)) => {
                    self.emit(I::LessEqualI(to, r, i as SignedBits9));
                    MM::I(r, i, false)
                }
                (RI::I(i), RI::R(r)) => {
                    self.emit(I::LessEqualI(to, r, i as SignedBits9));
                    MM::I(r, i, true)
                }
                (RI::I(i), RI::I(i2)) => {
                    self.emit(if i <= i2 {
                        I::LoadTrue(to)
                    } else {
                        I::LoadFalse(to)
                    });
                    MM::N
                }
            },
            BinOp::BitAnd => {
                let (l, r) = (self.imm_to_k(left)?, self.imm_to_k(right)?);
                match (l.1, r.1) {
                    (false, false) => {
                        self.emit(I::BitAnd(to, l.0, r.0));
                        MM::D(l.0, r.0)
                    }
                    (true, false) => {
                        self.emit(I::BitAndK(to, r.0, l.0));
                        MM::K(r.0, l.0 as Cst, true)
                    }
                    (false, true) => {
                        self.emit(I::BitAndK(to, l.0, r.0));
                        MM::K(l.0, r.0 as Cst, false)
                    }
                    _ => {
                        self.emit_loadk(to, l.0 as usize);
                        self.emit(I::BitAndK(to, to, r.0));
                        MM::K(to, r.0 as Cst, false)
                    }
                }
            }
            BinOp::BitOr => {
                let (l, r) = (self.imm_to_k(left)?, self.imm_to_k(right)?);
                match (l.1, r.1) {
                    (false, false) => {
                        self.emit(I::BitOr(to, l.0, r.0));
                        MM::D(l.0, r.0)
                    }
                    (true, false) => {
                        self.emit(I::BitOrK(to, r.0, l.0));
                        MM::K(r.0, l.0 as Cst, true)
                    }
                    (false, true) => {
                        self.emit(I::BitOrK(to, l.0, r.0));
                        MM::K(l.0, r.0 as Cst, false)
                    }
                    _ => {
                        self.emit_loadk(to, l.0 as usize);
                        self.emit(I::BitOrK(to, to, r.0));
                        MM::K(to, r.0 as Cst, false)
                    }
                }
            }
            BinOp::BitXor => {
                let (l, r) = (self.imm_to_k(left)?, self.imm_to_k(right)?);
                match (l.1, r.1) {
                    (false, false) => {
                        self.emit(I::BitXor(to, l.0, r.0));
                        MM::D(l.0, r.0)
                    }
                    (true, false) => {
                        self.emit(I::BitXorK(to, r.0, l.0));
                        MM::K(r.0, l.0 as Cst, true)
                    }
                    (false, true) => {
                        self.emit(I::BitXorK(to, l.0, r.0));
                        MM::K(l.0, r.0 as Cst, false)
                    }
                    _ => {
                        self.emit_loadk(to, l.0 as usize);
                        self.emit(I::BitXorK(to, to, r.0));
                        MM::K(to, r.0 as Cst, false)
                    }
                }
            }
            BinOp::ShiftL => match self.without_k(left, right, &using_regs)? {
                (RI::I(a), RI::I(b)) => {
                    self.emit(I::LoadI(to, (a << b) as SignedBits17));
                    MM::N
                }
                (RI::R(l), RI::I(i)) => {
                    self.emit(I::ShiftRI(to, l, -(i as SignedBits9)));
                    MM::I(l, -i, false)
                }
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::ShiftL(to, l, r));
                    MM::D(l, r)
                }
                (RI::I(i), RI::R(r)) => {
                    let l = addr(self.alloc_safely(&using_regs))?;
                    self.emit_loadi(l, i);
                    self.emit(I::ShiftL(to, l, r));
                    MM::D(l, r)
                }
            },
            BinOp::ShiftR => match self.without_k(left, right, &using_regs)? {
                (RI::I(a), RI::I(b)) => {
                    self.emit(I::LoadI(to, (a >> b) as SignedBits17));
                    MM::N
                }
                (RI::R(l), RI::I(i)) => {
                    self.emit(I::ShiftRI(to, l, i as SignedBits9));
                    MM::I(l, i, false)
                }
                (RI::R(l), RI::R(r)) => {
                    self.emit(I::ShiftR(to, l, r));
                    MM::D(l, r)
                }
                (RI::I(i), RI::R(r)) => {
                    let l = addr(self.alloc_safely(&using_regs))?;
                    self.emit_loadi(l, i);
                    self.emit(I::ShiftR(to, l, r));
                    MM::D(l, r)
                }
            },
            _ => {
                return Err(DukaDefaultError::UnsupportedFeature(format!(
                    "binary operator {}",
                    bin_op
                )));
            }
        };

        if !matches!(bin_op, BinOp::Equal | BinOp::NotEqual) // they are treated specially
            && let Some((meta, action)) = bin_op.get_meta_method()
        {
            let swap = matches!(action, MetaMethodAction::Swap);
            self.emit(match mm {
                MM::D(r, r2) => {
                    if swap {
                        I::MMBinary(r2, r, meta)
                    } else {
                        I::MMBinary(r, r2, meta)
                    }
                }
                MM::I(r, i, flip) => I::MMBinaryI(r, i as SignedBits8, meta, flip ^ swap),
                MM::K(r, k, flip) => I::MMBinaryK(r, k as Address, meta, flip ^ swap),
                MM::N => return Ok(()),
            });
        }

        Ok(())
    }

    fn gen_set_field(
        &mut self,
        tab: TablePlace,
        key: ValuePlace,
        val: ValuePlace,
    ) -> Result<(), DukaDefaultError> {
        let _: () = match tab {
            TablePlace::R(tab) => {
                let tab = addr(tab)?;
                match key {
                    ValuePlace::R(key) => {
                        let key = addr(key)?;
                        let (ad, kb) = self.imm_to_k(val)?;
                        self.emit(I::SetTable(tab, key, ad, kb));
                    }
                    ValuePlace::K(key) => {
                        let (ad, kb) = self.imm_to_k(val)?;
                        self.emit(I::SetField(tab, key as Address, ad, kb))
                    }
                    ValuePlace::I(i) => {
                        let (ad, kb) = self.imm_to_k(val)?;
                        if !i.is_negative()
                            && let Some(bits8) = I::MakeBits8(i as usize)
                        {
                            self.emit(I::SetI(tab, bits8, ad, kb))
                        } else {
                            let key = self.constants.push(ConstValue::Int(i));
                            self.emit(I::SetField(tab, key as Address, ad, kb))
                        }
                    }
                }
            }
            TablePlace::U(u) => match key {
                ValuePlace::R(key) => {
                    let key = addr(key)?;
                    let (ad, kb) = self.imm_to_k(val)?;
                    self.emit(I::SetTabUp(u as Address, key, ad, kb));
                }
                ValuePlace::K(key) => {
                    let (ad, kb) = self.imm_to_k(val)?;
                    self.emit(I::SetTabUpK(u as Address, key as Address, ad, kb))
                }
                ValuePlace::I(i) => {
                    let up = u as Address;
                    let (ad, kb) = self.imm_to_k(val)?;
                    if !i.is_negative()
                        && let Some(bits8) = I::MakeBits8(i as usize)
                    {
                        self.emit(I::SetTabUpI(up, bits8, ad, kb))
                    } else {
                        let key = self.constants.push(ConstValue::Int(i));
                        self.emit(I::SetTabUpK(up, key as Address, ad, kb))
                    }
                }
            },
        };
        Ok(())
    }

    fn gen_get_field(
        &mut self,
        to: usize,
        from: TablePlace,
        who: ValuePlace,
    ) -> Result<(), DukaDefaultError> {
        let to = addr(to)?;
        let _: () = match from {
            TablePlace::U(tab) => match who {
                ValuePlace::I(i) => {
                    self.emit(I::GetUpVal(to, tab as Bits17));
                    if i.is_positive()
                        && let Some(idx) = I::MakeBits9(i as usize)
                    {
                        self.emit(I::GetI(to, to, idx));
                    } else {
                        let k = self.constants.push(ConstValue::Int(i));
                        self.emit(I::GetField(to, to, k as Bits9))
                    }
                }
                ValuePlace::K(k) => self.emit(I::GetTabUp(to, tab as Address, k as Bits9)),
                ValuePlace::R(r) => {
                    self.emit(I::GetUpVal(to, tab as Bits17));
                    self.emit(I::GetTable(to, to, addr(r)?))
                }
            },
            TablePlace::R(tab) => match who {
                ValuePlace::I(i) => {
                    if i.is_positive()
                        && let Some(idx) = I::MakeBits9(i as usize)
                    {
                        self.emit(I::GetI(to, addr(tab)?, idx));
                    } else {
                        let k = self.constants.push(ConstValue::Int(i));
                        self.emit(I::GetField(to, addr(tab)?, k as Bits9))
                    }
                }
                ValuePlace::K(k) => self.emit(I::GetField(to, addr(tab)?, k as Bits9)),
                ValuePlace::R(r) => self.emit(I::GetTable(to, addr(tab)?, addr(r)?)),
            },
        };
        Ok(())
    }

    fn gen_proto(mut self, duka_ir: DukaIR) -> Result<DukaProto, DukaDefaultError> {
        let DukaIR {
            param_count,
            reg_lifetime,
            has_var_arg,
            instructions,
            nesteds,
            constants,
            up_indexes,
            debug_info,
            ..
        } = duka_ir;
        self.constants = *constants;
        self.debug_info = *debug_info;

        if has_var_arg {
            self.emit(I::VarArgPrepare(param_count as Bits25));
        }
        self.gen_irs(instructions, reg_lifetime.using)?;
        let nested_protos = nesteds
            .into_iter()
            .map(|di| Self::new().gen_proto(di))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DukaProto {
            up_indexes,
            constants: self.constants.into_vec().into(),
            instructions: self.instructions.into(),
            used_reg_count: reg_lifetime.count,
            nested_protos: nested_protos.into(),
            param_count,
            has_var_arg,
            debug_info: Box::new(self.debug_info),
            logic: None,
        })
    }
}

impl DefaultGenerator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            self_params: vec![],
            pending: vec![],
            labels: HashMap::new(),
        }
    }
}

impl DukaGenerator<DukaProto, DukaDefaultError> for DefaultGenerator {
    type InputType = DukaIR;
    type ConfigType = ();

    fn generate(ir: Self::InputType, _: Self::ConfigType) -> Result<DukaProto, DukaDefaultError> {
        Self::new().gen_proto(ir)
    }
}
