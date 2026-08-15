use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::{
    Attribute, Error, FnArg, Ident, ItemFn, ItemStruct, LitStr, Token, parse::Parse,
    punctuated::Punctuated, spanned::Spanned,
};

use crate::attr::*;
use crate::crate_path::resolve_root_str;

pub struct UserDataDef {
    payload: ItemStruct,
    constructor: Option<ItemFn>,
    destructor: Option<ItemFn>,
    methods: Punctuated<ItemFn, Token![,]>,
}

fn err(span: Span, message: String) -> Error {
    Error::new(span, message)
}

mod kw {
    use syn::custom_keyword;

    custom_keyword!(constructor);
    custom_keyword!(destructor);
}

impl Parse for UserDataDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let payload = input.parse::<ItemStruct>()?;

        let constructor = if input.parse::<kw::constructor>().is_ok() {
            let f = input.parse::<ItemFn>()?;
            if f.sig
                .inputs
                .iter()
                .any(|i| matches!(i, FnArg::Receiver(..)))
            {
                return Err(err(f.span(), "Constructor cannot access `self`".to_owned()));
            }
            Some(f)
        } else {
            None
        };
        let destructor = if input.parse::<kw::destructor>().is_ok() {
            let f = input.parse::<ItemFn>()?;
            Some(f)
        } else {
            None
        };

        let methods = input.parse_terminated(ItemFn::parse, Token![,])?;
        Ok(Self {
            payload,
            constructor,
            destructor,
            methods,
        })
    }
}

struct StructArgs {
    name: String,
    doc: String,
    example: Option<String>,
}

fn parse_struct_attr(attrs: &[Attribute]) -> syn::Result<Option<StructArgs>> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident("duka_builtin")) else {
        return Ok(None);
    };
    let tokens = attr.meta.require_list()?.tokens.clone();
    let mut out = StructArgs {
        name: String::new(),
        doc: String::new(),
        example: None,
    };
    for seg in split_commas(tokens) {
        let mut toks: Vec<TokenTree> = seg.into_iter().collect();
        let Some(TokenTree::Ident(key)) = toks.first() else {
            return Err(Error::new(
                Span::call_site(),
                "invalid duka_builtin attribute",
            ));
        };
        let key = key.to_string();
        toks.remove(0);
        if toks.first().map(is_eq).unwrap_or(false) {
            toks.remove(0);
        }
        let rest: TokenStream = toks.into_iter().collect();
        match key.as_str() {
            "name" => out.name = lit_str(&rest)?,
            "doc" => out.doc = lit_str(&rest)?,
            "example" => out.example = Some(lit_str(&rest)?),
            _ => {
                return Err(Error::new_spanned(
                    &key,
                    format!("unknown duka_builtin attribute: {}", key),
                ));
            }
        }
    }
    Ok(Some(out))
}

fn strip_duka_attr(mut f: ItemFn) -> ItemFn {
    f.attrs.retain(|a| !a.path().is_ident("duka_builtin"));
    f
}

