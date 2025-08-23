pub mod codegen;
pub mod error;
pub mod types;
pub mod vm;

#[cfg(test)]
mod tests {
    use crate::vm::instructions::{DecodeInstruction, Instruction, InstructionName};

    #[test]
    fn instruction_macro_test() {
        let i = Instruction::Move(1, 2);
        assert_eq!(i.decode(), DecodeInstruction::Move(1, 2));
        assert_eq!(i.name(), InstructionName::Move);
        assert_eq!(i.check_setA(), true);
        assert_eq!(Instruction::validate(i.raw()), true);
        let i = Instruction::LoadI(1, -2);
        assert_eq!(i.decode(), DecodeInstruction::LoadI(1, -2));
    }
}
