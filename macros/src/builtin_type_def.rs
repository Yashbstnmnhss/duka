use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{
    Attribute, Ident, ItemConst, ItemFn, LitInt, LitStr, Meta, Path, ReturnType, Token, braced,
    parenthesized, parse::Parse, punctuated::Punctuated,
};

use crate::attr::{
    AttrShape, MetaInfoFlags, TypeParamEntry, const_type, for_each_attr_key, lit_str,
    parse_builtin_type_args, strip_impl_prefix, unwrap_paren,
};

mod kw {
    use syn::custom_keyword;
    custom_keyword!(name);
    custom_keyword!(doc);
    custom_keyword!(example);
    custom_keyword!(flags);
}

struct ParamSpec {
    name: String,
    /// The `Kind` expression the check is generated from; `None` checks nothing.
    kind: Option<TokenStream>,
    /// How the kind reads in the language, for the meta table.
    spelling: String,
}

/// `A`, `B`, ... for a parameter written without a name of its own.
fn positional_name(index: usize) -> String {
    if index < 26 {
        ((b'A' + index as u8) as char).to_string()
    } else {
        format!("A{index}")
    }
}

struct TypeFnEntry {
    func: ItemFn,
    name: String,
    doc: String,
    example: Option<String>,
    params: Vec<ParamSpec>,
    env: bool,
    flags: MetaInfoFlags,
}

fn split_duka_attr(attrs: Vec<Attribute>) -> syn::Result<(Vec<Attribute>, Option<TokenStream>)> {
    let mut found = None;
    let mut kept = Vec::with_capacity(attrs.len());
    for a in attrs {
        let is_duka = a
            .path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "duka_builtin");
        if is_duka {
            if found.is_some() {
                return Err(syn::Error::new_spanned(
                    &a,
                    "duplicate `#[duka_builtin]` attribute",
                ));
            }
            found = Some(match &a.meta {
                Meta::List(list) => list.tokens.clone(),
                _ => {
                    return Err(syn::Error::new_spanned(
                        a,
                        "expected `#[duka_builtin(...)]`",
                    ));
                }
            });
        } else {
            kept.push(a);
        }
    }
    Ok((kept, found))
}

impl Parse for TypeFnEntry {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut func: ItemFn = input.parse()?;
        let (attrs, tokens) = split_duka_attr(std::mem::take(&mut func.attrs))?;
        func.attrs = attrs;
        let tokens = tokens.ok_or_else(|| {
            syn::Error::new_spanned(
                &func.sig.ident,
                "type function requires `#[duka_builtin(...)]`",
            )
        })?;
        let args = parse_builtin_type_args(tokens)?;
        let (params, env) = if !args.params.is_empty() {
            if args.count.is_some() || !args.binds.is_empty() {
                return Err(syn::Error::new_spanned(
                    &func.sig.ident,
                    "`params` replaces `count` and `bind`; write one or the other",
                ));
            }
            let params: Vec<ParamSpec> = args
                .params
                .into_iter()
                .map(
                    |TypeParamEntry {
                         name,
                         kind,
                         spelling,
                     }| ParamSpec {
                        name,
                        kind: Some(kind),
                        spelling,
                    },
                )
                .collect();
            (params, args.env)
        } else {
            let count = args.count.ok_or_else(|| {
                syn::Error::new_spanned(&func.sig.ident, "type function requires `params`")
            })?;
            let mut binds = args.binds;
            for b in &binds {
                if b.index >= count {
                    return Err(syn::Error::new_spanned(
                        &func.sig.ident,
                        format!(
                            "bound index {} is out of range for `count` {}",
                            b.index, count
                        ),
                    ));
                }
            }
            binds.sort_by_key(|b| b.index);
            let params = (0..count)
                .map(|i| {
                    let name = positional_name(i);
                    match binds.iter().find(|b| b.index == i) {
                        Some(b) => {
                            let ty = &b.ty;
                            ParamSpec {
                                name,
                                kind: Some(quote!(::duka_shared::dtype::Kind::OfType(#ty))),
                                spelling: crate::attr::type_spelling(
                                    &syn::parse2::<syn::Expr>(ty.clone())
                                        .unwrap_or_else(|_| syn::parse_quote!(Type::Any)),
                                ),
                            }
                        }
                        None => ParamSpec {
                            name,
                            kind: None,
                            spelling: "type".to_owned(),
                        },
                    }
                })
                .collect();
            (params, args.env)
        };
        if func.sig.inputs.len() != if env { 2 } else { 1 } {
            return Err(syn::Error::new_spanned(
                &func.sig.inputs,
                if env {
                    "an `env` type function takes `&mut TypeEnv` and `Box<[TypeValue]>`"
                } else {
                    "type function takes exactly one `Box<[TypeValue]>` argument"
                },
            ));
        }
        match &func.sig.output {
            ReturnType::Type(_, ty) => {
                if !ty.to_token_stream().to_string().ends_with("TypeResult") {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "type function must return `TypeResult`",
                    ));
                }
            }
            ReturnType::Default => {
                return Err(syn::Error::new_spanned(
                    &func.sig.output,
                    "type function must return `TypeResult`",
                ));
            }
        }

        let name = if args.name.is_empty() {
            strip_impl_prefix(&func.sig.ident)
        } else {
            args.name
        };
        Ok(Self {
            func,
            name,
            doc: args.doc,
            example: args.example,
            params,
            env,
            flags: args.flags,
        })
    }
}

