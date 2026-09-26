use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, LitStr, Path, Token, parenthesized};

use crate::attr::{MetaInfoFlags, or_compile_error, parse_type_or, root_tokens, strip_impl_prefix};
use crate::crate_path::resolve_root_str;

mod kw {
    syn::custom_keyword!(plain);
    syn::custom_keyword!(meta);
    syn::custom_keyword!(co);
    syn::custom_keyword!(userdata);
    syn::custom_keyword!(doc);
    syn::custom_keyword!(example);
    syn::custom_keyword!(init);
    syn::custom_keyword!(flags);
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

struct InitEntry {
    name: Ident,
    expr: Expr,
    meta: Ident,
    doc: Option<String>,
    example: Option<String>,
}

impl Parse for InitEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let expr: Expr = input.parse()?;
        input.parse::<kw::meta>()?;
        let meta: Ident = input.parse()?;

        let doc = if input.parse::<kw::doc>().is_ok() {
            let content;
            parenthesized!(content in input);
            Some(content.parse::<LitStr>()?.value())
        } else {
            None
        };

        let example = if input.parse::<kw::example>().is_ok() {
            let content;
            parenthesized!(content in input);
            Some(content.parse::<LitStr>()?.value())
        } else {
            None
        };

        Ok(InitEntry {
            name,
            expr,
            meta,
            doc,
            example,
        })
    }
}

struct PlainMeta<T: Parse> {
    plain: Vec<T>,
    meta: Vec<T>,
}
impl<T: Parse> Default for PlainMeta<T> {
    fn default() -> Self {
        Self {
            plain: vec![],
            meta: vec![],
        }
    }
}

fn at_section_boundary(inner: ParseStream) -> bool {
    inner.is_empty()
        || (inner.peek(kw::plain) && inner.peek2(Token![:]))
        || (inner.peek(kw::meta) && inner.peek2(Token![:]))
}

fn parse_entry_list<T: Parse>(inner: ParseStream) -> syn::Result<Vec<T>> {
    let mut out = vec![];
    loop {
        if at_section_boundary(inner) {
            break;
        }
        out.push(inner.parse::<T>()?);
        if inner.peek(Token![,]) {
            inner.parse::<Token![,]>()?;
        } else if at_section_boundary(inner) {
            break;
        } else {
            return Err(inner.error("expected `,` between entries"));
        }
    }
    Ok(out)
}

impl<T: Parse> Parse for PlainMeta<T> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut plain = vec![];
        let mut meta = vec![];
        let inner;
        syn::braced!(inner in input);

        while !inner.is_empty() {
            if inner.peek(kw::plain) && inner.peek2(Token![:]) {
                inner.parse::<kw::plain>()?;
                inner.parse::<Token![:]>()?;
                plain.extend(parse_entry_list::<T>(&inner)?);
            } else if inner.peek(kw::meta) && inner.peek2(Token![:]) {
                inner.parse::<kw::meta>()?;
                inner.parse::<Token![:]>()?;
                meta.extend(parse_entry_list::<T>(&inner)?);
            } else {
                return Err(inner.error("expected `plain:` or `meta:`"));
            }
        }
        Ok(Self { plain, meta })
    }
}
pub struct BuiltinDef {
    name: Ident,
    doc: String,
    example: Option<String>,
    flags: MetaInfoFlags,
    fns: PlainMeta<FnEntry>,
    consts: PlainMeta<Ident>,
    init: Punctuated<InitEntry, Token![,]>,
    mods: Option<PlainMeta<Path>>,
    uds: Option<PlainMeta<Ident>>,
}

