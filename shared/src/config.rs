//! Configuration for Duka

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DukaConfig {
    pub lexer: DukaLexerConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DukaLexerConfig {
    pub keep_comment: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
            default_nonnilable: true,
        }
    }
}

/// What the strict bundle turns on: one flag per check `tsc` gates behind
/// `--strict`, so a project can keep the checks it can afford and drop the one
/// it cannot. `Default` is every check on -- the language reads as strict
/// unless a project says otherwise.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StrictConfig {
    /// `strictNullChecks`: a value is declared as what it is. A literal `nil`
    /// declares `nil` rather than `any`, and an inferred local is what was put
    /// in it, not that plus a nil it might hold later.
    pub nonnilable: bool,
    /// `noImplicitAny`: a name with nothing written and nothing put in it, and
    /// a parameter with no annotation, read as `unknown` and say so with an
    /// error -- not as `any`, silently.
    pub implicit_any: bool,
    /// `strictFunctionTypes`: one signature fits where another is wanted only
    /// if its parameters take the wanted one's parameters -- the direction a
    /// call actually runs in.
    pub fn_types: bool,
}

impl Default for StrictConfig {
    fn default() -> Self {
        Self {
            nonnilable: true,
            implicit_any: true,
            fn_types: true,
        }
    }
}

impl StrictConfig {
    /// Every check off, the pre-strict behavior in full.
    pub const fn off() -> Self {
        Self {
            nonnilable: false,
            implicit_any: false,
            fn_types: false,
        }
    }

    /// One switch for both: a project that says strict takes every check, and
    /// one that declines it takes the relaxed behavior on all of them.
    pub const fn from_flag(on: bool) -> Self {
        if on {
            Self { nonnilable: true, implicit_any: true, fn_types: true }
        } else {
            Self::off()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DukaAnalyzerConfig {
    pub var_default_local: bool,
    pub type_annotations: bool,
    /// The strict bundle. Old configurations that name no `strict` read as
    /// strict -- that is what upgrading to this language version means.
    #[serde(default)]
    pub strict: StrictConfig,
    /// The language edition the project is read with. `None` means "latest".
    pub edition: Option<String>,
    /// The features enabled for this project. `None` means every feature the
    /// edition offers.
    pub features: Option<Vec<String>>,
}

impl Default for DukaAnalyzerConfig {
    fn default() -> Self {
        Self {
            var_default_local: true,
            type_annotations: true,
            strict: StrictConfig::default(),
            edition: None,
            features: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
