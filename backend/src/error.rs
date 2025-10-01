use duka_macros::ThatError;

use crate::vm::coroutine::CoroutineID;

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    #[error("{}")]
    Custom(String),

    #[error("No call frame found")]
    NoCallFrame,
    #[error("Unable to run this coroutine: {}")]
    UnableRunCoroutine(CoroutineID),
    #[error("Step cannot be zero in a for loop")]
    ZeroStepInForLoop,
    #[error("Previous instruction must be ExtraArg")]
    ExtraArgNotFound,
    #[error("Accessing out of valid index range in stack")]
    OutOfStack,
    #[error("Read unimplemented instruction")]
    UnimplementedInstruction,
    #[error("Unsupported operation: {} on {}")]
    UnsupportedOperation(&'static str, &'static str),
    /// (Expected!)
    #[error("Invalid type of value: expected {}")]
    InvalidValueType(&'static str),
}
