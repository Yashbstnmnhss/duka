use duka_macros::ThatError;

#[derive(Debug, Clone, PartialEq, ThatError)]
pub enum DukaRuntimeError {
    ExtraArgNotFound,
    OutOfStack,
}
