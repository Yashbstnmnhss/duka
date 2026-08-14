use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitStr, Path, Token};

use crate::crate_path::resolve_root_str;

mod kw {
    syn::custom_keyword!(plain);
    syn::custom_keyword!(meta);
    syn::custom_keyword!(co);
    syn::custom_keyword!(userdata);
    syn::custom_keyword!(doc);
    syn::custom_keyword!(example);
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

struct PlainMeta<T: Parse> {
    plain: Vec<T>,
    meta: Vec<T>,
}
impl<T: Parse> Parse for PlainMeta<T> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut plain = vec![];
        let mut meta = vec![];
        let inner;
        syn::braced!(inner in input);

        while !inner.is_empty() {
            if inner.peek(kw::plain) {
                inner.parse::<kw::plain>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<T, Token![,]> = inner.parse_terminated(T::parse, Token![,])?;
                plain.extend(list);
            } else if inner.peek(kw::meta) {
                inner.parse::<kw::meta>()?;
                inner.parse::<Token![:]>()?;
                let list: Punctuated<T, Token![,]> = inner.parse_terminated(T::parse, Token![,])?;
                meta.extend(list);
            } else {
                return Err(inner.error("expected 'plain' or 'meta'"));
            }
        }
        Ok(Self { plain, meta })
    }
}
pub struct BuiltinDef {
    name: Ident,
    doc: String,
    example: Option<String>,
    fns: PlainMeta<FnEntry>,
    consts: PlainMeta<Ident>,
    mods: Option<PlainMeta<Path>>,
    uds: Option<PlainMeta<Ident>>,
}

impl Parse for BuiltinDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![mod]>()?;
        let name = input.parse::<Ident>()?;

        let doc = if input.parse::<kw::doc>().is_ok() {
            input.parse::<LitStr>()?.token().to_string()
        } else {
            "".to_owned()
        };
        let example = if input.parse::<kw::example>().is_ok() {
            Some(input.parse::<LitStr>()?.token().to_string())
        } else {
            None
        };

        input.parse::<Token![fn]>()?;
        let fns = PlainMeta::<FnEntry>::parse(input)?;

        input.parse::<Token![const]>()?;
        let consts = PlainMeta::<Ident>::parse(input)?;

        let mods = if input.parse::<Token![mod]>().is_ok() {
            Some(PlainMeta::<Path>::parse(input)?)
        } else {
            None
        };
        let uds = if input.parse::<kw::userdata>().is_ok() {
            Some(PlainMeta::<Ident>::parse(input)?)
        } else {
            None
        };

        Ok(BuiltinDef {
            name,
            doc,
            example,
            fns,
            consts,
            mods,
            uds,
        })
    }
}

