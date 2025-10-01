use duka_macros::史書云;
use duka_shared::utils::SemVer;

use crate::{error::DukaRuntimeError, value::DukaProto};

pub mod codegen;
pub mod error;
pub mod instructions;
pub mod value;
pub mod vm;

pub trait Executable {
    type ReturnType;

    fn execute(&mut self) -> Result<Self::ReturnType, DukaRuntimeError>;
}

pub trait DukaVM {
    type OkType;

    fn execute(&mut self, proto: &DukaProto) -> Result<Self::OkType, DukaRuntimeError>;
}
pub trait DukaRuntime {}

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

    use crate::codegen::binary::DukaDumpError;

    #[test]
    fn instruction_macro_test() {
        use crate::instructions::{DecodeInstruction, Instruction, InstructionName};

        let i = Instruction::Move(1, 2);
        assert_eq!(i.decode(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name(), InstructionName::Move);
        assert_eq!(i.check_setA(), true);
        assert_eq!(Instruction::validate(i.raw()), true);
        let i = Instruction::LoadI(1, -2);
        assert_eq!(i.decode(), DecodeInstruction::LoadI(1, -2));
    }

    #[test]
    fn dumplings_test() -> Result<(), DukaDumpError> {
        use crate::codegen::binary::*;

        let header = DukaBinaryHeader {};
        let mut output: Vec<u8> = vec![];

        header.dl_write(&mut output)?;

        assert_eq!(output, [68, 85, 75, 65, 1, 1, 0, 8, 8, 4]);
        Ok(())
    }
}