impl UserDataDef {
    pub fn generate(self) -> TokenStream {
        let UserDataDef {
            payload,
            constructor,
            destructor,
            methods,
        } = self;
        let name = payload.ident.clone();
        let name_str = name.to_string();
        let type_name_upper = name_str.to_uppercase();

        let krate: TokenStream = resolve_root_str().parse().unwrap();
        let meta_ty = parse_type(&format!("{}::duka_shared::docs::MetaInfo", krate));
        let heap_type = parse_type(&format!("{}::duka_gc::Heap", krate));
        let table_type = parse_type(&format!("{}::value::RuntimeDukaTable", krate));
        let gc_cell_type = parse_type(&format!("{}::duka_gc::GcCell", krate));
        let user_data_type = parse_type(&format!("{}::value::UserData", krate));
        let user_data_payload_trait = parse_type(&format!("{}::value::UserDataPayload", krate));

        let struct_args = match parse_struct_attr(&payload.attrs) {
            Ok(v) => v.unwrap_or(StructArgs {
                name: String::new(),
                doc: String::new(),
                example: None,
            }),
            Err(e) => return e.into_compile_error(),
        };
        let type_display_name = if struct_args.name.is_empty() {
            name_str.clone()
        } else {
            struct_args.name.clone()
        };

        let mut cleaned_methods: Vec<ItemFn> = Vec::new();
        let mut metatable_inserts: Vec<TokenStream> = Vec::new();
        let mut method_meta_fns: Vec<TokenStream> = Vec::new();
        let mut method_meta_idents: Vec<Ident> = Vec::new();

        let methods: Vec<_> = if let Some(mut dm) = destructor {
            dm.sig.ident = str2ident("__gc");
            methods.into_iter().chain(std::iter::once(dm)).collect()
        } else {
            methods.into_iter().collect()
        };

        for method in methods {
            let attr = match method
                .attrs
                .iter()
                .find(|a| a.path().is_ident("duka_builtin"))
            {
                Some(a) => a,
                None => {
                    let e = Error::new_spanned(
                        &method.sig,
                        "userdata methods must carry a #[duka_builtin(...)] attribute",
                    );
                    return e.into_compile_error();
                }
            };
            let attr_tokens = match attr.meta.require_list() {
                Ok(m) => m.tokens.clone(),
                Err(e) => return e.into_compile_error(),
            };
            let args = match parse_builtin_args(attr_tokens) {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };

            let method_ident = method.sig.ident.clone();
            let reads = match gen_arg_reads(&method.sig, &args, &krate, 1, Some(&name)) {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };
            let ArgReads {
                read_stmts,
                call_args,
                meta_params,
                has_co,
            } = reads;
            if has_co {
                let e = Error::new_spanned(&method.sig, "co methods are not supported yet");
                return e.into_compile_error();
            }

            let return_kind = match classify_return(&method.sig.output) {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };
            let epilog = match gen_return(&return_kind, &krate) {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };
            let meta_returns: Vec<TokenStream> = match args
                .returns
                .iter()
                .map(|t| ty_to_kind(&t.ty, Span::call_site()).map(|i| i.meta.to_doc_type()))
                .collect()
            {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };

            let user_name = method_ident.to_string();
            let duka_name = args.name.clone().unwrap_or_else(|| user_name.clone());
            let meta_ident = str2ident(&format!(
                "__DUKA_{}_{}_META",
                type_name_upper,
                user_name.to_uppercase()
            ));
            let meta_fn = gen_meta(
                &user_name,
                &meta_ident,
                &args,
                &meta_params,
                &meta_returns,
                &krate,
            );

            let debug_name = format!("{}::{}", name_str, duka_name);
            let name_lit = LitStr::new(&duka_name, Span::call_site());
            let closure = quote! {
                #krate::value::RustClosure::returns(
                    move |sv, h, _api| -> Result<#krate::duka_shared::types::ValueCount, #krate::errors::DukaRuntimeError> {
                        #(#read_stmts)*
                        let __ret = #name::#method_ident(#(#call_args),*)?;
                        #epilog
                    },
                    Some(#debug_name.into())
                )
            };
            metatable_inserts.push(quote! {
                let __duka_closure = #krate::value::RuntimeValue::from_rust_closure(heap, #closure);
                tab.set_by_key(heap, #name_lit.to_string(), __duka_closure);
            });
            method_meta_fns.push(meta_fn);
            method_meta_idents.push(meta_ident);
            cleaned_methods.push(strip_duka_attr(method));
        }

        let payload = {
            let mut p = payload;
            p.attrs.retain(|a| !a.path().is_ident("duka_builtin"));
            p
        };
        let constructor = constructor.map(strip_duka_attr);

        let methods_count = metatable_inserts.len();
        let type_name_lit = LitStr::new(&type_display_name, Span::call_site());
        let type_doc_lit = LitStr::new(&struct_args.doc, Span::call_site());
        let type_meta_ident = str2ident(&format!("__DUKA_{}_META", type_name_upper));
        let type_example = match &struct_args.example {
            Some(e) => {
                let lit = LitStr::new(e, Span::call_site());
                quote! { Some(#lit) }
            }
            None => quote! { None },
        };

        quote! {
            #payload
            impl #user_data_payload_trait for #name {
                fn type_name(&self) -> &'static str {
                    #type_name_lit
                }
            }
            impl #name {
                #(#cleaned_methods)*
                #constructor
                pub fn into_user_data(self, heap: &mut #heap_type) -> #user_data_type {
                    let mut tab = #table_type::new(#methods_count);
                    #(#metatable_inserts)*
                    let __duka_mt = heap.alloc(#gc_cell_type::new(tab));

                    __duka_mt.borrow_mut().set_by_key(heap, "__index".to_string(), #krate::value::RuntimeValue::Table(__duka_mt));
                    #user_data_type {
                        payload: Box::new(self),
                        metatable: Some(__duka_mt)
                    }
                }
                pub fn into_value(self, heap: &mut #heap_type) -> #krate::value::RuntimeValue {
                    let __duka_ud = self.into_user_data(heap);
                    #krate::value::RuntimeValue::UserData(heap.alloc(#gc_cell_type::new(__duka_ud)))
                }
            }
            #(#method_meta_fns)*
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const #type_meta_ident: #meta_ty = #krate::duka_shared::docs::MetaInfo {
                name: #type_name_lit,
                doc: #type_doc_lit,
                info: #krate::duka_shared::docs::MetaItemInfo::UserData {
                    ty_name: #type_name_lit,
                    methods: &[#(#method_meta_idents),*],
                },
                example: #type_example,
            };
        }
    }
}