impl BuiltinDef {
    pub fn generate(self) -> TokenStream {
        fn reg_id(ident: &Ident) -> TokenStream {
            let name = ident.to_string();
            quote! { b = b.register(#name, #ident); }
        }
        fn reg_id_meta(ident: &Ident) -> TokenStream {
            let meta_ident = meta_ident(ident);
            quote! {
                let meta = #meta_ident;
                b = b.register(meta.name, #ident);
            }
        }
        fn map(
            o: Option<PlainMeta<Ident>>,
        ) -> (Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>) {
            if let Some(o) = o {
                (
                    o.plain.iter().map(reg_id).collect(),
                    o.meta.iter().map(reg_id_meta).collect(),
                    o.meta.iter().map(meta_ident).collect(),
                )
            } else {
                (vec![], vec![], vec![])
            }
        }
        let root_ts: proc_macro2::TokenStream = resolve_root_str().parse().unwrap();

        fn co(e: &FnEntry, root_ts: &TokenStream) -> TokenStream {
            let ident = &e.ident;
            if e.co {
                quote! { #root_ts::builtin::BuiltinFn::Co(#ident) }
            } else {
                quote! { #root_ts::builtin::BuiltinFn::Plain(#ident) }
            }
        }

        let fn_plain_registers = self.fns.plain.iter().map(|entry| {
            let ident = &entry.ident;
            let name = strip_impl_prefix(ident);
            let constructor = co(entry, &root_ts);
            quote! { b = b.register(#name, #constructor); }
        });

        let fn_meta_registers = self.fns.meta.iter().map(|entry| {
            let ident = &entry.ident;
            let meta_ident = meta_ident(ident);
            let constructor = co(entry, &root_ts);
            quote! {
                let meta = #meta_ident;
                b = b.register(meta.name, #constructor);
            }
        });

        let const_plain_registers = self.consts.plain.iter().map(reg_id);
        let const_meta_registers = self.consts.meta.iter().map(reg_id_meta);

        let (mod_plain_registers, mod_meta_registers, mod_meta_list) = if let Some(o) = self.mods {
            (
                o.plain
                    .iter()
                    .map(|i| {
                        let name = i.get_ident().expect("Use meta instead").to_string();
                        quote! {
                            let mr = #i::mods_registry(heap);
                            b = b.register(#name, #root_ts::builtin::make_module_table
                                (
                                    #i::registry(),
                                    #i::consts_registry(),
                                    mr,
                                    #name,
                                    heap
                                )
                            );
                        }
                    })
                    .collect(),
                o.meta
                    .iter()
                    .map(|i| {
                        quote! {
                            let name = #i::MODULE_META.name;
                            let mr = #i::mods_registry(heap);
                            b = b.register(name, #root_ts::builtin::make_module_table
                                (
                                    #i::registry(),
                                    #i::consts_registry(),
                                    mr,
                                    name,
                                    heap
                                )
                            );
                        }
                    })
                    .collect(),
                o.meta
                    .iter()
                    .map(|i| {
                        quote! { #i::MODULE_META }
                    })
                    .collect(),
            )
        } else {
            (vec![], vec![], vec![])
        };
        let (_, _, ud_meta_list) = map(self.uds);

        let fn_meta_list = self.fns.meta.iter().map(|entry| meta_ident(&entry.ident));
        let const_meta_list = self.consts.meta.iter().map(meta_ident);

        let all_meta_list = fn_meta_list
            .chain(const_meta_list)
            .chain(mod_meta_list.into_iter())
            .chain(ud_meta_list.into_iter());

        let name = self.name.to_string();
        let doc = self.doc;
        let example = self
            .example
            .map(|i| quote! {Some(#i)})
            .unwrap_or(quote! {None});

        quote! {
            pub fn registry() -> #root_ts::duka_shared::builtin::Builtins<BuiltinFn> {
                let mut b = #root_ts::duka_shared::builtin::Builtins::new();
                #(#fn_plain_registers)*
                #(#fn_meta_registers)*
                b
            }

            pub fn consts_registry() -> #root_ts::duka_shared::builtin::Builtins<RuntimeValue> {
                let mut b = #root_ts::duka_shared::builtin::Builtins::new();
                #(#const_plain_registers)*
                #(#const_meta_registers)*
                b
            }

            pub fn mods_registry(heap: &mut #root_ts::duka_gc::Heap) -> #root_ts::duka_shared::builtin::Builtins<#root_ts::value::RuntimeDukaTable> {
                let mut b = #root_ts::duka_shared::builtin::Builtins::new();
                #(#mod_plain_registers)*
                #(#mod_meta_registers)*
                b
            }

            pub(super) fn get_registry_table(heap: &mut #root_ts::duka_gc::Heap) -> #root_ts::value::RuntimeDukaTable {
                #root_ts::builtin::make_module_table(
                    registry(),
                    consts_registry(),
                    mods_registry(heap),
                    #name,
                    heap
                )
            }
            pub(super) const MODULE_NAME: &str = #name;
            pub(super) const MODULE_META: #root_ts::duka_shared::docs::MetaInfo = #root_ts::duka_shared::docs::MetaInfo {
                name: #name,
                doc: #doc,
                example: #example,
                info: #root_ts::duka_shared::docs::MetaItemInfo::Module {
                    inner: &[
                        #(#all_meta_list),*
                    ]
                }
            };
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

fn meta_ident(ident: &Ident) -> TokenStream {
    let name = ident.to_string().to_uppercase();
    Ident::new(&format!("__DUKA_{}_META", name), ident.span()).to_token_stream()
}
