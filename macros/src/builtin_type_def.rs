use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Ident, ItemConst, ItemFn, LitStr, Token, braced, parenthesized, parse::Parse,
    punctuated::Punctuated,
};

use crate::attr::{MetaInfoFlags, or_compile_error, root_tokens};

mod kw {
    use syn::custom_keyword;
    custom_keyword!(flags);
    custom_keyword!(doc);
    custom_keyword!(example);
}

pub struct BuiltinTypeDef {
    name: Ident,
    doc: String,
    example: Option<String>,
    flags: MetaInfoFlags,

    fns: Punctuated<ItemFn, Token![,]>,
    tys: Punctuated<ItemConst, Token![,]>,
}

impl Parse for BuiltinTypeDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<Token![type]>()?;
        input.parse::<Token![mod]>()?;
        let name = input.parse::<Ident>()?;

        let doc = if input.parse::<kw::doc>().is_ok() {
            input.parse::<LitStr>()?.value()
        } else {
            "".to_owned()
        };
        let example = if input.parse::<kw::example>().is_ok() {
            Some(input.parse::<LitStr>()?.value())
        } else {
            None
        };

        let flags = if input.parse::<kw::flags>().is_ok() {
            let inner;
            parenthesized!(inner in input);
            inner.parse::<MetaInfoFlags>()?
        } else {
            MetaInfoFlags::default()
        };

        let mut fns = None;
        let mut tys = None;

        while !input.is_empty() {
            if input.peek(kw::doc) || input.peek(kw::example) || input.peek(kw::flags) {
                return Err(input.error("`doc`, `example` and `flags` must come before sections"));
            } else if input.peek(Token![fn]) {
                if fns.is_some() {
                    return Err(input.error("duplicate `fn` section"));
                }

                input.parse::<Token![fn]>()?;

                let inner;
                braced!(inner in input);

                fns = Some(input.parse_terminated(ItemFn::parse, Token![,])?);
            } else if input.peek(Token![const]) {
                if tys.is_some() {
                    return Err(input.error("duplicate `const` section"));
                }

                input.parse::<Token![const]>()?;

                let inner;
                braced!(inner in input);

                tys = Some(input.parse_terminated(ItemConst::parse, Token![,])?);
            } else {
                return Err(input.error("expected a type section: `fn {…}`, `const {…}`"));
            }
        }

        Ok(Self {
            name,
            doc,
            example,
            flags,
            fns: fns.unwrap_or_default(),
            tys: tys.unwrap_or_default(),
        })
    }
}

impl BuiltinTypeDef {
    pub fn generate(self) -> TokenStream {
        let Self {
            name,
            doc,
            example,
            flags,
            fns,
            tys,
        } = self;

        let root_ts = or_compile_error!(root_tokens());

        let example = example.map(|i| quote! {Some(#i)}).unwrap_or(quote! {None});
        let flags = flags.into_tokens();

        let all_meta_list: Vec<TokenStream> = vec![];

        quote! {
            pub const TYPE_MODULE_NAME: &str = #name;
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const TYPE_MODULE_META: #root_ts::duka_shared::docs::MetaInfo = #root_ts::duka_shared::docs::MetaInfo {
                name: #name,
                doc: #doc,
                example: #example,
                info: #root_ts::duka_shared::docs::MetaItemInfo::Module {
                    inner: &[
                        #(#all_meta_list),*
                    ]
                },
                flags: #flags
            };
        }
    }
}
