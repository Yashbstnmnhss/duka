use syn::{DeriveInput, parse_macro_input};

mod errors;
mod instructions;

use errors::generate_errors;
use instructions::Instructions;

extern crate proc_macro;

#[proc_macro]
pub fn instructions(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let def = parse_macro_input!(input as Instructions);
    def.generate().into()
}

/// A very simple macro like thiserror crate
/// ## You must ensure that the count of message's interpolations is equal to the count of variants' parameters
/// *otherwise errors occur*
#[proc_macro_derive(ThatError, attributes(error))]
pub fn derive_that_error(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    generate_errors(input).into()
}
