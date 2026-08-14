use proc_macro_crate::{FoundCrate, crate_name};
// use proc_macro2::TokenStream;

// pub fn resolve_root() -> TokenStream {
//     resolve_root_str().parse().unwrap()
// }

pub fn resolve_root_str() -> String {
    match crate_name("duka-lib") {
        Ok(FoundCrate::Name(name)) => format!("::{}", name),
        Ok(FoundCrate::Itself) | Err(_) => "crate".to_string(),
    }
}
