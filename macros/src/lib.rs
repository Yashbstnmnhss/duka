// use proc_macro2::TokenStream;
// use quote::quote;
use syn::{DeriveInput, parse_macro_input};

mod errors;
mod info;
mod instructions;

use errors::generate_errors;
use instructions::Instructions;

use crate::info::generate_info;

extern crate proc_macro;

// #[proc_macro_attribute]
// pub fn self_terminating(
//     _attr: proc_macro::TokenStream,
//     input: proc_macro::TokenStream,
// ) -> proc_macro::TokenStream {
//     let input: TokenStream = input.into();
//     quote! {
//         #[doc = "# Self-terminating"]
//         #input
//     }
//     .into()
// }

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

/// Name tag for enum
/// ## Results are all lowercase without fields
/// Auto derive Display trait & name() function
/// and tags
#[proc_macro_derive(Info, attributes(name, tag, val))]
pub fn derive_info(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    generate_info(input).into()
}
