//! Backend of Duka  
//!
//! Including codegen, binary, virtual machine, runtime value

use std::{collections::HashMap, ops::Range};

use duka_macros::{Info, 史書云};
use duka_shared::{error::Span, utils::SemVer};

use crate::{error::DukaRuntimeError, value::DukaProto};

pub mod builtin;
pub mod codegen;
pub mod error;
pub mod instructions;
pub mod logic_instructions;
pub mod value;
pub mod vm;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugInfo {
    pub inst_spans: HashMap<Range<usize>, Span>,
    pub all_span: Span,
    pub debug_name: Option<String>,
}

#[derive(Info, Debug, Clone, PartialEq)]
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

    use duka_shared::value::ConstValue;

    use crate::{
        codegen::binary::{DukaBinary, DukaDumpError, Dumplings},
        instructions::Instruction,
        value::{DukaProto, UpIndex, UpValueKind},
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
        macro_rules! ins {
            ($n: ident ($($p: expr),*)) => {
                I::$n($($p),*)
            };
            ($($n: ident ($($p: expr),*));+) => {
                vec![$(ins!($n($($p),*))),+]
            }
        }

        let instructions = ins! {
            VarArgPrepare(2);
            AddI(1, 1, -1);
            GetTabUp(0, 0, 2);
            LoadI(1, 1);
            LoadI(2, 2);
            Call(0, 3, 3);
            SetTabUp(0, 1, 1, false);
            SetTabUp(0, 0, 0, false);
            Return(0, 1);
            Go(1, 2, 3);
            Yield(2, 2, 5)
        };
        for i in &instructions {
            println!("{i}");
        }

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
            up_indexes: vec![UpIndex {
                name: None,
                local: true,
                index: 2,
                kind: UpValueKind::Regular,
            }],
            constants: vec![ConstValue::Int(114514)],
            instructions: vec![Instruction::Move(1, 2), Instruction::Add(1, 2, 3)],
            nested_protos: vec![],
            has_var_arg: true,
            param_count: 5,
            reg_count: 10,
            debug_info: crate::DebugInfo::default(),
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
