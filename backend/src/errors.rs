use duka_macros::ThatError;
use duka_shared::errors::Span;

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

/// 外层错误类型：携带运行时错误与其调用栈 trace
#[derive(Debug, Clone)]
pub struct DukaTraceError {
    pub kind: DukaRuntimeError,
    pub trace: DukaStackTrace,
}

#[derive(Debug, Clone, Default)]
pub struct DukaStackTrace {
    pub frames: Vec<DukaTraceFrame>,
}
#[derive(Debug, Clone)]
pub struct DukaTraceFrame {
    pub debug_name: Option<Box<str>>,
    pub span: Option<Span>,
    pub is_native: bool,
}

impl std::fmt::Display for DukaStackTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.frames.is_empty() {
            return Ok(());
        }
        for frame in &self.frames {
            let name = frame.debug_name.as_deref();
            if frame.is_native {
                writeln!(f, "    at <{}>", name.unwrap_or("native"))?;
                continue;
            }
            match frame.span {
                Some(span) => writeln!(f, "    at <{}>:{}", name.unwrap_or("anonymous"), span)?,
                None => writeln!(f, "    at <{}>", name.unwrap_or("anonymous"))?,
            }
        }
        Ok(())
    }
}

impl std::fmt::Display for DukaTraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)?;
        if !self.trace.frames.is_empty() {
            write!(f, "\n  stack traceback:\n{}", self.trace)?;
        }
        Ok(())
    }
}

impl std::error::Error for DukaTraceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}
