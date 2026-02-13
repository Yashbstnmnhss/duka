use duka_macros::ThatError;

#[derive(ThatError, Debug)]
pub enum DukaDefaultError {
    #[error("Unsupported feature: {}")]
    UnsupportedFeature(String),
    #[error("Invalid address: {}")]
    InvalidAddress(usize),
    #[error("Invalid jumping position: from {} to {}")]
    InvalidJumpPosition { from: usize, to: usize },
    #[error("No 'take' found after {}")]
    ExpectedTake(String),
    #[error("Found alone 'take'")]
    AloneTake,
}
