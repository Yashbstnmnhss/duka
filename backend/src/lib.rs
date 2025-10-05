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
            VarArgPrepare(0);
            GetTabUp(0,0,2);
            LoadI(1,1);
            LoadI(2,2);
            Call(0,3,3);
            SetTabUp(0,1,1,false);
            SetTabUp(0,0,0,false);
            Return(0,1)
        };
        for i in &instructions {
            println!("{i}");
        }

        let i = I::Move(1, 2);
        assert_eq!(i.decode(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name(), InstructionName::Move);
        assert_eq!(i.check_setA(), true);
        assert_eq!(I::validate(i.raw()), true);
        let i = I::LoadI(1, -2);
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
