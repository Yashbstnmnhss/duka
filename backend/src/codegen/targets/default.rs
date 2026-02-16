use std::{collections::HashMap, fmt::Debug};

use duka_shared::{
    ast::{BinOp, UnOp},
    constants::MetaMethod,
    ir::{Constants, Cst, DukaIR, IR, Lab, ModifiablePlace, Reg, ValuePlace},
    types::{DebugInfo, DukaGenerator, ValueCount},
    value::{ConstValue, DukaInt},
};

use crate::{
    codegen::errors::DukaDefaultError,
    instructions::{
        Address, Bits9, Bits17, Bits25, Instruction as I, SignedBits8, SignedBits9, SignedBits25,
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
pub struct Generator {
    constants: Constants,
    debug_info: DebugInfo,
    instructions: Vec<I>,

    self_params: Vec<usize>,
    labels: HashMap<Lab, usize>,
    pendings: Vec<JumpPending>,
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

impl Generator {
    #[inline]
    fn emit_loadi(&mut self, to: Address, what: DukaInt) {
        if let Some(res) = I::MakeSignedBits17(what as isize) {
            self.emit(I::LoadI(to, res))
        } else {
            let k = self.constants.add(ConstValue::Int(what));
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

    fn val_to_addr(
        &mut self,
        pl: ValuePlace,
        if_not: Address,
    ) -> Result<Address, DukaDefaultError> {
        Ok(match pl {
            ValuePlace::I(i) => {
                self.emit_loadi(if_not, i);
                if_not
            }
            ValuePlace::K(k) => {
                self.emit_loadk(if_not, k);
                if_not
            }
            ValuePlace::R(r) => addr(r)?,
        })
    }

    fn r_or_k(&mut self, val: ValuePlace) -> Result<(Address, bool), DukaDefaultError> {
        Ok(match val {
            ValuePlace::R(r) => (addr(r)?, false),
            ValuePlace::K(k) => (addr(k)?, true),
            ValuePlace::I(i) => {
                let k: usize = self.constants.add(ConstValue::Int(i));
                (addr(k)?, true)
            }
        })
    }

    fn emit_jump(&mut self, label: Lab) -> Result<(), DukaDefaultError> {
        if let Some(to) = self.labels.get(&label) {
            self.emit(I::Jump(offset_jump(self.instructions.len(), *to)?));
        } else {
            let at = self.emit_placeholder();
            self.pendings.push(JumpPending {
                label,
                at,
                constructor: Box::new(move |to| {
                    let offset = offset_jump(at, to as usize)?;
                    Ok(I::Jump(offset))
                }),
            });
        }
        Ok(())
    }

    fn gen_irs(&mut self, irs: Vec<IR>) -> Result<(), DukaDefaultError> {
        let mut iter = irs.into_iter().peekable();

        macro_rules! take {
            ($ir: expr) => {{
                let Some(el @ IR::TakeAll | el @ IR::Take(..)) = iter.next() else {
                    return Err(DukaDefaultError::ExpectedTake($ir.into()));
                };
                match el {
                    IR::TakeAll => ValueCount::VarArg,
                    IR::Take(n) => ValueCount::Exact(n),
                    _ => unreachable!(),
                }
            }};
        }

        while let Some(ir) = iter.next() {
            match ir {
                IR::Void => continue,
                IR::Move(to, from) => {
                    self.emit(I::Move(addr(to)?, addr(from)?));
                }
                IR::LoadNil(to) => {
                    let count = iter
                        .by_ref()
                        .enumerate()
                        .take_while(|(i, ir)| matches!(ir, IR::LoadNil(t) if *t == i + to + 1))
                        .count()
                        + 1;
                    self.emit(I::LoadNil(addr(to)?, count as Bits17))
                }
                IR::LoadTrue(to) => self.emit(I::LoadTrue(addr(to)?)),
                IR::LoadFalse(to) => self.emit(I::LoadFalse(addr(to)?)),
                IR::LoadConst(to, k) => self.emit_loadk(addr(to)?, k),
                IR::LoadFloat(to, fl) => {
                    let k = self.constants.add(ConstValue::Float(fl));
                    self.emit_loadk(addr(to)?, k);
                }
                IR::LoadInt(to, what) => {
                    self.emit_loadi(addr(to)?, what);
                }
                IR::LoadString(to, str) => {
                    let k = self.constants.add(ConstValue::String(str));
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
                    let from = self.val_to_addr(place, to)?;
                    self.emit(match un_op {
                        UnOp::Length => I::Length(to, from),
                        UnOp::Not => I::Not(to, from),
                        UnOp::BitNot => I::BitNot(to, from),
                        UnOp::Minus => I::Minus(to, from),
                    });
                }
                IR::Binary(to, left, right, bin_op) => self.gen_binary(to, left, right, bin_op)?,
                IR::Concat(to, from, count) => {
                    let (to, from) = (addr(to)?, addr(from)?);
                    self.emit(I::Concat(from, count as Bits17));
                    if to != from {
                        self.emit(I::Move(to, from));
                    }
                }
                IR::Label(label) => {
                    self.labels.insert(label, self.instructions.len());
                }
                IR::Jump(label) => self.emit_jump(label)?,
                IR::ForPrep(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pendings.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::ForPrepare(a, offset_for(at, to)?))),
                    })
                }
                IR::ForLoop(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pendings.push(JumpPending {
                        label,
                        at,
                        constructor: Box::new(move |to| Ok(I::ForLoop(a, offset_for(at, to)?))),
                    })
                }
                IR::TForPrep(a, label) => {
                    let a = addr(a)?;
                    let at = self.emit_placeholder();
                    self.pendings.push(JumpPending {
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
                    self.pendings.push(JumpPending {
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
        }) = self.pendings.pop()
        {
            let to = *self
                .labels
                .get(&label)
                .ok_or(DukaDefaultError::UnsolvedLabel)?;
            self.emit_fixup(at, constructor(to)?);
        }

        Ok(())
    }

    fn check_imm(&mut self, vp: ValuePlace) -> ValuePlace {
        if let ValuePlace::I(i) = vp
            && I::MakeSignedBits8(i as isize).is_none()
        {
            let k = self.constants.add(ConstValue::Int(i));
            ValuePlace::K(k)
        } else {
            vp
        }
    }

    fn gen_binary(
        &mut self,
        to: usize,
        left: ValuePlace,
        right: ValuePlace,
        bin_op: BinOp,
    ) -> Result<(), DukaDefaultError> {
        enum MM {
            D(Address, Address),
            I(Address, DukaInt, bool),
            K(Address, Cst, bool),
            N,
        }

        let to = addr(to)?;
        let (left, right) = (self.check_imm(left), self.check_imm(right));

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
                    MM::I(to, i, true)
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
            BinOp::Equal => todo!(),
            BinOp::NotEqual => todo!(),
            BinOp::Greater => todo!(),
            BinOp::Less => todo!(),
            BinOp::GreaterEqual => todo!(),
            BinOp::LessEqual => todo!(),
            BinOp::BitAnd => todo!(),
            BinOp::BitOr => todo!(),
            BinOp::BitXor => todo!(),
            BinOp::ShiftL => todo!(),
            BinOp::ShiftR => todo!(),
            _ => {
                return Err(DukaDefaultError::UnsupportedFeature(format!(
                    "binary operator {}",
                    bin_op
                )));
            }
        };

        if let Some(meta) = MetaMethod::from_binop(bin_op) {
            self.emit(match mm {
                MM::D(r, r2) => I::MMBinary(r, r2, meta),
                MM::I(r, i, flip) => I::MMBinaryI(r, i as SignedBits8, meta, flip),
                MM::K(r, k, flip) => I::MMBinaryK(r, k as Address, meta, flip),
                MM::N => return Ok(()),
            });
        }

        Ok(())
    }

    fn gen_set_field(
        &mut self,
        tab: ModifiablePlace,
        key: ValuePlace,
        val: ValuePlace,
    ) -> Result<(), DukaDefaultError> {
        Ok(match tab {
            ModifiablePlace::R(tab) => match key {
                ValuePlace::R(key) => {
                    let tab = addr(tab)?;
                    let key = addr(key)?;
                    match val {
                        ValuePlace::R(_) => todo!(),
                        ValuePlace::K(_) => todo!(),
                        ValuePlace::I(_) => todo!(),
                    }
                    self.emit(I::SetTable(tab, key, todo!(), false));
                }
                ValuePlace::K(key) => match val {
                    ValuePlace::R(_) => todo!(),
                    ValuePlace::K(_) => todo!(),
                    ValuePlace::I(_) => todo!(),
                },
                ValuePlace::I(key) => match val {
                    ValuePlace::R(_) => todo!(),
                    ValuePlace::K(_) => todo!(),
                    ValuePlace::I(_) => todo!(),
                },
            },
            ModifiablePlace::U(u) => match key {
                ValuePlace::R(_) => {}
                ValuePlace::K(key) => match val {
                    ValuePlace::R(_) => todo!(),
                    ValuePlace::K(_) => todo!(),
                    ValuePlace::I(_) => todo!(),
                },
                ValuePlace::I(_) => match val {
                    ValuePlace::R(_) => todo!(),
                    ValuePlace::K(_) => todo!(),
                    ValuePlace::I(_) => todo!(),
                },
            },
        })
    }

    fn gen_get_field(
        &mut self,
        to: usize,
        from: ModifiablePlace,
        who: ValuePlace,
    ) -> Result<(), DukaDefaultError> {
        let to = addr(to)?;
        Ok(match from {
            ModifiablePlace::U(tab) => match who {
                ValuePlace::I(i) => {
                    self.emit(I::GetUpVal(to, tab as Bits17));
                    if i.is_positive()
                        && let Some(idx) = I::MakeBits9(i as usize)
                    {
                        self.emit(I::GetI(to, to, idx));
                    } else {
                        let k = self.constants.add(ConstValue::Int(i));
                        self.emit(I::GetField(to, to, k as Bits9))
                    }
                }
                ValuePlace::K(k) => self.emit(I::GetTabUp(to, tab as Address, k as Bits9)),
                ValuePlace::R(r) => {
                    self.emit(I::GetUpVal(to, tab as Bits17));
                    self.emit(I::GetTable(to, to, addr(r)?))
                }
            },
            ModifiablePlace::R(tab) => match who {
                ValuePlace::I(i) => {
                    if i.is_positive()
                        && let Some(idx) = I::MakeBits9(i as usize)
                    {
                        self.emit(I::GetI(to, addr(tab)?, idx));
                    } else {
                        let k = self.constants.add(ConstValue::Int(i));
                        self.emit(I::GetField(to, addr(tab)?, k as Bits9))
                    }
                }
                ValuePlace::K(k) => self.emit(I::GetField(to, addr(tab)?, k as Bits9)),
                ValuePlace::R(r) => self.emit(I::GetTable(to, addr(tab)?, addr(r)?)),
            },
        })
    }

    fn gen_proto(mut self, duka_ir: DukaIR) -> Result<DukaProto, DukaDefaultError> {
        let DukaIR {
            param_count,
            used_reg_count,
            has_var_arg,
            instructions,
            nesteds,
            constants,
            up_indexes,
            debug_info,
            label_names: _,
            logic,
        } = duka_ir;
        self.constants = *constants;
        self.debug_info = *debug_info;

        if has_var_arg {
            self.emit(I::VarArgPrepare(param_count as Bits25));
        }
        self.gen_irs(instructions)?;
        let nested_protos = nesteds
            .into_iter()
            .map(|di| Self::new().gen_proto(di))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DukaProto {
            up_indexes,
            constants: self.constants.into_vec().into(),
            instructions: self.instructions.into(),
            used_reg_count,
            nested_protos: nested_protos.into(),
            param_count,
            has_var_arg,
            debug_info: Box::new(self.debug_info),
            logic: None,
        })
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            self_params: vec![],
            pendings: vec![],
            labels: HashMap::new(),
        }
    }
}

impl DukaGenerator<DukaProto, DukaDefaultError> for Generator {
    type InputType = DukaIR;

    fn generate(ir: Self::InputType) -> Result<DukaProto, DukaDefaultError> {
        Self::new().gen_proto(ir)
    }
}