struct TypeConstEntry {
    konst: ItemConst,
    name: String,
    doc: String,
    example: Option<String>,
    ty: Option<TokenStream>,
    val: String,
    flags: MetaInfoFlags,
}

impl Parse for TypeConstEntry {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut konst: ItemConst = input.parse()?;
        let (attrs, tokens) = split_duka_attr(std::mem::take(&mut konst.attrs))?;
        konst.attrs = attrs;
        let tokens = tokens.ok_or_else(|| {
            syn::Error::new_spanned(
                &konst.ident,
                "type constant requires `#[duka_builtin(...)]`",
            )
        })?;
        if !konst.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &konst.generics,
                "type constant cannot have generics",
            ));
        }
        let mut name = String::new();
        let mut doc = String::new();
        let mut example = None;
        let mut ty = None;
        let mut val = String::new();
        let mut flags = MetaInfoFlags::default();
        for_each_attr_key(tokens, AttrShape::Constant, |key, rest, span| match key {
            "type" => {
                let text = lit_str(&rest)?;
                let meta = const_type(&text, span)?;
                ty = Some(meta.to_doc_type()?);
                Ok(())
            }
            "name" => {
                name = lit_str(&rest)?;
                Ok(())
            }
            "doc" => {
                doc = lit_str(&rest)?;
                Ok(())
            }
            "example" => {
                example = Some(lit_str(&rest)?);
                Ok(())
            }
            "value" => {
                val = lit_str(&rest)?;
                Ok(())
            }
            "flags" => {
                let inner = unwrap_paren(&rest)?;
                flags = syn::parse2(inner)?;
                Ok(())
            }
            _ => Err(syn::Error::new(
                span,
                format!("unhandled constant attribute `{key}`"),
            )),
        })?;
        let name = if name.is_empty() {
            strip_impl_prefix(&konst.ident)
        } else {
            name
        };
        Ok(Self {
            konst,
            name,
            doc,
            example,
            ty,
            val,
            flags,
        })
    }
}

pub struct BuiltinTypeDef {
    name: Ident,
    meta_name: Option<String>,
    doc: String,
    example: Option<String>,
    flags: MetaInfoFlags,
    fns: Vec<TypeFnEntry>,
    consts: Vec<TypeConstEntry>,
    mods: Vec<Path>,
}

impl Parse for BuiltinTypeDef {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<Token![type]>()?;
        input.parse::<Token![mod]>()?;
        let name = input.parse::<Ident>()?;

        let mut meta_name = None;
        let mut doc = None;
        let mut example = None;
        let mut flags = None;
        loop {
            if input.peek(kw::name) {
                if meta_name.is_some() {
                    return Err(input.error("duplicate `name`"));
                }
                input.parse::<kw::name>()?;
                meta_name = Some(input.parse::<LitStr>()?.value());
            } else if input.peek(kw::doc) {
                if doc.is_some() {
                    return Err(input.error("duplicate `doc`"));
                }
                input.parse::<kw::doc>()?;
                doc = Some(input.parse::<LitStr>()?.value());
            } else if input.peek(kw::example) {
                if example.is_some() {
                    return Err(input.error("duplicate `example`"));
                }
                input.parse::<kw::example>()?;
                example = Some(input.parse::<LitStr>()?.value());
            } else if input.peek(kw::flags) {
                if flags.is_some() {
                    return Err(input.error("duplicate `flags`"));
                }
                input.parse::<kw::flags>()?;
                let inner;
                parenthesized!(inner in input);
                flags = Some(inner.parse::<MetaInfoFlags>()?);
            } else {
                break;
            }
        }

