use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{Error, ItemConst, ItemFn, LitStr, Result, Signature, spanned::Spanned};

use crate::attr::*;
use crate::crate_path::resolve_root_str;

pub fn generate(item: TokenStream, attr: TokenStream) -> TokenStream {
    if let Ok(func) = syn::parse2::<ItemFn>(item.clone()) {
        match try_gen_func(func, attr) {
            Ok(ts) => ts,
            Err(e) => e.into_compile_error(),
        }
    } else if let Ok(conzt) = syn::parse2::<ItemConst>(item.clone()) {
        match try_gen_const(conzt, attr) {
            Ok(ts) => ts,
            Err(e) => e.into_compile_error(),
        }
    } else {
        Error::new(item.span(), "Only supports function and constant").into_compile_error()
    }
}

fn try_gen_const(conzt: ItemConst, attr: TokenStream) -> Result<TokenStream> {
    if !conzt.attrs.is_empty() || !conzt.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &conzt.ident,
            "duka_builtin cannot be combined with attributes or generics yet",
        ));
    }

    let args = parse_builtin_const_args(attr)?;

    let user_ident = conzt.ident.clone();
    let meta_ident = str2ident(&format!(
        "__DUKA_{}_META",
        user_ident.to_string().to_uppercase()
    ));
    let name_ident = str2ident(&format!(
        "__DUKA_{}_NAME",
        user_ident.to_string().to_uppercase()
    ));

    let krate: TokenStream = resolve_root_str().parse().unwrap();
    let meta_ty = parse_type(&format!("{}::duka_shared::docs::MetaInfo", krate));
    let name = LitStr::new(&args.name, Span::call_site());
    let doc = LitStr::new(&args.doc, Span::call_site());
    let example = match &args.example {
        Some(e) => {
            let lit = LitStr::new(e, Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };

    let ty_name = &args.ty.to_type();
    let val = LitStr::new(
        &args
            .val
            .unwrap_or_else(|| format!("{}", conzt.expr.to_token_stream())),
        Span::call_site(),
    );
    let vis = conzt.vis;
    let ty = conzt.ty;
    let expr = conzt.expr;

    let flags = args.flags.into_tokens();

    Ok(quote! {
        #vis const #user_ident: #ty = #expr;
        #[doc(hidden)]
        pub const #name_ident: &str = #name;
        #[cfg(feature = "docs")]
        #[doc(hidden)]
        #[allow(dead_code)]
        pub const #meta_ident: #meta_ty = #krate::duka_shared::docs::MetaInfo {
            name: #name,
            doc: #doc,
            info: #krate::duka_shared::docs::MetaItemInfo::Constant {
                ty: #ty_name,
                val: #val
            },
            example: #example,
            flags: #flags
        };
    })
}

fn try_gen_func(func: ItemFn, attr: TokenStream) -> Result<TokenStream> {
    let krate: TokenStream = resolve_root_str().parse().unwrap();
    if !func.attrs.is_empty() {
        return Err(Error::new_spanned(
            &func.sig,
            "duka_builtin cannot be combined with other attributes yet",
        ));
    }
    let args = parse_builtin_args(attr)?;

    let user_ident = func.sig.ident.clone();
    let user_ident_str = user_ident.to_string();
    let user_name = user_ident_str
        .strip_prefix("impl_")
        .unwrap_or(&user_ident_str);

    let internal_ident = str2ident(&format!("__duka_{}_impl", user_ident));
    let meta_ident = str2ident(&format!(
        "__DUKA_{}_META",
        user_ident.to_string().to_uppercase()
    ));

    let orig_sig = func.sig.clone();
    let orig_block = func.block.clone();

    let ArgReads {
        read_stmts,
        call_args,
        meta_params,
        has_co,
    } = gen_arg_reads(&user_name, &orig_sig, &args, &krate, 0, None)?;

    let meta_returns = args
        .returns
        .iter()
        .map(|t| ty_to_kind(&t.ty, Span::call_site()).map(|i| i.meta.to_doc_type()))
        .collect::<Result<Vec<_>>>()?;

    let return_kind = classify_return(&orig_sig.output)?;
    let epilog = gen_return(&return_kind, &krate)?;

    let internal_fn = ItemFn {
        attrs: vec![],
        vis: syn::Visibility::Inherited,
        sig: Signature {
            ident: internal_ident.clone(),
            ..orig_sig
        },
        block: orig_block,
    };

    let reg_name = args.name.clone().unwrap_or_else(|| user_name.to_string());
    let name_ident = str2ident(&format!(
        "__DUKA_{}_NAME",
        user_ident.to_string().to_uppercase()
    ));
    let name_lit = LitStr::new(&reg_name, Span::call_site());
    let meta_fn = gen_meta(
        user_name,
        &meta_ident,
        &args,
        &meta_params,
        &meta_returns,
        &krate,
    );

    let vis = &func.vis;

    let root_str = resolve_root_str();
    let co_state = parse_type(&format!("&mut {}::vm::coroutine::CoState", root_str));
    let heap = parse_type(&format!("&mut {}::duka_gc::Heap", root_str));
    let sv = mut_ref_arg("sv", &co_state);
    let h = mut_ref_arg("h", &heap);

    let retty = parse_type(&format!(
        "Result<{}::duka_shared::types::ValueCount, {}::errors::DukaRuntimeError>",
        root_str, root_str
    ));
    let wrapper_block = quote! {
        #(#read_stmts)*
        let __ret = #internal_ident(#(#call_args),*)?;
        #epilog
    };

    let inputs = if has_co {
        let native_api = parse_type(&format!("&mut {}::vm::coroutine::NativeApi", root_str));
        let api = mut_ref_arg("api", &native_api);
        quote! { #sv, #h, #api }
    } else {
        quote! { #sv, #h }
    };

    let out = quote! {
        #internal_fn
        #[doc(hidden)]
        pub const #name_ident: &str = #name_lit;
        #[doc(hidden)]
        #[allow(dead_code, unused_variables)]
        #vis fn #user_ident(#inputs) -> #retty {
            #wrapper_block
        }
        #meta_fn
    };
    Ok(out)
}
