// 公用attribute解析器

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use quote::quote;
use syn::{
    Error, FnArg, GenericArgument, Ident, LitStr, PatIdent, PathArguments, Result, ReturnType,
    Signature, Token, Type, TypePath, parenthesized, parse::Parse, punctuated::Punctuated,
};

use crate::crate_path::resolve_root_str;

#[derive(Clone)]
pub struct MetaInfoFlag {
    pub name: Ident,
    pub values: Punctuated<Ident, Token![,]>,
}
impl MetaInfoFlag {
    pub fn into_tokens(self) -> TokenStream {
        let name = self.name.to_string();
        let values = self
            .values
            .into_iter()
            .map(|v| LitStr::new(&v.to_string(), v.span()));
        quote! {
            (#name, &[#(#values),*])
        }
    }
}
impl Parse for MetaInfoFlag {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        input.parse::<Token![@]>()?;
        let name = input.parse()?;
        let content;
        parenthesized!(content in input);
        let values = content.parse_terminated(Ident::parse, Token![,])?;

        Ok(Self { name, values })
    }
}
#[derive(Default, Clone)]
pub struct MetaInfoFlags {
    pub flags: Punctuated<MetaInfoFlag, Token![,]>,
}
impl MetaInfoFlags {
    pub fn into_tokens(self) -> TokenStream {
        let flags = self.flags.into_iter().map(|f| f.into_tokens());
        quote! {
            &[#(#flags),*]
        }
    }
}
impl Parse for MetaInfoFlags {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        Ok(Self {
            flags: input.parse_terminated(MetaInfoFlag::parse, Token![,])?,
        })
    }
}

pub(crate) struct ArgReads {
    pub read_stmts: Vec<TokenStream>,
    pub call_args: Vec<TokenStream>,
    pub meta_params: Vec<TokenStream>,
    pub has_co: bool,
}

