use syn::{DeriveInput, parse_macro_input};

mod binop;
mod errors;
mod history;
mod info;
mod instructions;
mod visitors;

use binop::Ops;
use errors::generate_errors;
use history::History;
use info::generate_info;
use instructions::Instructions;
use visitors::generate_visitors;

extern crate proc_macro;

/// # Auto visitor
/// Attached with `Visit` and `Visitor`.
/// You can customize trait names with `visit_trait`, `visitor_trait` attributes
/// # Example:
/// ```
/// #[proc_macro_derive(Visitor, attributes(visit_trait = "MyVisit", visitor_trait = "MyVisitor"))]
/// ```
#[proc_macro_derive(
    Visitor,
    attributes(nonvisiting, block, ast, visit_trait, visitor_trait)
)]
pub fn derive_auto_visitor(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    generate_visitors(input, false).into()
}
/// # Auto mutable visitor
/// Attached with `VisitMut` and `VisitorMut`.
/// You can customize trait names with `visit_mut_trait`, `visitor_mut_trait` attributes
/// # Example:
/// ```
/// #[proc_macro_derive(VisitorMut, attributes(visit_mut_trait = "MyVisitMut", visitor_trait = "MyVisitorMut"))]
/// ```
#[proc_macro_derive(
    VisitorMut,
    attributes(nonvisiting, block, ast, visit_mut_trait, visitor_mut_trait)
)]
pub fn derive_auto_visitor_mut(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    generate_visitors(input, true).into()
}

/// ## 鐵鏽者, 編程語言也. 宏為其之菁萃.
///
/// ## 以此宏得一物 名SemVer 恆常者也
#[proc_macro]
pub fn 史書云(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let 史書 = parse_macro_input!(input as History);
    史書.generate().into()
}

/// auto instruction generator
#[proc_macro]
pub fn instructions(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let def = parse_macro_input!(input as Instructions);
    def.generate().into()
}

/// auto binary operator generator
#[proc_macro]
pub fn binops(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let def = parse_macro_input!(input as Ops);
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
#[proc_macro_derive(Info, attributes(name, tag, shy, idcard))]
pub fn derive_info(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    generate_info(input).into()
}
