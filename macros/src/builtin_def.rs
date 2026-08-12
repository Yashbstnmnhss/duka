use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, Token};

mod kw {
    syn::custom_keyword!(plain);
    syn::custom_keyword!(meta);
    syn::custom_keyword!(co);
}

struct FnEntry {
    ident: Ident,
    co: bool,
}

impl Parse for FnEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        let co = if input.peek(kw::co) {
            input.parse::<kw::co>()?;
            true
        } else {
            false
        };
        Ok(FnEntry { ident, co })
    }
}

pub struct BuiltinDef {
    fn_plain: Vec<FnEntry>,
    fn_meta: Vec<FnEntry>,
    const_plain: Vec<Ident>,
    const_meta: Vec<Ident>,
}

impl Parse for BuiltinDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut fn_plain = Vec::new();
        let mut fn_meta = Vec::new();
        let mut const_plain = Vec::new();
        let mut const_meta = Vec::new();

        input.parse::<Token![fn]>()?;
        let inner;
        syn::braced!(inner in input);

        while !inner.is_empty() {
            if inner.peek(kw::plain) {
                inner.parse::<kw::plain>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<FnEntry, Token![,]> =
                    inner.parse_terminated(FnEntry::parse, Token![,])?;
                fn_plain.extend(list);
            } else if inner.peek(kw::meta) {
                inner.parse::<kw::meta>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<FnEntry, Token![,]> =
                    inner.parse_terminated(FnEntry::parse, Token![,])?;
                fn_meta.extend(list);
            } else {
                return Err(inner.error("expected 'plain' or 'meta'"));
            }
        }

        input.parse::<Token![const]>()?;
        let inner;
        syn::braced!(inner in input);

        while !inner.is_empty() {
            if inner.peek(kw::plain) {
                inner.parse::<kw::plain>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<Ident, Token![,]> =
                    inner.parse_terminated(Ident::parse, Token![,])?;
                const_plain.extend(list);
            } else if inner.peek(kw::meta) {
                inner.parse::<kw::meta>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<Ident, Token![,]> =
                    inner.parse_terminated(Ident::parse, Token![,])?;
                const_meta.extend(list);
            } else {
                return Err(inner.error("expected 'plain' or 'meta'"));
            }
        }

        Ok(BuiltinDef {
            fn_plain,
            fn_meta,
            const_plain,
            const_meta,
        })
    }
}

impl BuiltinDef {
    pub fn generate(self) -> proc_macro2::TokenStream {
        let fn_plain_registers = self.fn_plain.iter().map(|entry| {
            let ident = &entry.ident;
            let name = strip_impl_prefix(ident);
            let constructor = if entry.co {
                quote! { BuiltinFn::Co(#ident) }
            } else {
                quote! { BuiltinFn::Plain(#ident) }
            };
            quote! { b = b.register(#name, #constructor); }
        });

        let fn_meta_registers = self.fn_meta.iter().map(|entry| {
            let ident = &entry.ident;
            let meta_ident = meta_const_ident(ident);
            let constructor = if entry.co {
                quote! { BuiltinFn::Co(#ident) }
            } else {
                quote! { BuiltinFn::Plain(#ident) }
            };
            quote! {
                let meta = #meta_ident;
                b = b.register(meta.name, #constructor);
            }
        });

        let const_plain_registers = self.const_plain.iter().map(|ident| {
            let name = ident.to_string();
            quote! { b = b.register(#name, #ident); }
        });

        let const_meta_registers = self.const_meta.iter().map(|ident| {
            let meta_ident = meta_const_ident(ident);
            quote! {
                let meta = #meta_ident;
                b = b.register(meta.name, #ident);
            }
        });

        let fn_meta_list = self
            .fn_meta
            .iter()
            .map(|entry| meta_const_ident(&entry.ident));
        let const_meta_list = self.const_meta.iter().map(meta_const_ident);
        let all_meta_list = fn_meta_list.chain(const_meta_list);

        quote! {
            pub fn registry() -> ::duka_shared::builtin::Builtins<BuiltinFn> {
                let mut b = ::duka_shared::builtin::Builtins::new();
                #(#fn_plain_registers)*
                #(#fn_meta_registers)*
                b
            }

            pub fn consts_registry() -> ::duka_shared::builtin::Builtins<RuntimeValue> {
                let mut b = ::duka_shared::builtin::Builtins::new();
                #(#const_plain_registers)*
                #(#const_meta_registers)*
                b
            }

            pub(super) fn builtin_metas() -> Vec<::duka_shared::docs::MetaInfo> {
                vec![
                    #(#all_meta_list),*
                ]
            }
        }
    }
}

fn strip_impl_prefix(ident: &Ident) -> String {
    let s = ident.to_string();
    if s.starts_with("impl_") {
        s.trim_start_matches("impl_").to_string()
    } else {
        s
    }
}

fn meta_const_ident(ident: &Ident) -> Ident {
    let name = ident.to_string().to_uppercase();
    Ident::new(&format!("__DUKA_{}_META", name), ident.span())
}