pub(crate) fn gen_arg_reads(
    user_name: &str,
    sig: &Signature,
    args: &BuiltinArgs,
    krate: &TokenStream,
    base: usize,
    self_ty: Option<&Ident>,
) -> Result<ArgReads> {
    let mut read_stmts: Vec<TokenStream> = vec![];
    let mut call_args: Vec<TokenStream> = vec![];
    let mut meta_params: Vec<TokenStream> = vec![];
    let mut param_i = 0usize;
    let mut read_idx = base;
    let mut has_co = false;

    for arg in &sig.inputs {
        match arg {
            FnArg::Receiver(recv) => {
                let meta = args.params.get(param_i).ok_or_else(|| {
                    Error::new_spanned(
                        recv,
                        "params(...) must declare the receiver as the first entry (`self: userdata`)",
                    )
                })?;
                let self_ty = self_ty.ok_or_else(|| {
                    Error::new_spanned(recv, "a function receiver is not supported in duka_builtin")
                })?;
                if !meta.is_userdata {
                    return Err(Error::new_spanned(
                        recv,
                        "the receiver must be declared as `self: userdata` in params(...)",
                    ));
                }
                param_i += 1;
                let udname = meta
                    .userdata_name
                    .clone()
                    .unwrap_or_else(|| self_ty.to_string());
                let mut meta = (*meta).clone();
                if meta.doc.is_none() {
                    meta.doc = Some(udname);
                }
                let kind = ArgKind {
                    helper: "",
                    meta: ParamTypeName::UserData,
                    union_members: None,
                };
                if recv.reference.is_none() {
                    return Err(Error::new_spanned(
                        recv,
                        "the receiver must be `&self` or `&mut self`",
                    ));
                }
                let is_mut = recv.mutability.is_some();
                let (borrow, any, downcast, bound_ty) = if is_mut {
                    (
                        quote! { let mut __duka_borrow = __duka_cell.borrow_mut(); },
                        quote! { (__duka_borrow.payload.as_mut() as &mut dyn std::any::Any) },
                        quote! { downcast_mut::<#self_ty>() },
                        quote! { &mut #self_ty },
                    )
                } else {
                    (
                        quote! { let __duka_borrow = __duka_cell.borrow(); },
                        quote! { (__duka_borrow.payload.as_ref() as &dyn std::any::Any) },
                        quote! { downcast_ref::<#self_ty>() },
                        quote! { &#self_ty },
                    )
                };
                let name =
                    LitStr::new(args.name.as_deref().unwrap_or(user_name), Span::call_site());
                let self_name = self_ty.to_string();
                read_stmts.push(quote! {
                    let #krate::value::RuntimeValue::UserData(__duka_cell) = sv.take_stack(1).map_err(|_| DukaRuntimeError::ArgumentMissing(0, #name.to_owned(), "receiver".to_owned()))? else {
                        return Err(
                        #krate::errors::DukaRuntimeError::ArgumentInvalidType(0, #name.to_owned(), #self_name, "other"));
                    };
                    #borrow
                    let __duka_self: #bound_ty = #any.#downcast.ok_or_else(|| {
                        #krate::errors::DukaRuntimeError::ArgumentInvalidType(0, #name.to_owned(), #self_name, "other")
                    })?;
                });
                call_args.push(quote! { __duka_self });
                meta_params.push(meta_param_tokens(&meta, kind, krate));
            }
            FnArg::Typed(pt) => {
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
                if meta.is_userdata {
                    return Err(Error::new_spanned(
                        ty,
                        "only the method receiver can be a `userdata` parameter for now",
                    ));
                }

                let kind = meta
                    .ty
                    .as_ref()
                    .map(|t| ty_to_kind(t, Span::call_site()))
                    .unwrap_or_else(|| arg_kind(ty))?;

                let idx = proc_macro2::Literal::usize_unsuffixed(read_idx);
                read_idx += 1;
                param_i += 1;
                let name_lit = LitStr::new(&meta.name, Span::call_site());
                let helper = Ident::new(kind.helper, Span::call_site());
                let args = if let Some(members) = &kind.union_members {
                    let ctypes: Vec<TokenStream> = members
                        .iter()
                        .map(|m| {
                            let m = Ident::new(m, Span::call_site());
                            quote! { #krate::duka_shared::constants::ctype::#m }
                        })
                        .collect();
                    let want = LitStr::new(&meta.ty.clone().unwrap_or_default(), Span::call_site());
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
                meta_params.push(meta_param_tokens(meta, kind, krate));
            }
        }
    }

    if param_i != args.params.len() {
        return Err(Error::new_spanned(
            sig,
            "params(...) count must match the number of non-injected arguments",
        ));
    }

    Ok(ArgReads {
        read_stmts,
        call_args,
        meta_params,
        has_co,
    })
}

pub(crate) fn gen_meta(
    user_name: &str,
    meta_ident: &Ident,
    args: &BuiltinArgs,
    meta_params: &[TokenStream],
    meta_returns: &[TokenStream],
    krate: &TokenStream,
) -> TokenStream {
    let meta_ty = parse_type(&format!("{}::duka_shared::docs::MetaInfo", krate));
    let name = LitStr::new(args.name.as_deref().unwrap_or(user_name), Span::call_site());
    let doc = LitStr::new(&args.doc, Span::call_site());
    let ret_text = LitStr::new(&args.return_doc, Span::call_site());
    let example = match &args.example {
        Some(e) => {
            let lit = LitStr::new(e, Span::call_site());
            quote! { Some(#lit) }
        }
        None => quote! { None },
    };
    let ret_var_arg = args.return_var_arg;
    let flags = args.flags.clone().into_tokens();
    quote! {
        #[cfg(feature = "docs")]
        #[doc(hidden)]
        #[allow(dead_code)]
        pub const #meta_ident: #meta_ty = #krate::duka_shared::docs::MetaInfo {
            name: #name,
            doc: #doc,
            info: #krate::duka_shared::docs::MetaItemInfo::Function {
                returns: #krate::duka_shared::docs::ReturnMeta {
                    text: #ret_text,
                    tys: &[#(#meta_returns),*],
                    var_arg: #ret_var_arg,
                },
                params: &[#(#meta_params),*],
            },
            example: #example,
            flags: #flags
        };
    }
}

pub(crate) fn gen_return(kind: &ReturnKind, krate: &TokenStream) -> Result<TokenStream> {
    let vc = quote! { #krate::duka_shared::types::ValueCount };
    Ok(match kind {
        ReturnKind::Zero => quote! { Ok(#vc::Exact(0)) },
        ReturnKind::One => quote! { sv.set_stack(0, __ret)?; Ok(#vc::Exact(1)) },
        ReturnKind::Dynamic => {
            quote! { sv.set_stack_many(0, &__ret)?; Ok(#vc::Exact(__ret.len())) }
        }
        ReturnKind::Many(tys) => {
            let n = tys.len();
            let n_lit = proc_macro2::Literal::usize_unsuffixed(n);
            let pats: Vec<Ident> = (0..n).map(|i| str2ident(&format!("__e{}", i))).collect();
            let mut sets = vec![];
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

pub(crate) enum ReturnKind {
    Zero,
    One,
    Dynamic,
    Many(Vec<Type>),
}

pub(crate) fn classify_return(output: &ReturnType) -> Result<ReturnKind> {
    let ty = match output {
        ReturnType::Type(_, ty) => ty.as_ref(),
        ReturnType::Default => {
            return Err(Error::new(
                Span::call_site(),
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
    let mut tys = vec![];
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

pub(crate) fn conv_expr(ty: &Type, bind: &Ident, krate: &TokenStream) -> Result<TokenStream> {
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
pub(crate) enum ParamTypeName {
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
    UserData,
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
            ParamTypeName::UserData => "Any",
            _ => panic!("Type is not supported here"),
        }
    }

    pub(crate) fn to_doc_type(&self) -> TokenStream {
        let base = format!("{}::duka_shared::docs::DocType", resolve_root_str());
        let s = match self {
            ParamTypeName::PreserveNumber => format!("{base}::PreserveNumber"),
            ParamTypeName::Bytes => format!("{base}::Bytes"),
            ParamTypeName::Function => {
                format!(
                    "{base}::Base({}::duka_shared::dtype::Type::Function(None))",
                    resolve_root_str()
                )
            }
            ParamTypeName::VarArg => {
                format!(
                    "{base}::Base({}::duka_shared::dtype::Type::Any)",
                    resolve_root_str()
                )
            }
            ParamTypeName::Union(items) => {
                let inner = items
                    .iter()
                    .map(|i| i.to_doc_type().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{base}::Union(&[{inner}])")
            }
            _ => {
                let base = format!(
                    "{base}::Base({}::duka_shared::dtype::Type::{}",
                    resolve_root_str(),
                    self.get_type_variant()
                );
                match self {
                    ParamTypeName::Table => format!("{base}(None, None))"),
                    ParamTypeName::Array => format!("{base}(None))"),
                    _ => format!("{base})"),
                }
            }
        };
        s.parse::<TokenStream>().unwrap()
    }

    /// to Type enum
    pub(crate) fn to_type(&self) -> TokenStream {
        let root = resolve_root_str();
        let s = match self {
            ParamTypeName::Table => {
                format!("{root}::duka_shared::dtype::Type::Table(None, None)")
            }
            ParamTypeName::Array => {
                format!("{root}::duka_shared::dtype::Type::Array(None)")
            }
            _ => format!(
                "{root}::duka_shared::dtype::Type::{}",
                self.get_type_variant()
            ),
        };
        s.parse::<TokenStream>().unwrap()
    }
}

pub(crate) struct ArgKind {
    pub(crate) helper: &'static str,
    pub(crate) meta: ParamTypeName,
    pub(crate) union_members: Option<Vec<&'static str>>,
}

fn last_seg_ident(ty: &Type) -> Option<String> {
    if let Type::Path(TypePath { path, .. }) = ty {
        return path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

fn single_generic(ty: &Type) -> Option<Type> {
    if let Type::Path(TypePath { path, .. }) = ty
        && let PathArguments::AngleBracketed(ab) = &path.segments.last()?.arguments
    {
        for a in &ab.args {
            if let GenericArgument::Type(t) = a {
                return Some(t.clone());
            }
        }
    }
    None
}

fn is_ref_ident(ty: &Type, ident: &str) -> bool {
    if let Type::Reference(r) = ty
        && let Type::Path(p) = &*r.elem
    {
        return p
            .path
            .segments
            .last()
            .map(|s| s.ident == ident)
            .unwrap_or(false);
    }
    false
}

pub(crate) fn ty_to_kind(ty: &str, span: Span) -> Result<ArgKind> {
    if ty.contains('|') {
        let mut inner = vec![];
        let mut kinds = vec![];
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

pub(crate) fn simple_kind(ty: &str, span: Span) -> Result<ArgKind> {
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

pub(crate) fn arg_kind(ty: &Type) -> Result<ArgKind> {
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

#[derive(Clone)]
pub(crate) struct RawParam {
    pub name: String,
    pub default: Option<TokenStream>,
    pub default_display: Option<String>,
    pub doc: Option<String>,
    pub ty: Option<String>,
    pub vararg: bool,
    pub is_userdata: bool,
    pub userdata_name: Option<String>,
}
pub(crate) struct RawReturn {
    pub ty: String,
}

pub(crate) struct BuiltinConstArgs {
    pub name: String,
    pub doc: String,
    pub example: Option<String>,
    pub val: Option<String>,
    pub ty: ParamTypeName,
    pub flags: MetaInfoFlags,
}
pub(crate) struct BuiltinArgs {
    pub name: Option<String>,
    pub doc: String,
    pub returns: Vec<RawReturn>,
    pub return_var_arg: bool,
    pub return_doc: String,
    pub example: Option<String>,
    pub params: Vec<RawParam>,
    pub flags: MetaInfoFlags,
}

pub(crate) fn split_commas(ts: TokenStream) -> Vec<TokenStream> {
    let mut out = vec![];
    let mut cur = vec![];
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
        if depth == 0
            && let TokenTree::Punct(p) = &tt
            && p.as_char() == ','
        {
            out.push(cur.drain(..).collect());
            continue;
        }
        cur.push(tt);
    }
    if !cur.is_empty() {
        out.push(cur.into_iter().collect());
    }
    out
}

pub(crate) fn parse_builtin_const_args(tokens: TokenStream) -> Result<BuiltinConstArgs> {
    let mut args = BuiltinConstArgs {
        name: String::new(),
        doc: String::new(),
        example: None,
        ty: ParamTypeName::Any,
        val: None,
        flags: MetaInfoFlags::default(),
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
            "type" => args.ty = simple_kind(&lit_str(&rest)?, Span::call_site())?.meta,
            "name" => args.name = lit_str(&rest)?,
            "doc" => args.doc = lit_str(&rest)?,
            "example" => args.example = Some(lit_str(&rest)?),
            "value" => args.val = Some(lit_str(&rest)?),
            "flags" => {
                let inner = unwrap_paren(&rest)?;
                args.flags = syn::parse2::<MetaInfoFlags>(inner)?;
            }
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
            Span::call_site(),
            "duka_builtin requires `name`",
        ));
    }
    Ok(args)
}

pub(crate) fn parse_builtin_args(tokens: TokenStream) -> Result<BuiltinArgs> {
    let mut args = BuiltinArgs {
        name: None,
        doc: String::new(),
        return_doc: String::new(),
        example: None,
        params: vec![],
        returns: vec![],
        return_var_arg: false,
        flags: MetaInfoFlags::default(),
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
                for p in parse_params(inner)? {
                    args.params.push(p);
                }
            }
            "flags" => {
                let inner = unwrap_paren(&rest)?;
                args.flags = syn::parse2::<MetaInfoFlags>(inner)?;
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

pub(crate) fn lit_str(ts: &TokenStream) -> Result<String> {
    let lit: LitStr = syn::parse2(ts.clone())?;
    Ok(lit.value())
}

fn unwrap_paren(ts: &TokenStream) -> Result<TokenStream> {
    let mut toks: Vec<TokenTree> = ts.clone().into_iter().collect();
    if toks.len() == 1
        && let TokenTree::Group(g) = toks.remove(0)
        && g.delimiter() == Delimiter::Parenthesis
    {
        Ok(g.stream())
    } else {
        Err(Error::new(
            Span::call_site(),
            "expected parenthesized params(...) or returns(...)",
        ))
    }
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
        while let Some(tt) = toks.get(i) {
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

fn parse_params(ts: TokenStream) -> Result<Vec<RawParam>> {
    let mut out: Vec<RawParam> = vec![];
    let tks = split_commas(ts);
    let len = tks.len();
    let mut last_param: Option<usize> = None;
    for (idx, seg) in tks.into_iter().enumerate() {
        let toks: Vec<TokenTree> = seg.into_iter().collect();
        if toks.is_empty() {
            continue;
        }
        let first = match toks.first() {
            Some(TokenTree::Ident(i)) => i.to_string(),
            Some(TokenTree::Punct(p)) if p.as_char() == '@' => {
                let Some(TokenTree::Ident(i)) = toks.get(1) else {
                    return Err(Error::new(
                        Span::call_site(),
                        "invalid annotation in params",
                    ));
                };
                let toks = toks[3..].to_vec();
                let val: LitStr = syn::parse2(toks.into_iter().collect())?;
                match i.to_string().as_str() {
                    "default" => {
                        if let Some(i) = last_param {
                            out[i].default_display = Some(val.value());
                        }
                    }
                    "doc" => {
                        if let Some(i) = last_param {
                            out[i].doc = Some(val.value());
                        }
                    }
                    _ => {
                        return Err(Error::new(
                            Span::call_site(),
                            "unknown annotation in params",
                        ));
                    }
                }
                continue;
            }
            _ => {
                return Err(Error::new(Span::call_site(), "invalid params entry"));
            }
        };

        if !toks.get(1).map(is_colon).unwrap_or(false) {
            return Err(Error::new(
                Span::call_site(),
                "expected `name : type ...` in params",
            ));
        }
        let mut ty_toks: Vec<TokenTree> = vec![];
        let mut end_ty = 2usize;
        while let Some(tt) = toks.get(end_ty) {
            if matches!(tt, TokenTree::Punct(p) if p.as_char() == '=') {
                break;
            }
            ty_toks.push(tt.clone());
            end_ty += 1;
        }
        let mut default: Option<TokenStream> = None;
        if toks.get(end_ty).map(is_eq).unwrap_or(false) {
            default = Some(toks[end_ty + 1..].iter().cloned().collect());
        }
        let is_userdata = matches!(
            ty_toks.first(),
            Some(TokenTree::Ident(i)) if i == "userdata"
        );
        let mut userdata_name: Option<String> = None;
        let mut ty_chars = String::new();
        for tt in &ty_toks {
            match tt {
                TokenTree::Ident(i) => ty_chars.push_str(&i.to_string()),
                TokenTree::Punct(p) => ty_chars.push(p.as_char()),
                TokenTree::Literal(_) => ty_chars.push_str(&tt.to_string()),
                TokenTree::Group(_) => ty_chars.push_str(&tt.to_string()),
            }
        }
        let vararg = idx == len - 1 && ty_chars == "vararg";
        let ty: Option<String> = if ty_chars.is_empty() {
            None
        } else if is_userdata {
            if let Some(TokenTree::Group(g)) = ty_toks.get(1)
                && g.delimiter() == Delimiter::Parenthesis
            {
                let lit: LitStr = syn::parse2(g.stream())?;
                userdata_name = Some(lit.value());
            }
            Some("userdata".to_owned())
        } else if vararg {
            None
        } else {
            Some(ty_chars)
        };
        out.push(RawParam {
            name: first,
            default,
            default_display: None,
            doc: None,
            vararg,
            ty,
            is_userdata,
            userdata_name,
        });
        last_param = Some(out.len() - 1);
    }
    Ok(out)
}

pub(crate) fn is_eq(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Punct(p) if p.as_char() == '=')
}
fn is_colon(tt: &TokenTree) -> bool {
    matches!(tt, TokenTree::Punct(p) if p.as_char() == ':')
}

pub(crate) fn str2ident(s: &str) -> Ident {
    Ident::new(s, Span::call_site())
}

pub(crate) fn parse_type(s: &str) -> Type {
    syn::parse2(s.parse::<TokenStream>().unwrap()).unwrap()
}

pub(crate) fn mut_ref_arg(name: &str, ty: &Type) -> syn::PatType {
    syn::PatType {
        attrs: vec![],
        pat: Box::new(syn::Pat::Ident(PatIdent {
            attrs: vec![],
            by_ref: None,
            mutability: Some(syn::token::Mut(Span::call_site())),
            ident: Ident::new(name, Span::call_site()),
            subpat: None,
        })),
        colon_token: Default::default(),
        ty: Box::new(ty.clone()),
    }
}

pub(crate) fn meta_param_tokens(
    meta: &RawParam,
    kind: ArgKind,
    krate: &TokenStream,
) -> TokenStream {
    let vararg = meta.vararg;
    let name = LitStr::new(&meta.name, Span::call_site());
    let ty = kind.meta.to_doc_type();
    let optional = meta.default.is_some();
    let default = match (&meta.default, &meta.default_display) {
        (Some(_), Some(d)) => {
            quote! { Some(#d) }
        }
        (Some(d), _) => {
            let lit = d.to_string();
            quote! { Some(#lit) }
        }
        (None, _) => quote! { None },
    };
    let doc = match &meta.doc {
        Some(d) => {
            quote! { Some(#d) }
        }
        None => quote! { None },
    };
    quote! {
        #krate::duka_shared::docs::ParamMeta {
            name: #name,
            ty: #ty,
            optional: #optional,
            default: #default,
            var_arg: #vararg,
            doc: #doc,
        }
    }
}
