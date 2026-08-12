use proc_macro2::{Delimiter, Ident, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use syn::{
    Error, FnArg, GenericArgument, ItemConst, ItemFn, LitStr, PatIdent, PathArguments, Result,
    ReturnType, Signature, Type, TypePath, spanned::Spanned,
};

const CRATE: &str = "crate";

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

    let mut args = parse_builtin_const_args(attr)?;
    args.ty = arg_kind(&conzt.ty)?.meta;

    let user_ident = conzt.ident.clone();
    let meta_ident = format_ident(&format!(
        "__DUKA_{}_META",
        user_ident.to_string().to_uppercase()
    ));

    let meta_ty = parse_type("::duka_shared::docs::MetaInfo");
    let module = LitStr::new(&args.module, proc_macro2::Span::call_site());
    let name = LitStr::new(&args.name, proc_macro2::Span::call_site());
    let doc = LitStr::new(&args.doc, proc_macro2::Span::call_site());
    let example = match &args.example {
        Some(e) => {
            let lit = LitStr::new(e, proc_macro2::Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };

    let ty_name = &args.ty.to_type();
    let val = LitStr::new(
        &args
            .val
            .unwrap_or_else(|| format!("{}", conzt.expr.to_token_stream())),
        proc_macro2::Span::call_site(),
    );
    let vis = conzt.vis;
    let ty = conzt.ty;
    let expr = conzt.expr;

    Ok(quote! {
        #vis const #user_ident: #ty = #expr;
        #[doc(hidden)]
        #[allow(dead_code)]
        pub const #meta_ident: #meta_ty = ::duka_shared::docs::MetaInfo {
            module: #module,
            name: #name,
            doc: #doc,
            info: ::duka_shared::docs::MetaItemInfo::Constant {
                ty: #ty_name,
                val: #val
            },
            example: #example,
        };
    })
}

fn try_gen_func(func: ItemFn, attr: TokenStream) -> Result<TokenStream> {
    let krate = Ident::new(CRATE, proc_macro2::Span::call_site());
    if !func.attrs.is_empty() {
        return Err(Error::new_spanned(
            &func.sig,
            "duka_builtin cannot be combined with other attributes yet",
        ));
    }
    let args = parse_builtin_args(attr)?;

    let user_ident = func.sig.ident.clone();
    let internal_ident = format_ident(&format!("__duka_{}_impl", user_ident));
    let meta_ident = format_ident(&format!(
        "__DUKA_{}_META",
        user_ident.to_string().to_uppercase()
    ));

    let orig_sig = func.sig.clone();
    let orig_block = func.block.clone();

    let mut read_stmts: Vec<TokenStream> = Vec::new();
    let mut call_args: Vec<TokenStream> = Vec::new();
    let mut meta_params: Vec<TokenStream> = Vec::new();
    let mut param_i = 0usize;
    let mut has_co = false;

    for arg in &orig_sig.inputs {
        let FnArg::Typed(pt) = arg else { continue };
        let name = match &*pt.pat {
            syn::Pat::Ident(PatIdent { ident, .. }) => ident.clone(),
            _ => return Err(Error::new_spanned(pt, "unsupported parameter pattern")),
        };
        let ty = &*pt.ty;
        if is_ref_ident(ty, "CoState") {
            call_args.push(quote! { sv });
            continue;
        }
        if is_ref_ident(ty, "Heap") {
            call_args.push(quote! { h });
            continue;
        }
        if is_ref_ident(ty, "NativeApi") {
            call_args.push(quote! { api });
            has_co = true;
            continue;
        }

        let meta = args.params.get(param_i).ok_or_else(|| {
            Error::new_spanned(
                ty,
                "params(...) must declare every non-injected argument, in order",
            )
        })?;

        let kind = (&meta.ty)
            .as_ref()
            .map(|t| ty_to_kind(t, proc_macro2::Span::call_site()))
            .unwrap_or_else(|| arg_kind(ty))?;

        let idx = proc_macro2::Literal::usize_unsuffixed(param_i);
        param_i += 1;
        let name_lit = LitStr::new(&meta.name, proc_macro2::Span::call_site());
        let helper = Ident::new(kind.helper, proc_macro2::Span::call_site());
        let args = if let Some(members) = &kind.union_members {
            let ctypes: Vec<TokenStream> = members
                .iter()
                .map(|m| {
                    let m = Ident::new(m, proc_macro2::Span::call_site());
                    quote! { ::duka_shared::constants::ctype::#m }
                })
                .collect();
            let want = LitStr::new(
                &meta.ty.clone().unwrap_or_default(),
                proc_macro2::Span::call_site(),
            );
            quote! { (sv, #idx, #name_lit, &[#(#ctypes),*], #want) }
        } else {
            quote! { (sv, #idx, #name_lit) }
        };
        let stmt = if let Some(default) = &meta.default {
            quote! {
                let #name: #ty = match #krate::builtin::arg::#helper #args {
                    Ok(v) => v,
                    Err(#krate::errors::DukaRuntimeError::ArgumentMissing(..)) => #default,
                    Err(e) => return Err(e),
                };
            }
        } else {
            quote! {
                let #name: #ty = #krate::builtin::arg::#helper #args?;
            }
        };
        read_stmts.push(stmt);
        call_args.push(quote! { #name });
        meta_params.push(meta_param_tokens(meta, kind));
    }

    if param_i != args.params.len() {
        return Err(Error::new_spanned(
            &func.sig,
            "params(...) count must match the number of non-injected arguments",
        ));
    }

    let meta_returns = args
        .returns
        .iter()
        .map(|t| ty_to_kind(&t.ty, proc_macro2::Span::call_site()).map(|i| i.meta.to_doc_type()))
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

    let user_ident_str = user_ident.to_string();
    let user_name = user_ident_str
        .strip_prefix("impl_")
        .unwrap_or(&user_ident_str);
    let meta_fn = gen_meta(user_name, &meta_ident, &args, &meta_params, &meta_returns);

    let vis = &func.vis;

    let co_state = parse_type(&format!("&mut {}::vm::coroutine::CoState", CRATE));
    let heap = parse_type("&mut ::duka_gc::Heap");
    let sv = mut_ref_arg("sv", &co_state);
    let h = mut_ref_arg("h", &heap);

    let retty = parse_type(&format!(
        "Result<::duka_shared::types::ValueCount, {}::errors::DukaRuntimeError>",
        CRATE
    ));
    let wrapper_block = quote! {
        #(#read_stmts)*
        let __ret = #internal_ident(#(#call_args),*)?;
        #epilog
    };

    let inputs = if has_co {
        let native_api = parse_type(&format!("&mut {}::vm::coroutine::NativeApi", CRATE));
        let api = mut_ref_arg("api", &native_api);
        quote! { #sv, #h, #api }
    } else {
        quote! { #sv, #h }
    };

    let out = quote! {
        #internal_fn
        #[doc(hidden)]
        #[allow(dead_code, unused_variables)]
        #vis fn #user_ident(#inputs) -> #retty {
            #wrapper_block
        }
        #meta_fn
    };
    Ok(out)
}

fn gen_meta(
    user_name: &str,
    meta_ident: &Ident,
    args: &BuiltinArgs,
    meta_params: &[TokenStream],
    meta_returns: &[TokenStream],
) -> TokenStream {
    let meta_ty = parse_type("::duka_shared::docs::MetaInfo");
    let module = LitStr::new(&args.module, proc_macro2::Span::call_site());
    let name = LitStr::new(
        &args.name.as_ref().map(|i| i.as_str()).unwrap_or(user_name),
        proc_macro2::Span::call_site(),
    );
    let doc = LitStr::new(&args.doc, proc_macro2::Span::call_site());
    let ret_text = LitStr::new(&args.return_doc, proc_macro2::Span::call_site());
    let example = match &args.example {
        Some(e) => {
            let lit = LitStr::new(e, proc_macro2::Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };
    let ret_var_arg = args.return_var_arg;
    quote! {
        #[doc(hidden)]
        #[allow(dead_code)]
        pub const #meta_ident: #meta_ty = ::duka_shared::docs::MetaInfo {
            module: #module,
            name: #name,
            doc: #doc,
            info: ::duka_shared::docs::MetaItemInfo::Function {
                returns: ::duka_shared::docs::ReturnMeta {
                    text: #ret_text,
                    tys: &[#(#meta_returns),*],
                    var_arg: #ret_var_arg,
                },
                params: &[#(#meta_params),*],
            },
            example: #example,
        };
    }
}

fn gen_return(kind: &ReturnKind, krate: &Ident) -> Result<TokenStream> {
    let vc = quote! { ::duka_shared::types::ValueCount };
    Ok(match kind {
        ReturnKind::Zero => quote! { Ok(#vc::Exact(0)) },
        ReturnKind::One => quote! { sv.set_stack(0, __ret)?; Ok(#vc::Exact(1)) },
        ReturnKind::Dynamic => {
            quote! { sv.set_stack_many(0, &__ret)?; Ok(#vc::Exact(__ret.len())) }
        }
        ReturnKind::Many(tys) => {
            let n = tys.len();
            let n_lit = proc_macro2::Literal::usize_unsuffixed(n);
            let pats: Vec<Ident> = (0..n).map(|i| format_ident(&format!("__e{}", i))).collect();
            let mut sets = Vec::new();
            for (i, t) in tys.iter().enumerate() {
                let idx = proc_macro2::Literal::usize_unsuffixed(i);
                let conv = conv_expr(t, &pats[i], krate)?;
                sets.push(quote! { sv.set_stack(#idx, #conv)?; });
            }
            quote! {
                let (#(#pats),*) = __ret;
                #(#sets)*
                Ok(#vc::Exact(#n_lit))
            }
        }
    })
}

enum ReturnKind {
    Zero,
    One,
    Dynamic,
    Many(Vec<Type>),
}

fn classify_return(output: &ReturnType) -> Result<ReturnKind> {
    let ty = match output {
        ReturnType::Type(_, ty) => ty.as_ref(),
        ReturnType::Default => {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "duka_builtin functions must declare an explicit return type",
            ));
        }
    };
    let Type::Path(TypePath { path, .. }) = ty else {
        return Err(Error::new_spanned(
            ty,
            "unsupported return type; use Result<...>",
        ));
    };
    let last = path
        .segments
        .last()
        .map(|s| s.ident == "Result")
        .unwrap_or(false);
    if !last {
        return Err(Error::new_spanned(
            ty,
            "duka_builtin functions must return Result<T, E>",
        ));
    }
    let PathArguments::AngleBracketed(ab) = &path.segments.last().unwrap().arguments else {
        return Err(Error::new_spanned(ty, "Result requires type arguments"));
    };
    let mut tys = Vec::new();
    for a in &ab.args {
        if let GenericArgument::Type(t) = a {
            tys.push(t.clone());
        }
    }
    if tys.len() < 2 {
        return Err(Error::new_spanned(ty, "Result requires T and E"));
    }
    let ok = &tys[0];
    match ok {
        Type::Tuple(tup) if tup.elems.is_empty() => Ok(ReturnKind::Zero),
        Type::Tuple(tup) => Ok(ReturnKind::Many(tup.elems.iter().cloned().collect())),
        Type::Path(p) => {
            let last = p
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            match last.as_str() {
                "RuntimeValue" => Ok(ReturnKind::One),
                "Vec" => {
                    let inner = single_generic(ok)
                        .ok_or_else(|| Error::new_spanned(ok, "Vec requires a type argument"))?;
                    if last_seg_ident(&inner).as_deref() == Some("RuntimeValue") {
                        Ok(ReturnKind::Dynamic)
                    } else {
                        Err(Error::new_spanned(
                            ok,
                            "Vec return element type must be RuntimeValue",
                        ))
                    }
                }
                _ => Err(Error::new_spanned(
                    ok,
                    "unsupported Result Ok type; use RuntimeValue, (), Vec<RuntimeValue> or a tuple",
                )),
            }
        }
        _ => Err(Error::new_spanned(ok, "unsupported Result Ok type")),
    }
}

fn conv_expr(ty: &Type, bind: &Ident, krate: &Ident) -> Result<TokenStream> {
    let name = last_seg_ident(ty).unwrap_or_default();
    let rv = quote! { #krate::value::RuntimeValue };
    Ok(match name.as_str() {
        "DukaInt" | "i64" => quote! { #rv::Int(#bind) },
        "DukaFloat" | "f64" => quote! { #rv::Float(#bind) },
        "bool" => quote! { #rv::Bool(#bind) },
        "RuntimeValue" => quote! { #bind },
        "Vec" => {
            let inner = single_generic(ty)
                .ok_or_else(|| Error::new_spanned(ty, "Vec requires a type argument"))?;
            if last_seg_ident(&inner).as_deref() == Some("u8") {
                quote! { #rv::from_string(h, String::from_utf8_lossy(&#bind).into_owned()) }
            } else if last_seg_ident(&inner).as_deref() == Some("RuntimeValue") {
                quote! {
                    #rv::
                }
            } else {
                return Err(Error::new_spanned(
                    ty,
                    "unsupported Vec element type in tuple return",
                ));
            }
        }
        "String" => quote! { #rv::from_string(h, #bind) },
        _ => return Err(Error::new_spanned(ty, "unsupported tuple element type")),
    })
}

#[derive(Debug)]
enum ParamTypeName {
    String,
    Int,
    Num,
    Bool,
    Table,
    Function,
    Array,
    Any,
    Nil,
    Bytes,
    PreserveNumber,
    VarArg,
    Union(Vec<ParamTypeName>),
}

impl ParamTypeName {
    fn get_type_variant(&self) -> &'static str {
        match self {
            ParamTypeName::String => "String",
            ParamTypeName::Int => "Int",
            ParamTypeName::Num => "Float",
            ParamTypeName::Bool => "Bool",
            ParamTypeName::Table => "Table",
            //ParamTypeName::Function => "Function",
            ParamTypeName::Any => "Any",
            ParamTypeName::Nil => "Nil",
            ParamTypeName::Array => "Array",
            _ => panic!("Type is not supported here"),
        }
    }

    fn to_doc_type(&self) -> TokenStream {
        let base = "::duka_shared::docs::DocType";
        let s = match self {
            ParamTypeName::PreserveNumber => format!("{base}::PreserveNumber"),
            ParamTypeName::Bytes => format!("{base}::Bytes"),
            ParamTypeName::Function => {
                format!("{base}::Base(::duka_shared::dtype::Type::Function(None))")
            }
            ParamTypeName::VarArg => {
                format!("{base}::Base(::duka_shared::dtype::Type::Any)")
            }
            ParamTypeName::Union(items) => {
                let inner = items
                    .iter()
                    .map(|i| i.to_doc_type().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{base}::Union(&[{inner}])")
            }
            _ => format!(
                "{base}::Base(::duka_shared::dtype::Type::{})",
                self.get_type_variant()
            ),
        };
        s.parse::<TokenStream>().unwrap()
    }

    /// to Type enum
    fn to_type(&self) -> TokenStream {
        format!("::duka_shared::dtype::Type::{}", self.get_type_variant())
            .parse::<TokenStream>()
            .unwrap()
    }
}

struct ArgKind {
    helper: &'static str,
    meta: ParamTypeName,
    union_members: Option<Vec<&'static str>>,
}

fn last_seg_ident(ty: &Type) -> Option<String> {
    if let Type::Path(TypePath { path, .. }) = ty {
        return path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

fn single_generic(ty: &Type) -> Option<Type> {
    if let Type::Path(TypePath { path, .. }) = ty {
        if let PathArguments::AngleBracketed(ab) = &path.segments.last()?.arguments {
            for a in &ab.args {
                if let GenericArgument::Type(t) = a {
                    return Some(t.clone());
                }
            }
        }
    }
    None
}

fn is_ref_ident(ty: &Type, ident: &str) -> bool {
    if let Type::Reference(r) = ty {
        if let Type::Path(p) = &*r.elem {
            return p
                .path
                .segments
                .last()
                .map(|s| s.ident == ident)
                .unwrap_or(false);
        }
    }
    false
}

fn ty_to_kind(ty: &str, span: Span) -> Result<ArgKind> {
    if ty.contains('|') {
        let mut inner = Vec::new();
        let mut kinds = Vec::new();
        for m in ty.split('|') {
            let m = m.trim();
            if m.is_empty() {
                continue;
            }
            let k = simple_kind(m, span)?;
            kinds.push(member_ctype(m));
            inner.push(k.meta);
        }
        return Ok(ArgKind {
            helper: "take_union",
            meta: ParamTypeName::Union(inner),
            union_members: Some(kinds),
        });
    }
    simple_kind(ty, span)
}

fn member_ctype(m: &str) -> &'static str {
    match m {
        "int" => "INT",
        "float" | "num" | "number" | "preserve_number" => "NUM",
        "str" | "string" | "bytes" => "STR",
        "bool" => "BOO",
        "table" => "TAB",
        "list" | "array" => "ARR",
        "function" | "func" | "fn" => "FUN",
        "nil" => "NIL",
        "*" | "any" => "ANY",
        _ => "ANY",
    }
}

fn simple_kind(ty: &str, span: Span) -> Result<ArgKind> {
    Ok(match ty {
        "int" => ArgKind {
            helper: "take_int",
            meta: ParamTypeName::Int,
            union_members: None,
        },
        "float" | "num" => ArgKind {
            helper: "take_num",
            meta: ParamTypeName::Num,
            union_members: None,
        },
        "number" | "preserve_number" => ArgKind {
            helper: "take_number",
            meta: ParamTypeName::PreserveNumber,
            union_members: None,
        },
        "str" | "string" => ArgKind {
            helper: "take_string",
            meta: ParamTypeName::String,
            union_members: None,
        },
        "bytes" => ArgKind {
            helper: "take_bytes",
            meta: ParamTypeName::Bytes,
            union_members: None,
        },
        "*" | "any" => ArgKind {
            helper: "take_any",
            meta: ParamTypeName::Any,
            union_members: None,
        },
        "bool" => ArgKind {
            helper: "take_bool",
            meta: ParamTypeName::Bool,
            union_members: None,
        },
        "array" | "list" => ArgKind {
            helper: "take_array",
            meta: ParamTypeName::Array,
            union_members: None,
        },
        "table" => ArgKind {
            helper: "take_table",
            meta: ParamTypeName::Table,
            union_members: None,
        },
        "function" | "func" | "fn" => ArgKind {
            helper: "take_function",
            meta: ParamTypeName::Function,
            union_members: None,
        },
        "nil" => ArgKind {
            helper: "take_any",
            meta: ParamTypeName::Nil,
            union_members: None,
        },
        _ => {
            return Err(Error::new(
                span,
                "unsupported parameter type; use Vec<u8>/String, DukaInt/i64, DukaFloat/f64, bool, RuntimeValue or Gc<GcCell<RuntimeDukaTable>>",
            ));
        }
    })
}
fn arg_kind(ty: &Type) -> Result<ArgKind> {
    let name = last_seg_ident(ty).unwrap_or_default();
    match name.as_str() {
        "DukaInt" | "i64" => Ok(ArgKind {
            helper: "take_int",
            meta: ParamTypeName::Int,
            union_members: None,
        }),
        "DukaFloat" | "f64" => Ok(ArgKind {
            helper: "take_num",
            meta: ParamTypeName::Num,
            union_members: None,
        }),
        "bool" => Ok(ArgKind {
            helper: "take_bool",
            meta: ParamTypeName::Bool,
            union_members: None,
        }),
        "RuntimeValue" => Ok(ArgKind {
            helper: "take_any",
            meta: ParamTypeName::Any,
            union_members: None,
        }),
        "String" | "str" => Ok(ArgKind {
            helper: "take_string",
            meta: ParamTypeName::String,
            union_members: None,
        }),
        "Vec" => {
            let inner = single_generic(ty)
                .ok_or_else(|| Error::new_spanned(ty, "Vec requires a type argument"))?;
            match last_seg_ident(&inner).as_deref() {
                Some("u8") => Ok(ArgKind {
                    helper: "take_bytes",
                    meta: ParamTypeName::Bytes,
                    union_members: None,
                }),
                Some("RuntimeValue") => Ok(ArgKind {
                    helper: "take_many",
                    meta: ParamTypeName::VarArg,
                    union_members: None,
                }),
                _ => Err(Error::new_spanned(
                    ty,
                    "only Vec<u8> bytes Vec<RuntimeValue> are supported as a Vec<T> parameter",
                )),
            }
        }
        "Gc" => Ok(ArgKind {
            helper: "take_table",
            meta: ParamTypeName::Table,
            union_members: None,
        }),
        _ => Err(Error::new_spanned(
            ty,
            "unsupported parameter type; use Vec<u8>/String, DukaInt/i64, DukaFloat/f64, bool, RuntimeValue or Gc<GcCell<RuntimeDukaTable>>",
        )),
    }
}

struct RawParam {
    name: String,
    default: Option<TokenStream>,
    doc: Option<String>,
    ty: Option<String>,
    vararg: bool,
}
struct RawReturn {
    ty: String,
}

struct BuiltinConstArgs {
    module: String,
    name: String,
    doc: String,
    example: Option<String>,
    val: Option<String>,
    ty: ParamTypeName,
}
struct BuiltinArgs {
    module: String,
    name: Option<String>,
    doc: String,
    returns: Vec<RawReturn>,
    return_var_arg: bool,
    return_doc: String,
    example: Option<String>,
    params: Vec<RawParam>,
}

fn split_commas(ts: TokenStream) -> Vec<TokenStream> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    let mut depth = 0usize;
    for tt in ts {
        if let TokenTree::Group(g) = &tt {
            let d = g.delimiter();
            if matches!(
                d,
                Delimiter::Parenthesis | Delimiter::Bracket | Delimiter::Brace
            ) {
                depth += 1;
            }
            cur.push(tt);
            if matches!(
                d,
                Delimiter::Parenthesis | Delimiter::Bracket | Delimiter::Brace
            ) {
                depth -= 1;
            }
            continue;
        }
        if depth == 0 {
            if let TokenTree::Punct(p) = &tt {
                if p.as_char() == ',' {
                    out.push(cur.drain(..).collect());
                    continue;
                }
            }
        }
        cur.push(tt);
    }
    if !cur.is_empty() {
        out.push(cur.into_iter().collect());
    }
    out
}

fn parse_builtin_const_args(tokens: TokenStream) -> Result<BuiltinConstArgs> {
    let mut args = BuiltinConstArgs {
        module: String::new(),
        name: String::new(),
        doc: String::new(),
        example: None,
        ty: ParamTypeName::Any,
        val: None,
    };
    for seg in split_commas(tokens) {
        let mut toks: Vec<TokenTree> = seg.into_iter().collect();
        let Some(TokenTree::Ident(key)) = toks.first() else {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
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
            "module" => args.module = lit_str(&rest)?,
            "name" => args.name = lit_str(&rest)?,
            "doc" => args.doc = lit_str(&rest)?,
            "example" => args.example = Some(lit_str(&rest)?),
            "value" => args.val = Some(lit_str(&rest)?),
            _ => {
                return Err(Error::new_spanned(
                    &key,
                    format!("unknown duka_builtin attribute: {}", key),
                ));
            }
        }
    }
    if args.name.is_empty() {
        return Err(Error::new(
            proc_macro2::Span::call_site(),
            "duka_builtin requires `name`",
        ));
    }
    Ok(args)
}

fn parse_builtin_args(tokens: TokenStream) -> Result<BuiltinArgs> {
    let mut args = BuiltinArgs {
        module: String::new(),
        name: None,
        doc: String::new(),
        return_doc: String::new(),
        example: None,
        params: vec![],
        returns: vec![],
        return_var_arg: false,
    };
    let mut last_param: Option<usize> = None;
    for seg in split_commas(tokens) {
        let mut toks: Vec<TokenTree> = seg.into_iter().collect();
        let Some(TokenTree::Ident(key)) = toks.first() else {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
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
            "module" => args.module = lit_str(&rest)?,
            "name" => args.name = Some(lit_str(&rest)?),
            "doc" => args.doc = lit_str(&rest)?,
            "return_doc" => args.return_doc = lit_str(&rest)?,
            "example" => args.example = Some(lit_str(&rest)?),
            "returns" => {
                let inner = unwrap_paren(&rest)?;
                let (items, var_arg) = parse_returns(inner)?;
                for item in items {
                    args.returns.push(RawReturn { ty: item });
                }
                args.return_var_arg = var_arg;
            }
            "params" => {
                let inner = unwrap_paren(&rest)?;
                for item in parse_params(inner)? {
                    if item.0 == "doc" {
                        if let Some(i) = last_param {
                            args.params[i].doc = item.1.clone();
                        }
                        continue;
                    }
                    args.params.push(
                        if let Some(t) = &item.1
                            && t == "vararg"
                        {
                            RawParam {
                                name: item.0.clone(),
                                default: item.2,
                                doc: item.3,
                                ty: None,
                                vararg: true,
                            }
                        } else {
                            RawParam {
                                name: item.0.clone(),
                                default: item.2,
                                doc: item.3,
                                ty: item.1,
                                vararg: false,
                            }
                        },
                    );
                    last_param = Some(args.params.len() - 1);
                }
            }
            _ => {
                return Err(Error::new_spanned(
                    &key,
                    format!("unknown duka_builtin attribute: {}", key),
                ));
            }
        }
    }
    Ok(args)
}

fn lit_str(ts: &TokenStream) -> Result<String> {
    let lit: LitStr = syn::parse2(ts.clone())?;
    Ok(lit.value())
}

fn unwrap_paren(ts: &TokenStream) -> Result<TokenStream> {
    let mut toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    if toks.len() == 1 {
        if let TokenTree::Group(g) = toks.remove(0) {
            if g.delimiter() == Delimiter::Parenthesis {
                return Ok(g.stream());
            }
        }
    }
    Err(Error::new(
        proc_macro2::Span::call_site(),
        "expected parenthesized params(...) or returns(...)",
    ))
}

fn parse_returns(ts: TokenStream) -> Result<(Vec<String>, bool)> {
    let mut out = vec![];
    let mut var_arg = false;
    let tks = split_commas(ts);
    let len = tks.len();
    for (idx, seg) in tks.into_iter().enumerate() {
        let toks: Vec<TokenTree> = seg.into_iter().collect();
        let mut ty_chars = String::new();
        let mut i = 0usize;
        loop {
            let Some(tt) = toks.get(i) else { break };
            match tt {
                TokenTree::Ident(i) => ty_chars.push_str(&i.to_string()),
                TokenTree::Punct(p) => ty_chars.push(p.as_char()),
                TokenTree::Literal(_) => ty_chars.push_str(&tt.to_string()),
                TokenTree::Group(_) => ty_chars.push_str(&tt.to_string()),
            }
            i += 1;
        }
        let ty = if ty_chars.is_empty() {
            continue;
        } else if ty_chars == "vararg" && idx == len - 1 {
            var_arg = true;
            break;
        } else {
            ty_chars
        };
        out.push(ty);
    }
    Ok((out, var_arg))
}

fn parse_params(
    ts: TokenStream,
) -> Result<Vec<(String, Option<String>, Option<TokenStream>, Option<String>)>> {
    let mut out = Vec::new();
    let tks = split_commas(ts);
    let len = tks.len();
    for (idx, seg) in tks.into_iter().enumerate() {
        let toks: Vec<TokenTree> = seg.into_iter().collect();
        if toks.is_empty() {
            continue;
        }
        let first = match toks.first() {
            Some(TokenTree::Ident(i)) => i.to_string(),
            _ => {
                return Err(Error::new(
                    proc_macro2::Span::call_site(),
                    "invalid params entry",
                ));
            }
        };
        if first == "doc" && toks.get(1).map(is_eq).unwrap_or(false) {
            let val: LitStr = syn::parse2(toks[3..].to_vec().into_iter().collect())?;
            out.push(("doc".into(), Some(val.value()), None, None));
            continue;
        }
        if !toks.get(1).map(is_colon).unwrap_or(false) {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "expected `name : type ...` in params",
            ));
        }
        let mut ty_chars = String::new();
        let mut end_ty = 2usize;
        loop {
            let Some(tt) = toks.get(end_ty) else { break };
            if matches!(tt, TokenTree::Punct(p) if p.as_char() == '=') {
                break;
            }
            match tt {
                TokenTree::Ident(i) => ty_chars.push_str(&i.to_string()),
                TokenTree::Punct(p) => ty_chars.push(p.as_char()),
                TokenTree::Literal(_) => ty_chars.push_str(&tt.to_string()),
                TokenTree::Group(_) => ty_chars.push_str(&tt.to_string()),
            }
            end_ty += 1;
        }
        let ty: Option<String> = if ty_chars.is_empty() {
            None
        } else if ty_chars == "vararg" && idx == len - 1 {
            Some("vararg".to_owned())
        } else {
            Some(ty_chars)
        };
        let mut default: Option<TokenStream> = None;
        let doc: Option<String> = None;
        if toks.get(end_ty).map(is_eq).unwrap_or(false) {
            default = Some(toks[end_ty + 1..].to_vec().into_iter().collect());
        }
        out.push((first, ty, default, doc));
    }
    Ok(out)
}

fn is_eq(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Punct(p) if p.as_char() == '=')
}
fn is_colon(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Punct(p) if p.as_char() == ':')
}

fn format_ident(s: &str) -> Ident {
    Ident::new(s, proc_macro2::Span::call_site())
}

fn parse_type(s: &str) -> Type {
    syn::parse2(s.parse::<TokenStream>().unwrap()).unwrap()
}

fn mut_ref_arg(name: &str, ty: &Type) -> syn::PatType {
    syn::PatType {
        attrs: vec![],
        pat: Box::new(syn::Pat::Ident(PatIdent {
            attrs: vec![],
            by_ref: None,
            mutability: Some(syn::token::Mut(proc_macro2::Span::call_site())),
            ident: Ident::new(name, proc_macro2::Span::call_site()),
            subpat: None,
        })),
        colon_token: Default::default(),
        ty: Box::new(ty.clone()),
    }
}

fn meta_param_tokens(meta: &RawParam, kind: ArgKind) -> TokenStream {
    let vararg = meta.vararg;
    let name = LitStr::new(&meta.name, proc_macro2::Span::call_site());
    let ty = kind.meta.to_doc_type();
    let optional = meta.default.is_some();
    let default = match &meta.default {
        Some(d) => {
            let lit = LitStr::new(&d.to_string(), proc_macro2::Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };
    let doc = match &meta.doc {
        Some(d) => {
            let lit = LitStr::new(d, proc_macro2::Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };
    quote! {
        ::duka_shared::docs::ParamMeta {
            name: #name,
            ty: #ty,
            optional: #optional,
            default: #default,
            var_arg: #vararg,
            doc: #doc,
        }
    }
}
