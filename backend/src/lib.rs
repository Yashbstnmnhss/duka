pub mod codegen;
pub mod error;
pub mod types;
pub mod vm;

#[cfg(test)]
mod tests {

    use crate::codegen::binary::DukaDumpError;

    #[test]
    fn instruction_macro_test() {
        use crate::vm::instructions::{DecodeInstruction, Instruction, InstructionName};

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
