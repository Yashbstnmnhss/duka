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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAnalyzerConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAdapterConfig {}
