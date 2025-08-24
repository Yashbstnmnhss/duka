use duka_macros::ThatError;

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    #[error("The previous instruction must be ExtraArg")]
    ExtraArgNotFound,
    #[error("Accessing out of valid index range in stack")]
    OutOfStack,
}
