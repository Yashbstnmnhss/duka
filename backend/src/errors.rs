use duka_macros::ThatError;

use crate::vm::coroutine::CoroutineID;

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    #[error("{}")]
    Custom(String),

    #[error("No call frame found")]
    NoCallFrame,
    #[error("No {} key found in {}")]
    NoSuchKey(String, &'static str),
    #[error("Unable to run this coroutine: {}")]
    UnableRunCoroutine(CoroutineID),
    #[error("Step cannot be zero in a for loop")]
    ZeroStepInForLoop,
    #[error("Previous instruction must be ExtraArg")]
    ExtraArgNotFound,
    #[error("Accessing out of valid index range in {}")]
    OutOfRange(&'static str),
    #[error("Read unimplemented meta_method: {}")]
    UnimplementedMetamethod(String),
    #[error("Read invalid instruction: {}")]
    InvalidInstruction(&'static str),
    #[error("Unsupported operation: {} on {}")]
    UnsupportedOperation(&'static str, &'static str),
    #[error("Unsupported meta_method: {} in {}")]
    NoSuchMetamethod(&'static str, String),
    /// (Expected!)
    #[error("Invalid type of value: expected {}")]
    InvalidValueType(&'static str),
    #[error("Cannot divided by zero")]
    DividedByZero,

    #[error("Module error: {}")]
    ModuleError(String),
    #[error("Argument missing at {} for {}, expected {}")]
    ArgumentMissing(usize, String, String),
    #[error("Argument at {} for {} is not {}, got {}")]
    ArgumentInvalidType(usize, String, &'static str, &'static str),
}
