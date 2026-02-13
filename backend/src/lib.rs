//! Backend of Duka  
//!
//! Including codegen, binary, virtual machine, runtime value

use duka_macros::{Info, 史書云};
use duka_shared::utils::SemVer;

use crate::{errors::DukaRuntimeError, value::DukaProto};

pub mod builtin;
pub mod codegen;
pub mod errors;
pub mod instructions;
pub mod logic_instructions;
pub mod value;
pub mod vm;

#[derive(Info, Debug, Clone, PartialEq)]
#[non_exhaustive]
#[idcard(u8)]
pub enum SysCallId {
    Logic,
}

pub trait Executable {
    type ReturnType;

    fn execute(&mut self) -> Result<Self::ReturnType, DukaRuntimeError>;
}

pub trait DukaVM {
    type OkType;

    fn execute(&mut self, proto: &DukaProto) -> Result<Self::OkType, DukaRuntimeError>;
}

pub const VERSION: SemVer = 史書云! {
    <<後端>> 者
    為 世家 "項目之創立" 也
    為 世家 "Instruction之完善" 也
    為 世家 "虛擬機之創立" 也
    為 世家 "Dumplings之嘗試" 也
    為 世家 "SemVer及其宏之創立" 也
    為 列傳 "Dumplings讀寫bug" 也
};

#[cfg(test)]
mod tests {

    use std::io::Cursor;

    use duka_shared::{
        ir::{UpIndex, UpValueKind},
        types::DebugInfo,
        value::ConstValue,
    };

    use crate::{
        codegen::binary::{DukaBinary, DukaDumpError, Dumplings},
        instructions::Instruction,
        value::DukaProto,
    };

    #[test]
    fn split_test() {
        let a: u16 = 12;
        let b: u16 = 23;
        println!("{a:b} & {b:b}");
        let r = ((a as u32) << 16) | (b as u32);
        println!("{r:b}");
        assert_eq!(a as u32, (r & ((u16::MAX as u32) << 16)) >> 16);
        assert_eq!(b as u32, r & (u16::MAX as u32));
    }

    #[test]
    fn instruction_macro_test() {
        use crate::instructions::{DecodeInstruction, Instruction as I, InstructionName};

        let i = I::Move(1, 2);
        assert_eq!(i.decode().unwrap(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name().unwrap(), InstructionName::Move);
        assert!(i.check_setA().unwrap());
        assert!(I::validate(i.raw()));
        let i = I::LoadI(1, -2);
        assert_eq!(i.decode().unwrap(), DecodeInstruction::LoadI(1, -2));
    }

    #[test]
    fn dumpling_header_test() -> Result<(), DukaDumpError> {
        use crate::codegen::binary::*;

        let header = DukaBinaryHeader {};
        let mut output: Vec<u8> = vec![];

        header.dl_write(&mut output)?;

        let header2 = DukaBinaryHeader::dl_read(&mut Cursor::new(&output))?;
        println!("{:?}", header2);

        assert_eq!(output, [68, 85, 75, 65, 1, 0, 0, 5, 1, 8, 8, 4]);
        Ok(())
    }

    #[test]
    fn dumpling_proto_test() -> Result<(), DukaDumpError> {
        let proto = DukaProto {
            up_indexes: [UpIndex {
                name: None,
                local: true,
                index: 2,
                kind: UpValueKind::Regular,
            }]
            .into(),
            constants: [ConstValue::Int(114514)].into(),
            instructions: [Instruction::Move(1, 2), Instruction::Add(1, 2, 3)].into(),
            nested_protos: Box::default(),
            has_var_arg: true,
            param_count: 5,
            used_reg_count: 10,
            debug_info: Box::new(DebugInfo::default()),
            logic: None,
        };
        let binary = DukaBinary::new(proto);
        let mut output = vec![];
        binary.dl_write(&mut output)?;
        println!("{:?}", output);

        let binary2 = DukaBinary::dl_read(&mut Cursor::new(&output))?;
        assert_eq!(binary, binary2);
        Ok(())
    }
}
