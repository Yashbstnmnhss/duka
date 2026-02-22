//! Configuration for Duka

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaConfig {
    pub parser: DukaParserConfig,
    pub analyzer: DukaAnalyzerConfig,
    pub adapter: DukaAdapterConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaParserConfig {
    pub use_stmt_expr: bool,
    pub use_bang_expr: bool,
    pub use_bang_stmt: bool,
}
impl Default for DukaParserConfig {
    fn default() -> Self {
        Self {
            use_bang_expr: true,
            use_bang_stmt: true,
            use_stmt_expr: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAnalyzerConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAdapterConfig {
    do_inline_adapt: bool,
}

impl Default for DukaAdapterConfig {
    fn default() -> Self {
        Self {
            do_inline_adapt: true,
        }
    }
}
