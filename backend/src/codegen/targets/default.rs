use std::collections::HashMap;

use duka_shared::{
    ast::{BinOp, UnOp},
    ir::{Constants, DukaIR, IR, Lab, Place, RKI},
    types::{DebugInfo, DukaGenerator, ValueCount},
    value::{ConstValue, DukaInt},
};

use crate::{
    codegen::errors::DukaDefaultError,
    instructions::{Address, Bits9, Bits17, Bits25, Instruction as I, SignedBits17, SignedBits25},
    value::DukaProto,
};

#[derive(Debug, Default)]
pub struct Generator {
    constants: Constants,
    debug_info: DebugInfo,
    instructions: Vec<I>,

    self_params: Vec<usize>,
    labels: HashMap<Lab, usize>,
    pending_gotos: Vec<(usize, Lab)>,
}

#[inline]
fn addr(n: usize) -> Result<Address, DukaDefaultError> {
    I::MakeAddress(n).ok_or(DukaDefaultError::InvalidAddress(n))
}

#[inline]
fn offset(from: usize, to: usize) -> Result<SignedBits25, DukaDefaultError> {
    let val = to as isize - from as isize;
    I::MakeSignedBits25(val).ok_or(DukaDefaultError::InvalidJumpPosition { from, to })
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

    fn rki_to_addr(&mut self, pl: RKI, if_not: Address) -> Result<Address, DukaDefaultError> {
        Ok(match pl {
            RKI::I(i) => {
                self.emit_loadi(if_not, i);
                if_not
            }
            RKI::K(k) => {
                self.emit_loadk(if_not, k);
                if_not
            }
            RKI::R(r) => addr(r)?,
        })
    }

    fn emit_jump(&mut self, label: Lab) -> Result<(), DukaDefaultError> {
        if let Some(to) = self.labels.get(&label) {
            self.emit(I::Jump(offset(self.instructions.len(), *to)?));
        } else {
            let at = self.emit_placeholder();
            self.pending_gotos.push((at, label));
        }
        Ok(())
    }

    fn do_irs(&mut self, irs: Vec<IR>) -> Result<(), DukaDefaultError> {
        let mut iter = irs.into_iter().peekable();

        macro_rules! take {
            ($ir: expr) => {{
                let Some(el @ IR::TakeAll | el @ IR::Take(..)) = iter.next() else {
                    return Err(DukaDefaultError::ExpectedTake($ir));
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
                IR::Void => (),
                IR::Move(to, from) => self.emit(I::Move(addr(to)?, addr(from)?)),
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
                IR::GetField(to, from, who) => {
                    let to = addr(to)?;
                    match from {
                        Place::U(tab) => match who {
                            RKI::I(i) => {}
                            RKI::K(k) => self.emit(I::GetTabUp(to, addr(tab)?, k as Bits9)),
                            RKI::R(r) => {}
                        },
                        Place::R(tab) => match who {
                            RKI::I(i) => {
                                if i.is_positive()
                                    && let Some(idx) = I::MakeBits9(i as usize)
                                {
                                    self.emit(I::GetI(to, addr(tab)?, idx));
                                } else {
                                    let k = self.constants.add(ConstValue::Int(i));
                                    self.emit(I::GetField(to, addr(tab)?, k as Bits9))
                                }
                            }
                            RKI::K(k) => self.emit(I::GetField(to, addr(tab)?, k as Bits9)),
                            RKI::R(r) => self.emit(I::GetTable(to, addr(tab)?, addr(r)?)),
                        },
                        _ => unreachable!(),
                    }
                }
                IR::SetField(tab, key, val) => {}
                IR::NewTable(to) => self.emit(I::NewTable(addr(to)?, 0, false)),
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
                    let returns = take!("Call".to_owned());
                    self.emit(I::Call(addr(callee)?, params.into(), returns.into()))
                }
                IR::TailCall(callee, params) => {
                    if let Some(at) = self.self_params.pop() {
                        self.emit_fixup(at, I::Self_(addr(callee)?, 0, 0, true));
                    }
                    let returns = take!("TailCall".to_owned());
                    self.emit(I::TailCall(addr(callee)?, params.into(), returns.into()))
                }
                IR::Closure(to, idx) => self.emit(I::Closure(addr(to)?, idx as Bits17)),
                IR::Return(from, value_count) => self.emit(if value_count.is_empty() {
                    I::Return0()
                } else {
                    I::Return(addr(from)?, value_count.into())
                }),
                IR::VarArg(to) => {
                    let count = take!("VarArg".to_owned());
                    self.emit(I::VarArg(addr(to)?, count.into()));
                }
                IR::Spawn(to, from) => self.emit(I::Spawn(addr(to)?, addr(from)?)),
                IR::Go(callee, params) => {
                    let returns = take!("Go".to_owned());
                    self.emit(I::Go(addr(callee)?, params.into(), returns.into()))
                }
                IR::Yield(from, count) => {
                    let returns = take!("Yield".to_owned());
                    self.emit(I::Yield(addr(from)?, count.into(), returns.into()))
                }
                IR::Unary(to, place, un_op) => {
                    let to = addr(to)?;
                    let from = self.rki_to_addr(place, to)?;
                    self.emit(match un_op {
                        UnOp::Length => I::Length(to, from),
                        UnOp::Not => I::Not(to, from),
                        UnOp::BitNot => I::BitNot(to, from),
                        UnOp::Minus => I::Minus(to, from),
                    });
                }
                IR::Binary(to, left, right, bin_op) => {
                    let to = addr(to)?;
                    match bin_op {
                        BinOp::Add => todo!(),
                        BinOp::Sub => todo!(),
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
                        BinOp::Concat => todo!(),
                        _ => {
                            return Err(DukaDefaultError::UnsupportedFeature(format!(
                                "binary operator {}",
                                bin_op
                            )));
                        }
                    }
                }
                IR::Label(label) => {
                    self.labels.insert(label, self.instructions.len());
                }
                IR::Jump(label) => self.emit_jump(label)?,
                IR::ForPrep(from, _) => {}
                IR::ForLoop(_, _) => {}
                IR::TForPrep(_, _) => {}
                IR::TForCall(_, _) => todo!(),
                IR::TForLoop(_, _) => todo!(),
                IR::SkipNext(cond, what) => self.emit(I::Test(addr(cond)?, what)),
                IR::Take(_) | IR::TakeAll => return Err(DukaDefaultError::AloneTake),
                IR::SysCall(_) => unimplemented!(),
            }
        }

        while let Some((at, to)) = self.pending_gotos.pop() {
            self.emit_fixup(at, I::Jump(offset(at, to)?));
        }

        Ok(())
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
        self.do_irs(instructions)?;
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
            pending_gotos: vec![],
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
