//! Configuration for Duka

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaConfig {
    pub parser: DukaParserConfig,
    pub analyzer: DukaAnalyzerConfig,
    pub adapter: DukaAdapterConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaIRConfig {
    pub var_default_local: bool,
}
impl Default for DukaIRConfig {
    fn default() -> Self {
        Self {
            var_default_local: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaParserConfig {
    pub use_stmt_expr: bool,
    pub use_bang_expr: bool,
    pub use_bang_stmt: bool,
    pub type_annotations: bool,
    /// Strict mode, values cannot be assigned with nil unless it has `xxx | nil` or `xxx?` annotation
    pub default_nonnilable: bool,
    /// Whether a bare `function f()...` defaults to a local binding.
    /// Mirrors `DukaIRConfig::var_default_local` so the parser can decide the
    /// AST `global` flag before IR generation (e.g. the REPL sets it to `false`
    /// so bare functions become global and persist across lines).
    pub var_default_local: bool,
}
impl Default for DukaParserConfig {
    fn default() -> Self {
        Self {
            use_bang_expr: true,
            use_bang_stmt: true,
            use_stmt_expr: true,
            type_annotations: true,
            var_default_local: true,
            default_nonnilable: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAnalyzerConfig {
    pub var_default_local: bool,
    pub type_annotations: bool,
    pub default_nonnilable: bool,
}

impl Default for DukaAnalyzerConfig {
    fn default() -> Self {
        Self {
            var_default_local: true,
            type_annotations: true,
            default_nonnilable: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DukaAdapterConfig {
    pub do_inline_adapt: bool,
}

impl Default for DukaAdapterConfig {
    fn default() -> Self {
        Self {
            do_inline_adapt: true,
        }
    }
}
