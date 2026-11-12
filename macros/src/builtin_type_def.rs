use proc_macro2::TokenStream;
use syn::parse::Parse;

pub struct BuiltinTypeDef {}

impl Parse for BuiltinTypeDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        todo!()
    }
}

impl BuiltinTypeDef {
    pub fn generate(self) -> TokenStream {
        todo!()
    }
}
