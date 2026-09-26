use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    Attribute, Error, FnArg, Ident, ItemFn, ItemStruct, LitStr, Token, braced, parse::Parse,
    spanned::Spanned,
};

use crate::attr::*;

#[derive(Debug)]
pub struct UserDataDef {
    payload: ItemStruct,
    constructor: Option<ItemFn>,
    destructor: Option<ItemFn>,
    methods: Vec<MethodItem>,
}

#[derive(Debug)]
struct MethodItem {
    f: ItemFn,
    in_block: bool,
}

const METAMETHODS: &[&str] = &[
    "__index",
    "__newindex",
    "__gc",
    "__mode",
    "__len",
    "__eq",
    "__add",
    "__sub",
    "__mul",
    "__mod",
    "__pow",
    "__div",
    "__idiv",
    "__band",
    "__bor",
    "__bxor",
    "__shl",
    "__shr",
    "__unm",
    "__bnot",
    "__lt",
    "__le",
    "__concat",
    "__call",
    "__close",
    "__tostring",
    "__bind",
    "__return",
    "__zero",
    "__while",
    "__forin",
    "__combine",
];

fn err(span: Span, message: String) -> Error {
    Error::new(span, message)
}

mod kw {
    use syn::custom_keyword;

    custom_keyword!(constructor);
    custom_keyword!(destructor);
    custom_keyword!(metamethod);
}

fn section_ahead(input: &syn::parse::ParseBuffer) -> bool {
    input.peek(kw::constructor) || input.peek(kw::destructor) || input.peek(kw::metamethod)
}

