#[derive(Debug, Clone)]
pub enum DukaWasmError {
    UnsupportedFeature(String),
    InvalidAddress(usize),
    InvalidJumpPosition { from: usize, to: usize },
    InvalidRegister(usize),
    MemoryAllocationFailed,
    WasmGenerationFailed(String),
}
