use quote::quote;
use syn::{Data, DeriveInput, Ident, LitStr, Token, parse::Parse};

mod cs {
    use syn::custom_keyword;

    custom_keyword!(為);
    custom_keyword!(本紀);
    custom_keyword!(世家);
    custom_keyword!(列傳);
    custom_keyword!(也);
    custom_keyword!(者);
}

pub struct History {
    major: u8,
    minor: u8,
    patch: u8,
}
impl Parse for History {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut major = 0;
        let mut minor = 0;
        let mut patch = 0;

        input.parse::<Token![<<]>()?;
        input.parse::<Ident>()?;
        input.parse::<Token![>>]>()?;
        input.parse::<cs::者>()?;

        while !input.is_empty() {
            input.parse::<cs::為>()?;

            if input.parse::<cs::本紀>().is_ok() {
                minor = 0;
                patch = 0;
                major += 1;
            } else if input.parse::<cs::世家>().is_ok() {
                minor += 1;
                patch = 0;
            } else {
                input.parse::<cs::列傳>()?;
                patch += 1;
            }
            input.parse::<LitStr>()?;
            input.parse::<cs::也>()?;
        }

        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}
impl History {
    pub fn generate(&self) -> proc_macro2::TokenStream {
        let semver_path: syn::Path = syn::parse_str("SemVer").unwrap();

        let Self {
            major,
            minor,
            patch,
        } = self;
        quote! {
            #semver_path::new(#major, #minor, #patch)
        }
    }
}
