use duka_macros::ThatError;

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    #[error("The step cannot be zero in a for loop")]
    ZeroStepInForLoop,
    #[error("The previous instruction must be ExtraArg")]
    ExtraArgNotFound,
    #[error("Accessing out of valid index range in stack")]
    OutOfStack,
    #[error("Read unimplemented instruction")]
    UnimplementedInstruction,
    #[error("Unsupported operation: {} on {}")]
    UnsupportedOperation(&'static str, &'static str),
    #[error("Invalid type of value: expected {}")]
    InvalidValueType(&'static str),
}
