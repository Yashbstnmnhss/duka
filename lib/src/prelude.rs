//! Everything that has to be declared before the checker can resolve a name
//! the file itself does not declare.
//!
//! It comes in two halves and both of them are data rather than code: the type
//! prelude the language ships, and the builtins the runtime this build links
//! against provides. Neither is a list written here, so adding either is a
//! change to what the project provides rather than to the compiler.

use duka_frontend::analyzer::ScopeAnalysis;
use duka_frontend::analyzer::builtin::TYPE_BUILTINS_META;
use duka_frontend::analyzer::prelude::inject_type_prelude;
use duka_frontend::analyzer::prelude::{inject_builtins, inject_type_builtins};
use duka_shared::errors::DukaSpannedError;

/// Declares the prelude first, then what the runtime provides: a name the file
/// declares itself shadows both, and a prelude name outranks a builtin.
pub fn inject(analysis: &mut ScopeAnalysis) -> Vec<DukaSpannedError> {
    let errors = inject_type_prelude(analysis);
    inject_builtins(
        analysis,
        &duka_backend::builtin::all_builtin_registrations(),
    );
    inject_type_builtins(analysis, &TYPE_BUILTINS_META);
    errors
}