impl Parse for BuiltinDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
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

        let mut fns: Option<PlainMeta<FnEntry>> = None;
        let mut consts: Option<PlainMeta<Ident>> = None;
        let mut init: Option<Punctuated<InitEntry, Token![,]>> = None;
        let mut mods: Option<PlainMeta<Path>> = None;
        let mut uds: Option<PlainMeta<Ident>> = None;

        while !input.is_empty() {
            if input.peek(kw::doc) || input.peek(kw::example) || input.peek(kw::flags) {
                return Err(input.error("`doc`, `example` and `flags` must come before sections"));
            } else if input.peek(Token![fn]) {
                if fns.is_some() {
                    return Err(input.error("duplicate `fn` section"));
                }
                input.parse::<Token![fn]>()?;
                fns = Some(PlainMeta::<FnEntry>::parse(input)?);
            } else if input.peek(Token![const]) {
                if consts.is_some() {
                    return Err(input.error("duplicate `const` section"));
                }
                input.parse::<Token![const]>()?;
                consts = Some(PlainMeta::<Ident>::parse(input)?);
            } else if input.peek(kw::init) {
                if init.is_some() {
                    return Err(input.error("duplicate `init` section"));
                }
                input.parse::<kw::init>()?;
                let inner;
                syn::braced!(inner in input);
                init = Some(inner.parse_terminated(InitEntry::parse, Token![,])?);
            } else if input.peek(Token![mod]) {
                if mods.is_some() {
                    return Err(input.error("duplicate `mod` section"));
                }
                input.parse::<Token![mod]>()?;
                mods = Some(PlainMeta::<Path>::parse(input)?);
            } else if input.peek(kw::userdata) {
                if uds.is_some() {
                    return Err(input.error("duplicate `userdata` section"));
                }
                input.parse::<kw::userdata>()?;
                uds = Some(PlainMeta::<Ident>::parse(input)?);
            } else {
                return Err(input.error(
                    "expected a section: `fn {…}`, `const {…}`, `init {…}`, `mod {…}` or `userdata {…}`",
                ));
            }
        }

        Ok(BuiltinDef {
            name,
            doc,
            example,
            flags,
            fns: fns.unwrap_or_default(),
            consts: consts.unwrap_or_default(),
            init: init.unwrap_or_default(),
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
            let name_ident = meta_name_ident(ident);
            quote! { b = b.register(#name_ident, #ident); }
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
        let root_ts = or_compile_error!(root_tokens());

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
            let name_ident = meta_name_ident(ident);
            let constructor = co(entry, &root_ts);
            quote! { b = b.register(#name_ident, #constructor); }
        });

        let const_plain_registers = self.consts.plain.iter().map(reg_id);
        let const_meta_registers = self.consts.meta.iter().map(reg_id_meta);

        let (mod_plain_registers, mod_meta_registers, mod_meta_list) = if let Some(o) = self.mods {
            let plain: syn::Result<Vec<TokenStream>> = o
                .plain
                .iter()
                .map(|i| {
                    let ident = i.get_ident().ok_or_else(|| {
                        Error::new_spanned(
                            i,
                            "plain `mod` entries must be a single identifier; use `meta:` for paths",
                        )
                    })?;
                    let name = ident.to_string();
                    Ok(quote! {
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
                        })
                })
                .collect();
            (
                or_compile_error!(plain),
                o.meta
                    .iter()
                    .map(|i| {
                        quote! {
                            let name = #i::MODULE_NAME;
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

        let root = resolve_root_str();
        let tn = parse_type_or!(&format!("{}::duka_shared::docs::MetaInfo", root));
        let tno = parse_type_or!(&format!("{}::duka_shared::docs::MetaItemInfo", root));

        let all_meta_list = fn_meta_list
            .chain(const_meta_list)
            .chain(mod_meta_list)
            .chain(ud_meta_list)
            .chain(self.init.iter().map(
                |InitEntry {
                     name,
                     meta,
                     doc,
                     example,
                     ..
                 }| {
                    let name = name.to_string();
                    let doc = doc.clone().unwrap_or_default();
                    let example = example
                        .as_ref()
                        .map(|i| {
                            quote! {
                                Some(#i)
                            }
                        })
                        .unwrap_or(quote! {None});
                    quote! {
                        #tn {
                            name: #name,
                            doc: #doc,
                            example: #example,
                            flags: &[],
                            info: #tno::Static {
                                inner: &#meta
                            }
                        }
                    }
                },
            ));

        let name = self.name.to_string();
        let doc = self.doc;
        let example = self
            .example
            .map(|i| quote! {Some(#i)})
            .unwrap_or(quote! {None});
        let init_registers = self.init.iter().map(|entry| {
            let key = LitStr::new(&entry.name.to_string(), Span::call_site());
            let expr = &entry.expr;
            quote! {
                let __init_val = #expr;
                __table.set_by_key(heap, #key.to_string(), __init_val);
            }
        });

        let flags = self.flags.into_tokens();

        quote! {
            pub fn registry() -> #root_ts::duka_shared::builtin::Builtins<#root_ts::builtin::BuiltinFn> {
                let mut b = #root_ts::duka_shared::builtin::Builtins::new();
                #(#fn_plain_registers)*
                #(#fn_meta_registers)*
                b
            }

            pub fn consts_registry() -> #root_ts::duka_shared::builtin::Builtins<#root_ts::value::RuntimeValue> {
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

            pub(crate) fn get_registry_table(heap: &mut #root_ts::duka_gc::Heap) -> #root_ts::value::RuntimeDukaTable {
                let mut __table = #root_ts::builtin::make_module_table(
                    registry(),
                    consts_registry(),
                    mods_registry(heap),
                    #name,
                    heap
                );
                #(#init_registers)*
                __table
            }
            pub const MODULE_NAME: &str = #name;
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const MODULE_META: #root_ts::duka_shared::docs::MetaInfo = #root_ts::duka_shared::docs::MetaInfo {
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

fn meta_ident(ident: &Ident) -> TokenStream {
    let name = ident.to_string().to_uppercase();
    Ident::new(&format!("__DUKA_{}_META", name), ident.span()).to_token_stream()
}

fn meta_name_ident(ident: &Ident) -> TokenStream {
    let name = ident.to_string().to_uppercase();
    Ident::new(&format!("__DUKA_{}_NAME", name), ident.span()).to_token_stream()
}