        let mut fns: Option<Vec<TypeFnEntry>> = None;
        let mut consts: Option<Vec<TypeConstEntry>> = None;
        let mut mods: Option<Punctuated<Path, Token![,]>> = None;

        while !input.is_empty() {
            if input.peek(kw::name)
                || input.peek(kw::doc)
                || input.peek(kw::example)
                || input.peek(kw::flags)
            {
                return Err(
                    input.error("`name`, `doc`, `example` and `flags` must come before sections")
                );
            } else if input.peek(Token![fn]) {
                if fns.is_some() {
                    return Err(input.error("duplicate `fn` section"));
                }
                input.parse::<Token![fn]>()?;
                let inner;
                braced!(inner in input);
                fns = Some(
                    inner
                        .parse_terminated(TypeFnEntry::parse, Token![,])?
                        .into_iter()
                        .collect(),
                );
            } else if input.peek(Token![const]) {
                if consts.is_some() {
                    return Err(input.error("duplicate `const` section"));
                }
                input.parse::<Token![const]>()?;
                let inner;
                braced!(inner in input);
                consts = Some(
                    inner
                        .parse_terminated(TypeConstEntry::parse, Token![,])?
                        .into_iter()
                        .collect(),
                );
            } else if input.peek(Token![mod]) {
                if mods.is_some() {
                    return Err(input.error("duplicate `mod` section"));
                }
                input.parse::<Token![mod]>()?;
                let inner;
                braced!(inner in input);
                mods = Some(inner.parse_terminated(Path::parse, Token![,])?);
            } else {
                return Err(
                    input.error("expected a type section: `fn {…}`, `const {…}` or `mod {…}`")
                );
            }
        }

        let fns = fns.unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        for e in &fns {
            if !seen.insert(e.name.clone()) {
                return Err(syn::Error::new_spanned(
                    &e.func.sig.ident,
                    format!("duplicate type function `{}`", e.name),
                ));
            }
        }
        let consts = consts.unwrap_or_default();
        let mut seen_const = std::collections::HashSet::new();
        for e in &consts {
            if !seen_const.insert(e.name.clone()) {
                return Err(syn::Error::new_spanned(
                    &e.konst.ident,
                    format!("duplicate type constant `{}`", e.name),
                ));
            }
        }
        let mods = mods.unwrap_or_default().into_iter().collect();

        Ok(Self {
            name,
            meta_name,
            doc: doc.unwrap_or_default(),
            example,
            flags: flags.unwrap_or_default(),
            fns,
            consts,
            mods,
        })
    }
}

