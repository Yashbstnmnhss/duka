//! All macros here are highly attached with duka crates.
//! Do not use them independently

use syn::{DeriveInput, parse_macro_input};

mod attr;
mod binop;
mod builtin;
mod builtin_def;
mod crate_path;
mod errors;
mod history;
mod info;
mod instructions;
mod user_data;
mod visitors;

use binop::Ops;
use errors::generate_errors;
use history::History;
use info::generate_info;
use instructions::Instructions;
use visitors::generate_visitors;

use crate::{builtin_def::BuiltinDef, user_data::UserDataDef};

extern crate proc_macro;

/// # Auto visitor
/// Attached with `Visit` and `Visitor`.
/// You can customize trait names with `visit_trait`, `visitor_trait` attributes, defaults are `Visit` `Visitor`
/// # Example:
/// ```
/// #[derive(VisitorMut)]
/// #[visit_trait = "MyVisit"]
/// #[visitor_trait = "MyVisitor"]
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
/// You can customize trait names with `visit_mut_trait`, `visitor_mut_trait` attributes, defaults are `VisitMut` `VisitorMut`
/// # NOITCE:
/// Different to `#[block]` in `Visit`
/// the attribute `#[block_mut]` will **stop any deeply visiting**, this is because all visiting here is mutable, it allows you to insert new statement into a block (not only just replacing one to another statement), for further visiting, you can handle them manually by `.visit_mut` and so on
/// # Example:
/// ```
/// #[derive(VisitorMut)]
/// #[visit_mut_trait = "MyVisitMut"]
/// #[visitor_mut_trait = "MyVisitorMut"]
/// ```
#[proc_macro_derive(
    VisitorMut,
    attributes(nonvisiting, block_mut, ast, visit_mut_trait, visitor_mut_trait)
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

/// # Auto Instruction Generator
///
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

/// # duka_builtin
/// Turns a plain rust function into a Duka builtin:
/// reads & type-checks arguments, writes returns, and emits a `BuiltinMeta`
#[proc_macro_attribute]
pub fn duka_builtin(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    builtin::generate(item.into(), attr.into()).into()
}

/// # duka_builtin
/// Use this to export functions with `#[duka_builtin]`
#[proc_macro]
pub fn duka_builtin_def(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let def = parse_macro_input!(input as BuiltinDef);
    def.generate().into()
}

/// # duka_user_data
/// Declare a struct as `RuntimeValue::UserData`
#[proc_macro]
pub fn duka_user_data(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let def = parse_macro_input!(input as UserDataDef);
    def.generate().into()
}