impl Parse for UserDataDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let payload = input.parse::<ItemStruct>()?;
        let mut constructor: Option<ItemFn> = None;
        let mut destructor: Option<ItemFn> = None;
        let mut methods: Vec<MethodItem> = vec![];

        while !input.is_empty() {
            let mut after_method = false;
            if input.peek(kw::constructor) {
                input.parse::<kw::constructor>()?;
                if constructor.is_some() {
                    return Err(err(input.span(), "duplicate `constructor`".to_owned()));
                }
                let f = input.parse::<ItemFn>()?;
                if f.sig
                    .inputs
                    .iter()
                    .any(|i| matches!(i, FnArg::Receiver(..)))
                {
                    return Err(err(f.span(), "Constructor cannot access `self`".to_owned()));
                }
                constructor = Some(f);
            } else if input.peek(kw::destructor) {
                input.parse::<kw::destructor>()?;
                if destructor.is_some() {
                    return Err(err(input.span(), "duplicate `destructor`".to_owned()));
                }
                destructor = Some(input.parse::<ItemFn>()?);
            } else if input.peek(kw::metamethod) {
                input.parse::<kw::metamethod>()?;
                let content;
                braced!(content in input);
                let block = content.parse_terminated(ItemFn::parse, Token![,])?;
                if block.is_empty() {
                    return Err(err(
                        content.span(),
                        "`metamethod` block is empty".to_owned(),
                    ));
                }
                for f in block {
                    methods.push(MethodItem { f, in_block: true });
                }
            } else {
                methods.push(MethodItem {
                    f: input.parse::<ItemFn>()?,
                    in_block: false,
                });
                after_method = true;
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if after_method && !input.is_empty() && !section_ahead(input) {
                return Err(err(input.span(), "expected `,` between methods".to_owned()));
            }
        }

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
    for_each_attr_key(tokens, AttrShape::Struct, |key, rest, _span| match key {
        "name" => {
            out.name = lit_str(&rest)?;
            Ok(())
        }
        "doc" => {
            out.doc = lit_str(&rest)?;
            Ok(())
        }
        "example" => {
            out.example = Some(lit_str(&rest)?);
            Ok(())
        }
        _ => Err(Error::new(
            Span::call_site(),
            format!("unhandled struct attribute `{key}`"),
        )),
    })?;
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
            mut methods,
        } = self;
        let name = payload.ident.clone();
        let name_str = name.to_string();
        let type_name_upper = name_str.to_uppercase();

        let krate = or_compile_error!(root_tokens());
        let meta_ty = parse_type_or!(&format!("{}::duka_shared::docs::MetaInfo", krate));
        let heap_type = parse_type_or!(&format!("{}::duka_gc::Heap", krate));
        let table_type = parse_type_or!(&format!("{}::value::RuntimeDukaTable", krate));
        let gc_cell_type = parse_type_or!(&format!("{}::duka_gc::GcCell", krate));
        let user_data_type = parse_type_or!(&format!("{}::value::UserData", krate));
        let user_data_payload_trait = parse_type_or!(&format!("{}::value::UserDataPayload", krate));

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

        if let Some(mut dm) = destructor {
            if !dm.attrs.iter().any(|a| a.path().is_ident("duka_builtin")) {
                if !dm
                    .sig
                    .inputs
                    .iter()
                    .any(|i| matches!(i, FnArg::Receiver(..)))
                {
                    return err(
                        dm.sig.span(),
                        "destructor must take `&self` or `&mut self`".to_owned(),
                    )
                    .into_compile_error();
                }
                dm.attrs.push(syn::parse_quote! {
                    #[duka_builtin(params(self: userdata), doc = "called when the value is collected")]
                });
            }
            dm.sig.ident = str2ident("__gc");
            methods.push(MethodItem {
                f: dm,
                in_block: false,
            });
        }

        let mut cleaned_methods: Vec<ItemFn> = vec![];
        let mut metatable_inserts: Vec<TokenStream> = vec![];
        let mut method_meta_fns: Vec<TokenStream> = vec![];
        let mut method_meta_idents: Vec<Ident> = vec![];
        let mut seen: Vec<String> = vec![];
        let mut has_index = false;

        for MethodItem {
            f: method,
            in_block,
        } in methods
        {
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
            let user_name = strip_impl_prefix(&method_ident);
            let mut duka_name = args.name.clone().unwrap_or_else(|| user_name.clone());
            if in_block && !duka_name.starts_with("__") {
                duka_name = format!("__{duka_name}");
            }
            if duka_name.starts_with("__") && !METAMETHODS.contains(&duka_name.as_str()) {
                let e = err(
                    attr.span(),
                    format!(
                        "unknown metamethod `{duka_name}`; expected one of: {}",
                        METAMETHODS.join(", ")
                    ),
                );
                return e.into_compile_error();
            }
            if seen.contains(&duka_name) {
                let e = err(attr.span(), format!("duplicate method `{duka_name}`"));
                return e.into_compile_error();
            }
            seen.push(duka_name.clone());
            if duka_name == "__index" {
                has_index = true;
            }

            let reads = match gen_arg_reads(&user_name, &method.sig, &args, &krate, 1, Some(&name))
            {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };
            let ArgReads {
                read_stmts,
                call_args,
                meta_params,
                has_co,
            } = reads;

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
                .map(|t| ty_to_kind(&t.ty, Span::call_site()).and_then(|i| i.meta.to_doc_type()))
                .collect::<Result<Vec<_>, Error>>()
            {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };

            let meta_ident = str2ident(&format!(
                "__DUKA_{}_{}_META",
                type_name_upper,
                duka_name.to_uppercase()
            ));
            let meta_fn = match gen_meta(
                &user_name,
                &meta_ident,
                &args,
                &meta_params,
                &meta_returns,
                &krate,
            ) {
                Ok(v) => v,
                Err(e) => return e.into_compile_error(),
            };

            let debug_name = format!("{}::{}", name_str, duka_name);
            let name_lit = LitStr::new(&duka_name, Span::call_site());
            let api_param = if has_co {
                quote! { api }
            } else {
                quote! { _api }
            };
            let closure = quote! {
                #krate::value::RustClosure::returns(
                    move |sv, h, #api_param| -> Result<#krate::duka_shared::types::ValueCount, #krate::errors::DukaRuntimeError> {
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
            let mut cleaned = strip_duka_attr(method);
            if duka_name == "__close" {
                cleaned.attrs.push(syn::parse_quote! {
                    #[deprecated(note = "`__close` is never dispatched by the duka vm; declare a `destructor` instead, it registers as `__gc`")]
                });
            }
            cleaned_methods.push(cleaned);
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
        let type_name_ident = str2ident(&format!("__DUKA_{}_NAME", type_name_upper));
        let type_example = match &struct_args.example {
            Some(e) => {
                let lit = LitStr::new(e, Span::call_site());
                quote! { Some(#lit) }
            }
            None => quote! { None },
        };
        let index_stmt = if has_index {
            quote! {}
        } else {
            quote! {
                __duka_mt.borrow_mut().set_by_key(heap, "__index".to_string(), #krate::value::RuntimeValue::Table(__duka_mt));
            }
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

                    #index_stmt
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
            pub const #type_name_ident: &str = #type_name_lit;
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
                flags: &[]
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_in_any_order() {
        let def: UserDataDef = syn::parse_str(
            r#"
            struct S;
            metamethod {
                #[duka_builtin(params(self: userdata))]
                fn impl_len(&self) -> Result<(), ()> { Ok(()) },
            }
            destructor fn drop(&mut self) -> Result<(), ()> { Ok(()) }
            constructor fn new() -> Self { S }
            #[duka_builtin(params(self: userdata))]
            fn m(&self) -> Result<(), ()> { Ok(()) },
        "#,
        )
        .unwrap();
        assert!(def.constructor.is_some());
        assert!(def.destructor.is_some());
        assert_eq!(def.methods.len(), 2);
        assert!(def.methods[0].in_block);
        assert!(!def.methods[1].in_block);
    }

    #[test]
    fn rejects_duplicate_sections() {
        let e = syn::parse_str::<UserDataDef>(
            "struct S; constructor fn a() -> Self { S } constructor fn b() -> Self { S }",
        )
        .unwrap_err();
        assert!(e.to_string().contains("duplicate `constructor`"));

        let e = syn::parse_str::<UserDataDef>(
            "struct S; destructor fn a(&self) -> Result<(), ()> { Ok(()) } destructor fn b(&self) -> Result<(), ()> { Ok(()) }",
        )
        .unwrap_err();
        assert!(e.to_string().contains("duplicate `destructor`"));
    }

    #[test]
    fn rejects_empty_metamethod_block() {
        let e = syn::parse_str::<UserDataDef>("struct S; metamethod {}").unwrap_err();
        assert!(e.to_string().contains("`metamethod` block is empty"));
    }

    #[test]
    fn rejects_missing_comma_between_methods() {
        let e = syn::parse_str::<UserDataDef>(
            r#"
            struct S;
            #[duka_builtin(params(self: userdata))]
            fn a(&self) -> Result<(), ()> { Ok(()) }
            #[duka_builtin(params(self: userdata))]
            fn b(&self) -> Result<(), ()> { Ok(()) }
        "#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("expected `,` between methods"));
    }

    #[test]
    fn metamethod_whitelist_covers_vm_and_csugar() {
        for name in [
            "__index",
            "__newindex",
            "__gc",
            "__close",
            "__tostring",
            "__add",
            "__call",
            "__bind",
            "__return",
            "__zero",
            "__while",
            "__forin",
            "__combine",
        ] {
            assert!(METAMETHODS.contains(&name), "missing {name}");
        }
        assert!(!METAMETHODS.contains(&"__toString"));
    }
}