impl BuiltinTypeDef {
    pub fn generate(self) -> TokenStream {
        let Self {
            name,
            meta_name,
            doc,
            example,
            flags,
            fns,
            consts,
            mods,
        } = self;

        let fn_items = fns.iter().map(|e| &e.func);
        let const_items = consts.iter().map(|e| &e.konst);

        let fn_registers = fns.iter().map(|e| {
            let ident = &e.func.sig.ident;
            let name_lit = LitStr::new(&e.name, ident.span());
            let required = LitInt::new(&e.params.len().to_string(), Span::call_site());
            let checks = e.params.iter().enumerate().map(|(index, spec)| {
                let idx = LitInt::new(&index.to_string(), Span::call_site());
                let param = LitStr::new(&spec.name, ident.span());
                let spelling = LitStr::new(&spec.spelling, ident.span());
                match &spec.kind {
                    Some(kind) => quote! {
                        if let Some(__a) = __args.get(#idx)
                            && let Err(__why) = crate::analyzer::tyval::kind_accepts(&(#kind), __a)
                        {
                            return Err(format!(
                                "Expected {} for the type argument `{}`: {}",
                                #spelling, #param, __why,
                            ));
                        }
                    },
                    None => quote! {},
                }
            });

            if e.env {
                quote! {
                    b = b.register(
                        #name_lit,
                        ::duka_shared::dtype::Ty::Ext(crate::analyzer::tyval::TypeLeaf::Closure(
                            Box::new(crate::analyzer::tyval::TypeClosure::rust(
                                #name_lit,
                                crate::analyzer::builtin::TypeFunctionBuiltin::Env(
                                    |__env, __args| {
                                        if __args.len() < #required {
                                            return Err(concat!("Expected ", stringify!(#required), " type argument").to_owned());
                                        }
                                        #(#checks)*
                                        #ident(__env, __args)
                                    },
                                ),
                            )),
                        )),
                    );
                }
            } else {
                quote! {
                    b = b.register(
                        #name_lit,
                        ::duka_shared::dtype::Ty::Ext(crate::analyzer::tyval::TypeLeaf::Closure(
                            Box::new(crate::analyzer::tyval::TypeClosure::rust(
                                #name_lit,
                                crate::analyzer::builtin::TypeFunctionBuiltin::Plain(
                                    |__args| {
                                        if __args.len() < #required {
                                            return Err(concat!("Expected ", stringify!(#required), " type argument").to_owned());
                                        }
                                        #(#checks)*
                                        #ident(__args)
                                    },
                                ),
                            )),
                        )),
                    );
                }
            }
        });

        let const_registers = consts.iter().map(|e| {
            let ident = &e.konst.ident;
            let name_lit = LitStr::new(&e.name, ident.span());
            quote! {
                b = b.register(#name_lit, ::duka_shared::dtype::Ty::Ext(crate::analyzer::tyval::TypeLeaf::Final(#ident)));
            }
        });

        let mod_registers = mods.iter().map(|path| {
            quote! {
                for (k, v) in #path::registry().into_inner() {
                    b = b.register(k, v);
                }
            }
        });

        let fn_metas = fns.iter().map(|e| {
            let name_lit = LitStr::new(&e.name, Span::call_site());
            let doc_lit = LitStr::new(&e.doc, Span::call_site());
            let ex = e
                .example
                .as_ref()
                .map(|i| quote! { Some(#i) })
                .unwrap_or(quote! { None });
            let param_entries = e.params.iter().map(|p| {
                let name_lit = LitStr::new(&p.name, Span::call_site());
                let kind_lit = LitStr::new(&p.spelling, Span::call_site());
                quote! {
                    ::duka_shared::docs::TypeParamMeta {
                        name: #name_lit,
                        kind: #kind_lit,
                    }
                }
            });
            let flags = e.flags.clone().into_tokens();
            quote! {
                ::duka_shared::docs::MetaInfo {
                    name: #name_lit,
                    doc: #doc_lit,
                    example: #ex,
                    info: ::duka_shared::docs::MetaItemInfo::Type {
                        params: &[#(#param_entries),*]
                    },
                    flags: #flags,
                }
            }
        });

        let const_metas = consts.iter().map(|e| {
            let name_lit = LitStr::new(&e.name, Span::call_site());
            let doc_lit = LitStr::new(&e.doc, Span::call_site());
            let ex = e
                .example
                .as_ref()
                .map(|i| quote! { Some(#i) })
                .unwrap_or(quote! { None });
            let val = LitStr::new(&e.val, Span::call_site());
            let ty = e.ty.clone().unwrap_or_else(|| {
                quote! { ::duka_shared::docs::DocType::Base(::duka_shared::dtype::Type::Any) }
            });
            let flags = e.flags.clone().into_tokens();
            quote! {
                ::duka_shared::docs::MetaInfo {
                    name: #name_lit,
                    doc: #doc_lit,
                    example: #ex,
                    info: ::duka_shared::docs::MetaItemInfo::Constant {
                        ty: #ty,
                        val: #val,
                    },
                    flags: #flags,
                }
            }
        });

        let mod_metas = mods.iter().map(|path| quote! { #path::TYPE_MODULE_META });

        let all_meta_list: Vec<TokenStream> =
            fn_metas.chain(const_metas).chain(mod_metas).collect();

        let meta_name = meta_name.unwrap_or_else(|| name.to_string());
        let meta_name_lit = LitStr::new(&meta_name, name.span());
        let name_lit = LitStr::new(&name.to_string(), name.span());
        let doc_lit = LitStr::new(&doc, Span::call_site());
        let example = example.map(|i| quote! {Some(#i)}).unwrap_or(quote! {None});
        let flags = flags.into_tokens();

        quote! {
            #(
                #[allow(non_snake_case)]
                #fn_items
            )*
            #(#const_items)*

            pub fn registry() -> ::duka_shared::builtin::Builtins<crate::analyzer::tyval::TypeValue> {
                let mut b = ::duka_shared::builtin::Builtins::new();
                #(#fn_registers)*
                #(#const_registers)*
                #(#mod_registers)*
                b
            }

            pub const TYPE_MODULE_NAME: &str = #name_lit;

            #[doc(hidden)]
            #[allow(dead_code)]
            pub const TYPE_MODULE_META: ::duka_shared::docs::MetaInfo = ::duka_shared::docs::MetaInfo {
                name: #meta_name_lit,
                doc: #doc_lit,
                example: #example,
                info: ::duka_shared::docs::MetaItemInfo::Module {
                    inner: &[
                        #(#all_meta_list),*
                    ]
                },
                flags: #flags
            };
        }
    }
}
