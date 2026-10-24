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
                    want: None,
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
                meta_params.push(meta_param_tokens(&meta, kind, krate)?);
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
                    .map(|t| ty_to_kind_bounded(t, &args.type_params, Span::call_site()))
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
                    let want = kind
                        .want
                        .clone()
                        .or_else(|| meta.ty.clone())
                        .unwrap_or_default();
                    let want = LitStr::new(&want, Span::call_site());
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
                meta_params.push(meta_param_tokens(meta, kind, krate)?);
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
) -> Result<TokenStream> {
    let meta_ty = parse_type(&format!("{}::duka_shared::docs::MetaInfo", krate))?;
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
    let mut meta_vars = vec![];
    for var in &args.type_params {
        let name = LitStr::new(&var.name, Span::call_site());
        let literal = var.literal;
        let pack = var.pack;
        // a bound and a default are types of the language, not of Rust: they
        // are read by the type parser the signature uses and reach the
        // metadata as the documentation's own type
        let bound = match &var.bound {
            None => quote! { None },
            Some(text) => {
                let ty = doc_type_of(text, Span::call_site(), "type parameter bound")?;
                quote! { Some(#ty) }
            }
        };
        let default = match &var.default {
            None => quote! { None },
            Some(text) => {
                let ty = doc_type_of(text, Span::call_site(), "type parameter default")?;
                quote! { Some(#ty) }
            }
        };
        meta_vars.push(quote! {
            #krate::duka_shared::docs::VarMeta {
                name: #name,
                bound: #bound,
                default: #default,
                literal: #literal,
                pack: #pack,
            }
        });
    }
    // the `where` list and the constraints are what a call has to discharge and
    // what its values have to satisfy
    let meta_where = args
        .where_clauses
        .iter()
        .map(where_clause_tokens)
        .collect::<Result<Vec<_>>>()?;
    let meta_constraints = args
        .constraints
        .iter()
        .map(constraint_tokens)
        .collect::<Result<Vec<_>>>()?;
    Ok(quote! {
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
                type_params: &[#(#meta_vars),*],
                where_clauses: &[#(#meta_where),*],
                constraints: &[#(#meta_constraints),*],
            },
            example: #example,
            flags: #flags
        };
    })
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
    let Some(seg) = path.segments.last() else {
        return Err(Error::new_spanned(
            ty,
            "duka_builtin functions must return Result<T, E>",
        ));
    };
    if seg.ident != "Result" {
        return Err(Error::new_spanned(
            ty,
            "duka_builtin functions must return Result<T, E>",
        ));
    }
    let PathArguments::AngleBracketed(ab) = &seg.arguments else {
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
                return Err(Error::new_spanned(
                    ty,
                    "Vec<RuntimeValue> cannot be a member of a tuple return; return `Vec<RuntimeValue>` directly instead",
                ));
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
    Table(Option<Box<ParamTypeName>>, Option<Box<ParamTypeName>>),
    Function(Option<Box<FnSignature>>),
    Array(Option<Box<ParamTypeName>>),
    Any,
    Nil,
    Bytes,
    VarArg,
    UserData,
    /// A userdata type, named: `userdata(File)`
    UserDataType(String),
    /// A type the language declares, named: `type(CoStatus)`
    ///
    /// A bare name is a variable of the signature, so a type that is not one
    /// has to say so. A name applied to arguments is a type function call, the
    /// way a type position reads it, and needs no form of its own.
    NamedType(String, Vec<ParamTypeName>),
    /// A coroutine handle
    Coroutine,
    /// A literal type: `"name"`, `42`, `true`
    Literal(LiteralKind),
    Union(Vec<ParamTypeName>),
    /// A bare identifier that names one of the signature's own type parameters
    Param(String),
}

/// The value a literal type names
#[derive(Debug)]
pub(crate) enum LiteralKind {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Debug)]
pub(crate) struct FnSignature {
    pub(crate) params: Vec<ParamTypeName>,
    pub(crate) returns: Vec<ParamTypeName>,
    /// The variables the signature declares, the `T` of `fn<T>(T) -> T`
    pub(crate) type_params: Vec<VarDecl>,
}

impl ParamTypeName {
    fn helper(&self) -> &'static str {
        match self {
            ParamTypeName::Int => "take_int",
            ParamTypeName::Num => "take_num",
            ParamTypeName::String => "take_string",
            ParamTypeName::Bytes => "take_bytes",
            ParamTypeName::Bool => "take_bool",
            ParamTypeName::Array(..) => "take_array",
            ParamTypeName::Table(..) => "take_table",
            ParamTypeName::Function(..) => "take_function",
            ParamTypeName::Nil | ParamTypeName::Any | ParamTypeName::UserData => "take_any",
            // A userdata is a value like any other; what it is is the type's
            // business, not the reading's.
            ParamTypeName::UserDataType(..) => "take_any",
            // A named type is a value like any other too: the name says what
            // it is, and the scope that knows the name is not this one.
            ParamTypeName::NamedType(..) => "take_any",
            // A coroutine is a value like any other too: the runtime checks
            // what it is where it is used.
            ParamTypeName::Coroutine => "take_any",
            // A literal type says which value is wanted, and the type checker
            // is the one that insists on it. The reader has nothing to read:
            // what arrives is a value like any other.
            ParamTypeName::Literal(..) => "take_any",
            ParamTypeName::VarArg => "take_many",
            ParamTypeName::Union(..) => "take_union",
            // A variable is filled in by the caller, so what arrives is whatever
            // it was given: the runtime reads it the way it reads `any`.
            ParamTypeName::Param(..) => "take_any",
        }
    }

    fn outer_ctype(&self) -> &'static str {
        match self {
            ParamTypeName::Int => "INT",
            ParamTypeName::Num => "NUM",
            ParamTypeName::String | ParamTypeName::Bytes => "STR",
            ParamTypeName::Bool => "BOO",
            ParamTypeName::Table(..) => "TAB",
            ParamTypeName::Array(..) => "ARR",
            ParamTypeName::Function(..) => "FUN",
            ParamTypeName::Nil => "NIL",
            ParamTypeName::Coroutine => "COR",
            ParamTypeName::Literal(..) => "ANY",
            _ => "ANY",
        }
    }

    pub(crate) fn name(&self) -> String {
        match self {
            ParamTypeName::String => "string".to_owned(),
            ParamTypeName::Int => "int".to_owned(),
            ParamTypeName::Num => "num".to_owned(),
            ParamTypeName::Bool => "bool".to_owned(),
            ParamTypeName::Param(name) => name.clone(),
            ParamTypeName::Table(None, None) => "table".to_owned(),
            ParamTypeName::Table(k, v) => {
                let k = k
                    .as_deref()
                    .map(ParamTypeName::name)
                    .unwrap_or_else(|| "any".to_owned());
                let v = v
                    .as_deref()
                    .map(ParamTypeName::name)
                    .unwrap_or_else(|| "any".to_owned());
                format!("table<{k}, {v}>")
            }
            ParamTypeName::Function(None) => "fn".to_owned(),
            ParamTypeName::Function(Some(sig)) => {
                let params = sig
                    .params
                    .iter()
                    .map(ParamTypeName::name)
                    .collect::<Vec<_>>()
                    .join(", ");
                // the variables are written the way a signature writes them
                let vars = if sig.type_params.is_empty() {
                    String::new()
                } else {
                    let vars = sig
                        .type_params
                        .iter()
                        .map(|v| {
                            if v.pack {
                                format!("...{}", v.name)
                            } else {
                                v.name.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("<{vars}>")
                };
                if sig.returns.is_empty() {
                    format!("fn{vars}({params})")
                } else {
                    let returns = sig
                        .returns
                        .iter()
                        .map(ParamTypeName::name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("fn{vars}({params}) -> {returns}")
                }
            }
            ParamTypeName::Array(None) => "array".to_owned(),
            ParamTypeName::Array(Some(inner)) => format!("array<{}>", inner.name()),
            ParamTypeName::Any => "any".to_owned(),
            ParamTypeName::Nil => "nil".to_owned(),
            ParamTypeName::Bytes => "bytes".to_owned(),
            ParamTypeName::VarArg => "vararg".to_owned(),
            ParamTypeName::UserData => "userdata".to_owned(),
            ParamTypeName::UserDataType(name) => name.clone(),
            ParamTypeName::NamedType(name, args) => {
                let args = args
                    .iter()
                    .map(ParamTypeName::name)
                    .collect::<Vec<_>>()
                    .join(", ");
                if args.is_empty() {
                    name.clone()
                } else {
                    format!("{name}({args})")
                }
            }
            ParamTypeName::Coroutine => "coroutine".to_owned(),
            ParamTypeName::Literal(LiteralKind::Str(s)) => format!("{s:?}"),
            ParamTypeName::Literal(LiteralKind::Int(n)) => n.to_string(),
            ParamTypeName::Literal(LiteralKind::Bool(b)) => b.to_string(),
            ParamTypeName::Literal(LiteralKind::Float(f)) => f.to_string(),
            ParamTypeName::Union(items) => items
                .iter()
                .map(ParamTypeName::name)
                .collect::<Vec<_>>()
                .join(" | "),
        }
    }

    pub(crate) fn to_doc_type(&self) -> Result<TokenStream> {
        let s = self.to_doc_type_str()?;
        parse_str_tokens(&s, "doc type")
    }

    fn to_doc_type_str(&self) -> Result<String> {
        let root = resolve_root_str();
        let base = format!("{root}::duka_shared::docs::DocType");
        let ty = format!("{root}::duka_shared::dtype::Type");
        Ok(match self {
            ParamTypeName::Bytes => format!("{base}::Bytes"),
            ParamTypeName::String => format!("{base}::Base({ty}::String)"),
            ParamTypeName::Int => format!("{base}::Base({ty}::Int)"),
            ParamTypeName::Num => format!("{base}::Base({ty}::Float)"),
            ParamTypeName::Bool => format!("{base}::Base({ty}::Bool)"),
            ParamTypeName::Param(name) => format!("{base}::Param({name:?})"),
            ParamTypeName::UserDataType(name) => format!("{base}::UserData({name:?})"),
            ParamTypeName::NamedType(name, args) => {
                let args = args
                    .iter()
                    .map(|a| a.to_doc_type_str())
                    .collect::<Result<Vec<_>>>()?;
                format!("{base}::Named({name:?}, &[{}])", args.join(", "))
            }
            ParamTypeName::Coroutine => format!("{base}::Base({ty}::Coroutine)"),
            ParamTypeName::UserData => format!("{base}::UserData(\"\")"),
            ParamTypeName::Any => format!("{base}::Base({ty}::Any)"),
            ParamTypeName::Nil => format!("{base}::Base({ty}::Nil)"),
            ParamTypeName::Literal(LiteralKind::Float(f)) => format!(
                "{base}::Base({ty}::Literal({root}::duka_shared::value::ConstValue::Float({f})))"
            ),
            ParamTypeName::Literal(LiteralKind::Str(s)) => format!("{base}::StringLiteral({s:?})"),
            ParamTypeName::Literal(LiteralKind::Int(n)) => format!(
                "{base}::Base({ty}::Literal({root}::duka_shared::value::ConstValue::Int({n})))"
            ),
            ParamTypeName::Literal(LiteralKind::Bool(b)) => format!(
                "{base}::Base({ty}::Literal({root}::duka_shared::value::ConstValue::Bool({b})))"
            ),
            ParamTypeName::VarArg => format!("{base}::Base({ty}::Any)"),
            ParamTypeName::Array(None) => format!("{base}::Base({ty}::Array(None))"),
            ParamTypeName::Array(Some(inner)) => {
                format!("{base}::Array(&{})", inner.to_doc_type_str()?)
            }
            ParamTypeName::Table(None, None) => format!("{base}::Base({ty}::Table(None, None))"),
            ParamTypeName::Table(k, v) => format!(
                "{base}::Table({}, {})",
                opt_doc_type_str(k)?,
                opt_doc_type_str(v)?
            ),
            ParamTypeName::Function(None) => format!("{base}::Base({ty}::Function(None))"),
            ParamTypeName::Function(Some(sig)) => {
                let render = |list: &[ParamTypeName]| -> Result<String> {
                    let items = list
                        .iter()
                        .map(|p| match p {
                            ParamTypeName::VarArg => Ok(format!("{base}::VarArg")),
                            other => other.to_doc_type_str(),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(items.join(", "))
                };
                // the variables a signature declares travel with it
                let vars = sig
                    .type_params
                    .iter()
                    .map(|v| {
                        let name = format!("{:?}", v.name);
                        let bound = match &v.bound {
                            None => "None".to_owned(),
                            Some(text) => {
                                let ty = parse_type_name(
                                    text,
                                    Span::call_site(),
                                    "type parameter bound",
                                    PARAM_TYPE_HINT,
                                )?
                                .to_doc_type_str()?;
                                format!("Some(&{ty})")
                            }
                        };
                        let default = match &v.default {
                            None => "None".to_owned(),
                            Some(text) => {
                                let ty = parse_type_name(
                                    text,
                                    Span::call_site(),
                                    "type parameter default",
                                    PARAM_TYPE_HINT,
                                )?
                                .to_doc_type_str()?;
                                format!("Some(&{ty})")
                            }
                        };
                        Ok(format!(
                            "{root}::duka_shared::docs::VarMeta {{ name: {name}, bound: {bound}, default: {default}, literal: {}, pack: {} }}",
                            v.literal, v.pack
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                format!(
                    "{base}::Function(&[{}], &[{}], &[{}])",
                    render(&sig.params)?,
                    render(&sig.returns)?,
                    vars.join(", ")
                )
            }
            ParamTypeName::Union(items) => {
                let inner = items
                    .iter()
                    .map(|i| i.to_doc_type_str())
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                format!("{base}::Union(&[{inner}])")
            }
        })
    }
}

fn opt_doc_type_str(ty: &Option<Box<ParamTypeName>>) -> Result<String> {
    Ok(match ty {
        Some(ty) => format!("Some(&{})", ty.to_doc_type_str()?),
        None => "None".to_owned(),
    })
}

impl ParamTypeName {
    /// `T?` -- the type with nil added to it, the way the language reads a
    /// question mark on a type.
    fn nilable(self) -> ParamTypeName {
        match self {
            // `any` takes nil already, and the language does not write a union
            // around it when it does not have to
            ParamTypeName::Any => ParamTypeName::Any,
            ParamTypeName::Union(mut items) => {
                if !items.iter().any(|i| matches!(i, ParamTypeName::Nil)) {
                    items.push(ParamTypeName::Nil);
                }
                ParamTypeName::Union(items)
            }
            other => ParamTypeName::Union(vec![other, ParamTypeName::Nil]),
        }
    }

    /// `T!` -- the type with nil taken out of it. A type that was nothing but
    /// nil is `any`, which is what the language reads `nil!` as.
    fn nonnilable(self) -> ParamTypeName {
        match self {
            ParamTypeName::Nil => ParamTypeName::Any,
            ParamTypeName::Union(items) => {
                let mut kept = items
                    .into_iter()
                    .filter_map(ParamTypeName::without_nil)
                    .collect::<Vec<_>>();
                match kept.len() {
                    0 => ParamTypeName::Any,
                    1 => kept.swap_remove(0),
                    _ => ParamTypeName::Union(kept),
                }
            }
            other => other,
        }
    }

    /// A union member with nil taken out of it, `None` when the member was nil
    /// itself.
    fn without_nil(self) -> Option<ParamTypeName> {
        match self {
            ParamTypeName::Nil => None,
            ParamTypeName::Union(items) => {
                let mut kept = items
                    .into_iter()
                    .filter_map(ParamTypeName::without_nil)
                    .collect::<Vec<_>>();
                match kept.len() {
                    0 => None,
                    1 => Some(kept.swap_remove(0)),
                    _ => Some(ParamTypeName::Union(kept)),
                }
            }
            other => Some(other),
        }
    }
}

pub(crate) struct ArgKind {
    pub(crate) helper: &'static str,
    pub(crate) meta: ParamTypeName,
    pub(crate) union_members: Option<Vec<&'static str>>,
    /// What the reader says it wanted, when that is not the type the
    /// parameter declares
    pub(crate) want: Option<String>,
}

impl ArgKind {
    fn plain(helper: &'static str, meta: ParamTypeName) -> Self {
        ArgKind {
            helper,
            meta,
            union_members: None,
            want: None,
        }
    }
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
    let meta = parse_type_name(ty, span, "parameter", PARAM_TYPE_HINT)?;
    if let ParamTypeName::Union(inner) = &meta {
        let members = inner.iter().map(|i| i.outer_ctype()).collect();
        return Ok(ArgKind {
            helper: "take_union",
            meta,
            union_members: Some(members),
            want: None,
        });
    }
    let helper = meta.helper();
    Ok(ArgKind::plain(helper, meta))
}

/// The reader a parameter needs, when the type it declares is a variable the
/// signature gives a bound to.
///
/// The bound says what the value has to be. A bound that names no variable
/// says it exactly, so the reader can be the bound's own: a value of a type
/// variable is read as `any` otherwise, and the bound is what says that what
/// arrives is a number.
///
/// A bound that names a variable says only that the value is one of several
/// things, and which one is not known until the call says, so the value is
/// read as `any` there.
fn ty_to_kind_bounded(ty: &str, type_params: &[VarDecl], span: Span) -> Result<ArgKind> {
    let kind = ty_to_kind(ty, span)?;
    let ParamTypeName::Param(name) = &kind.meta else {
        return Ok(kind);
    };
    let Some(var) = type_params.iter().find(|v| v.name == name.as_str()) else {
        return Ok(kind);
    };
    let Some(bound) = &var.bound else {
        return Ok(kind);
    };
    let bound_text = bound.clone();
    let bound = parse_type_name(&bound_text, span, "type parameter bound", PARAM_TYPE_HINT)?;
    if mentions_variable(&bound) {
        return Ok(kind);
    }
    let members = match bound {
        ParamTypeName::Union(items) => items,
        other => vec![other],
    };
    let ctypes = members.iter().map(|m| m.outer_ctype()).collect();
    Ok(ArgKind {
        helper: "take_union",
        meta: kind.meta,
        union_members: Some(ctypes),
        want: Some(bound_text),
    })
}

/// Whether a type names a variable of the signature, which is what says it is
/// not a shape of its own yet.
fn mentions_variable(ty: &ParamTypeName) -> bool {
    match ty {
        ParamTypeName::Param(..) => true,
        ParamTypeName::Array(Some(inner)) => mentions_variable(inner),
        ParamTypeName::Table(k, v) => {
            k.as_deref().is_some_and(mentions_variable)
                || v.as_deref().is_some_and(mentions_variable)
        }
        ParamTypeName::Function(Some(sig)) => sig
            .params
            .iter()
            .chain(sig.returns.iter())
            .any(mentions_variable),
        ParamTypeName::Union(items) => items.iter().any(mentions_variable),
        _ => false,
    }
}

const PARAM_TYPE_HINT: &str = "int, num, number, string, bytes, bool, array, table, fn, nil, any, vararg, or a union like `fn | nil`";
const CONST_TYPE_HINT: &str = "int, float, num, string, bool, nil, any, table, array, fn";

struct TypeParser {
    chars: Vec<char>,
    pos: usize,
    span: Span,
    label: &'static str,
    hint: &'static str,
}

enum Number {
    I(i64),
    F(f64),
}

impl TypeParser {
    fn new(ty: &str, span: Span, label: &'static str, hint: &'static str) -> Self {
        Self {
            chars: ty.chars().collect(),
            pos: 0,
            span,
            label,
            hint,
        }
    }

    fn parse_full(mut self) -> Result<ParamTypeName> {
        let ty = self.parse()?;
        self.skip_ws();
        if self.pos != self.chars.len() {
            let rest: String = self.chars[self.pos..].iter().collect();
            return Err(Error::new(
                self.span,
                format!("unexpected `{}` in type", rest.trim()),
            ));
        }
        Ok(ty)
    }

    fn err(&self, msg: String) -> Error {
        Error::new(self.span, msg)
    }

    #[inline]
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    #[inline]
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_arrow(&mut self) -> bool {
        self.skip_ws();
        if self.chars.get(self.pos) == Some(&'-') && self.chars.get(self.pos + 1) == Some(&'>') {
            self.pos += 2;
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<String> {
        self.skip_ws();
        if !matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == '_') {
            return None;
        }
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        Some(self.chars[start..self.pos].iter().collect())
    }

    fn parse(&mut self) -> Result<ParamTypeName> {
        let mut items = vec![self.parse_prim()?];
        while self.eat('|') {
            items.push(self.parse_prim()?);
        }
        if items.len() == 1 {
            Ok(items.swap_remove(0))
        } else {
            Ok(ParamTypeName::Union(items))
        }
    }

    /// A type atom and the question or exclamation mark that may follow it.
    ///
    /// The mark binds to the atom, the way the language binds it: `int? | nil`
    /// is `int | nil | nil`, not `int | (nil?)`.
    fn parse_prim(&mut self) -> Result<ParamTypeName> {
        let ty = self.parse_atom()?;
        self.parse_suffix(ty)
    }

    fn parse_suffix(&mut self, ty: ParamTypeName) -> Result<ParamTypeName> {
        self.skip_ws();
        match self.peek() {
            Some('?') => {
                self.pos += 1;
                Ok(ty.nilable())
            }
            Some('!') => {
                self.pos += 1;
                Ok(ty.nonnilable())
            }
            _ => Ok(ty),
        }
    }

    /// A type atom: a name, a literal, a function signature, or a group of
    /// types in parentheses.
    fn parse_atom(&mut self) -> Result<ParamTypeName> {
        self.skip_ws();
        if self.peek() == Some('*') {
            self.pos += 1;
            return Ok(ParamTypeName::Any);
        }
        if self.peek() == Some('(') {
            self.pos += 1;
            let inner = self.parse()?;
            if !self.eat(')') {
                return Err(self.err("expected `)` after the type".to_owned()));
            }
            return Ok(inner);
        }
        if self.peek() == Some('"') {
            return Ok(ParamTypeName::Literal(LiteralKind::Str(
                self.parse_string_literal()?,
            )));
        }
        if self.at_number_literal() {
            return Ok(ParamTypeName::Literal(
                match self.parse_number_literal()? {
                    Number::I(i) => LiteralKind::Int(i),
                    Number::F(f) => LiteralKind::Float(f),
                },
            ));
        }
        let Some(word) = self.ident() else {
            return Err(self.err(format!(
                "expected a type name for the {}, expected one of: {}",
                self.label, self.hint
            )));
        };
        if word == "fn" {
            return self.parse_fn_sig();
        }
        // `userdata(File)` names the userdata type, and `userdata` on its own
        // is a userdata of any kind
        if word == "userdata" {
            if self.eat('(') {
                let Some(name) = self.ident() else {
                    return Err(self.err("expected the name of a userdata type".to_owned()));
                };
                if !self.eat(')') {
                    return Err(self.err("expected `)` after the userdata type".to_owned()));
                }
                return Ok(ParamTypeName::UserDataType(name));
            }
            return Ok(ParamTypeName::UserData);
        }
        // `type(CoStatus)` names a type the language declares, which is not
        // something a call can reach
        if word == "type" {
            if self.eat('(') {
                let Some(name) = self.ident() else {
                    return Err(self.err("expected the name of a type".to_owned()));
                };
                if !self.eat(')') {
                    return Err(self.err("expected `)` after the type".to_owned()));
                }
                return Ok(ParamTypeName::NamedType(name, vec![]));
            }
            return Err(
                self.err("expected `(` after `type`: a type is written `type(Name)`".to_owned())
            );
        }
        if self.peek() == Some('(') {
            // A name applied to arguments is a type function call, the way a
            // type position reads it. A name that is a type already is not.
            if is_type_name(&word) {
                return Err(self.err(format!("type `{word}` does not take a function signature")));
            }
            if !self.eat('(') {
                return Err(self.err("expected `(` after the name".to_owned()));
            }
            let mut args = vec![];
            if !self.eat(')') {
                loop {
                    args.push(self.parse()?);
                    if self.eat(',') {
                        continue;
                    }
                    if self.eat(')') {
                        break;
                    }
                    return Err(self.err("expected `,` or `)` in the arguments".to_owned()));
                }
            }
            return Ok(ParamTypeName::NamedType(word, args));
        }
        let base = match word.as_str() {
            "int" => ParamTypeName::Int,
            "float" | "num" => ParamTypeName::Num,
            "string" | "str" => ParamTypeName::String,
            "bytes" => ParamTypeName::Bytes,
            "bool" => ParamTypeName::Bool,
            "nil" => ParamTypeName::Nil,
            "any" => ParamTypeName::Any,
            "coroutine" => ParamTypeName::Coroutine,
            "func" | "function" => ParamTypeName::Function(None),
            "array" | "list" => ParamTypeName::Array(None),
            "table" => ParamTypeName::Table(None, None),
            "true" => return Ok(ParamTypeName::Literal(LiteralKind::Bool(true))),
            "false" => return Ok(ParamTypeName::Literal(LiteralKind::Bool(false))),
            // anything else is a name the signature declared, not a type: it is
            // checked against `type_params(...)` where that is written
            other => ParamTypeName::Param(other.to_owned()),
        };
        if !self.eat('<') {
            return Ok(base);
        }
        let args = self.parse_args()?;
        match (word.as_str(), args.len()) {
            ("array" | "list", 1) => {
                let mut args = args.into_iter();
                Ok(ParamTypeName::Array(args.next().map(Box::new)))
            }
            ("array" | "list", _) => {
                Err(self.err("array requires a type argument: array<int>".to_owned()))
            }
            ("table", 2) => {
                let mut args = args.into_iter();
                Ok(ParamTypeName::Table(
                    args.next().map(Box::new),
                    args.next().map(Box::new),
                ))
            }
            ("table", 0) => Ok(ParamTypeName::Table(None, None)),
            ("table", _) => Err(self.err("table takes two type arguments: table<K, V>".to_owned())),
            _ => Err(self.err(format!("type `{word}` does not take type arguments"))),
        }
    }

    /// A string literal, the opening quote included
    fn parse_string_literal(&mut self) -> Result<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("unterminated string literal in type".to_owned()));
            };
            self.pos += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let Some(esc) = self.peek() else {
                        return Err(self.err("unterminated escape in type".to_owned()));
                    };
                    self.pos += 1;
                    out.push(match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        other => other,
                    });
                }
                other => out.push(other),
            }
        }
    }

    fn at_number_literal(&self) -> bool {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => true,
            Some('-') => matches!(self.chars.get(self.pos + 1), Some(c) if c.is_ascii_digit()),
            _ => false,
        }
    }

    fn parse_number_literal(&mut self) -> Result<Number> {
        let start = self.pos;
        let mut float = false;

        if self.peek() == Some('-') {
            self.pos += 1;
        }

        loop {
            if matches!(self.peek(), Some('.')) && !float {
                float = true;
            } else if !matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                break;
            }
            self.pos += 1;
        }

        let text: String = self.chars[start..self.pos].iter().collect();
        if float {
            text.parse::<f64>()
                .map_err(|_| self.err(format!("invalid float literal `{text}` in type")))
                .map(Number::F)
        } else {
            text.parse::<i64>()
                .map_err(|_| self.err(format!("invalid integer literal `{text}` in type")))
                .map(Number::I)
        }
    }

    fn parse_fn_sig(&mut self) -> Result<ParamTypeName> {
        let type_params = self.parse_type_params()?;
        if !self.eat('(') {
            return Ok(ParamTypeName::Function(None));
        }
        let mut params = vec![];
        if !self.eat(')') {
            loop {
                if let Some(va) = self.try_vararg() {
                    params.push(va);
                    if !self.eat(')') {
                        return Err(self.err(
                            "`vararg` must be the last parameter in the fn signature".to_owned(),
                        ));
                    }
                    break;
                }
                params.push(self.parse()?);
                if self.eat(',') {
                    continue;
                }
                if self.eat(')') {
                    break;
                }
                return Err(self.err("expected `,` or `)` in the fn signature".to_owned()));
            }
        }
        let mut returns = vec![];
        if self.eat_arrow() {
            loop {
                if let Some(va) = self.try_vararg() {
                    returns.push(va);
                    if self.eat(',') {
                        return Err(self.err(
                            "`vararg` must be the last return in the fn signature".to_owned(),
                        ));
                    }
                    break;
                }
                returns.push(self.parse()?);
                if self.eat(',') {
                    continue;
                }
                break;
            }
        }
        Ok(ParamTypeName::Function(Some(Box::new(FnSignature {
            params,
            returns,
            type_params,
        }))))
    }

    /// The variables a signature declares, the `<T, U: int, ...Ts>` of
    /// `fn<T, U: int, ...Ts>(...)`.
    fn parse_type_params(&mut self) -> Result<Vec<VarDecl>> {
        let mut out = vec![];
        if !self.eat('<') {
            return Ok(out);
        }
        loop {
            let mut pack = false;
            if self.chars.get(self.pos) == Some(&'.')
                && self.chars.get(self.pos + 1) == Some(&'.')
                && self.chars.get(self.pos + 2) == Some(&'.')
            {
                pack = true;
                self.pos += 3;
            }
            let Some(name) = self.ident() else {
                return Err(self.err("expected the name of a type parameter".to_owned()));
            };
            let mut bound = None;
            let mut default = None;
            self.skip_ws();
            if self.peek() == Some(':') {
                self.pos += 1;
                bound = Some(self.parse_type_text()?);
            }
            self.skip_ws();
            if self.peek() == Some('=') {
                self.pos += 1;
                default = Some(self.parse_type_text()?);
            }
            out.push(VarDecl {
                name,
                bound,
                default,
                literal: false,
                pack,
            });
            self.skip_ws();
            if self.eat(',') {
                continue;
            }
            if self.eat('>') {
                break;
            }
            return Err(self.err("expected `,` or `>` in the type parameters".to_owned()));
        }
        Ok(out)
    }

    /// A type, read as the source text it is written as, so that `int | nil`
    /// survives as written.
    fn parse_type_text(&mut self) -> Result<String> {
        let start = self.pos;
        let mut depth = 0i32;
        loop {
            let Some(c) = self.peek() else {
                break;
            };
            match c {
                '<' => depth += 1,
                '>' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                ',' if depth == 0 => break,
                ')' if depth == 0 => break,
                _ => {}
            }
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if text.trim().is_empty() {
            return Err(self.err("expected a type".to_owned()));
        }
        Ok(text)
    }

    fn try_vararg(&mut self) -> Option<ParamTypeName> {
        self.skip_ws();
        if self.pos + 3 <= self.chars.len() && self.chars[self.pos..self.pos + 3] == ['.', '.', '.']
        {
            self.pos += 3;
            return Some(ParamTypeName::VarArg);
        }
        let save = self.pos;
        if self.ident().as_deref() == Some("vararg") {
            return Some(ParamTypeName::VarArg);
        }
        self.pos = save;
        None
    }

    fn parse_args(&mut self) -> Result<Vec<ParamTypeName>> {
        let mut args = vec![];
        if self.eat('>') {
            return Ok(args);
        }
        loop {
            args.push(self.parse()?);
            if self.eat(',') {
                continue;
            }
            if self.eat('>') {
                return Ok(args);
            }
            return Err(self.err("expected `,` or `>` in type arguments".to_owned()));
        }
    }
}

pub(crate) fn parse_type_name(
    ty: &str,
    span: Span,
    label: &'static str,
    hint: &'static str,
) -> Result<ParamTypeName> {
    TypeParser::new(ty, span, label, hint).parse_full()
}

/// A type of the language, as the metadata carries it: the same type the
/// signature itself uses, read by the same parser.
fn doc_type_of(text: &str, span: Span, label: &'static str) -> Result<TokenStream> {
    let ty = parse_type_name(text, span, label, PARAM_TYPE_HINT)?;
    ty.to_doc_type()
}

pub(crate) fn arg_kind(ty: &Type) -> Result<ArgKind> {
    let name = last_seg_ident(ty).unwrap_or_default();
    match name.as_str() {
        "DukaInt" | "i64" => Ok(ArgKind::plain("take_int", ParamTypeName::Int)),
        "DukaFloat" | "f64" => Ok(ArgKind::plain("take_num", ParamTypeName::Num)),
        "bool" => Ok(ArgKind::plain("take_bool", ParamTypeName::Bool)),
        "RuntimeValue" => Ok(ArgKind::plain("take_any", ParamTypeName::Any)),
        "String" | "str" => Ok(ArgKind::plain("take_string", ParamTypeName::String)),
        "Vec" => {
            let inner = single_generic(ty)
                .ok_or_else(|| Error::new_spanned(ty, "Vec requires a type argument"))?;
            match last_seg_ident(&inner).as_deref() {
                Some("u8") => Ok(ArgKind::plain("take_bytes", ParamTypeName::Bytes)),
                Some("RuntimeValue") => Ok(ArgKind::plain("take_many", ParamTypeName::VarArg)),
                _ => Err(Error::new_spanned(
                    ty,
                    "only Vec<u8> bytes Vec<RuntimeValue> are supported as a Vec<T> parameter",
                )),
            }
        }
        "Gc" => Ok(ArgKind::plain(
            "take_table",
            ParamTypeName::Table(None, None),
        )),
        _ => Err(Error::new_spanned(
            ty,
            "unsupported parameter type; use String, Vec<u8>, DukaInt/i64, DukaFloat/f64, bool, RuntimeValue, Vec<RuntimeValue> or a runtime table type",
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
    pub type_params: Vec<VarDecl>,
    /// The `where` list the call has to discharge
    pub where_clauses: Vec<WhereClauseMeta>,
    /// The constraints the call's values have to satisfy
    pub constraints: Vec<ConstraintClauseMeta>,
}

/// `T`, `literal T`, `T: bound`, `T = default` or `...Ts`, as written in
/// `type_params(...)`
#[derive(Debug)]
pub(crate) struct VarDecl {
    pub name: String,
    pub bound: Option<String>,
    pub default: Option<String>,
    pub literal: bool,
    /// `...Ts`: the variable takes the rest of the type arguments as a list
    pub pack: bool,
}

pub(crate) fn split_commas(ts: TokenStream) -> Vec<TokenStream> {
    let mut out = vec![];
    let mut cur = vec![];
    let mut depth = 0usize;
    let mut angle = 0usize;
    let mut last_punct = ' ';
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
            last_punct = ' ';
            continue;
        }
        if depth == 0
            && let TokenTree::Punct(p) = &tt
        {
            match p.as_char() {
                '<' => angle += 1,
                '>' if angle > 0 && last_punct != '-' => angle -= 1,
                ',' if angle == 0 => {
                    out.push(cur.drain(..).collect());
                    last_punct = ' ';
                    continue;
                }
                _ => {}
            }
            last_punct = p.as_char();
        }
        cur.push(tt);
    }
    if !cur.is_empty() {
        out.push(cur.into_iter().collect());
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttrShape {
    Func,
    Constant,
    Struct,
}

impl AttrShape {
    pub(crate) fn allowed(self) -> &'static [&'static str] {
        match self {
            AttrShape::Func => &[
                "name",
                "doc",
                "return_doc",
                "example",
                "params",
                "type_params",
                "returns",
                "where",
                "constraint",
                "flags",
            ],
            AttrShape::Constant => &["type", "name", "doc", "example", "value", "flags"],
            AttrShape::Struct => &["name", "doc", "example", "impls"],
        }
    }

    fn describe(self) -> &'static str {
        match self {
            AttrShape::Func => "duka_builtin function attribute",
            AttrShape::Constant => "duka_builtin constant attribute",
            AttrShape::Struct => "duka_builtin struct attribute",
        }
    }
}

const SAFE_USER_ATTRS: &[&str] = &[
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "doc",
    "inline",
    "must_use",
    "track_caller",
    "cold",
];

pub(crate) fn filter_user_attrs(
    attrs: &[syn::Attribute],
    what: &str,
) -> Result<Vec<syn::Attribute>> {
    let mut out = Vec::with_capacity(attrs.len());
    for a in attrs {
        let ident = a
            .path()
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if !SAFE_USER_ATTRS.contains(&ident.as_str()) {
            return Err(Error::new_spanned(
                a,
                format!(
                    "`#[{ident}]` is not supported on a duka_builtin {what}; allowed: {}",
                    SAFE_USER_ATTRS.join(", ")
                ),
            ));
        }
        out.push(a.clone());
    }
    Ok(out)
}

pub(crate) fn strip_impl_prefix(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("impl_").map(ToOwned::to_owned).unwrap_or(s)
}

pub(crate) fn for_each_attr_key(
    tokens: TokenStream,
    shape: AttrShape,
    mut f: impl FnMut(&str, TokenStream, Span) -> Result<()>,
) -> Result<()> {
    for seg in split_commas(tokens) {
        let mut toks: Vec<TokenTree> = seg.into_iter().collect();
        let Some(first) = toks.first().cloned() else {
            return Err(Error::new(
                Span::call_site(),
                format!("empty entry in {}", shape.describe()),
            ));
        };
        let TokenTree::Ident(key_ident) = first else {
            return Err(Error::new_spanned(
                &first,
                format!("expected a key in {}", shape.describe()),
            ));
        };
        let key = key_ident.to_string();
        if !shape.allowed().contains(&key.as_str()) {
            return Err(Error::new_spanned(
                &key_ident,
                format!(
                    "unknown {}; `{}` is not allowed, allowed keys: {}",
                    shape.describe(),
                    key,
                    shape.allowed().join(", ")
                ),
            ));
        }
        toks.remove(0);
        if toks.first().map(is_eq).unwrap_or(false) {
            toks.remove(0);
        }
        let rest: TokenStream = toks.into_iter().collect();
        f(&key, rest, key_ident.span())?;
    }
    Ok(())
}

fn const_type(ty: &str, span: Span) -> Result<ParamTypeName> {
    let meta = parse_type_name(ty, span, "constant", CONST_TYPE_HINT)?;
    check_const_type(&meta, span)?;
    Ok(meta)
}

/// Whether a name is one of the types the language has, rather than a variable
/// of the signature or a type the language declares.
///
/// A name that is a type already does not take arguments, so `int(...)` is a
/// mistake rather than an application.
fn is_type_name(word: &str) -> bool {
    matches!(
        word,
        "int"
            | "float"
            | "num"
            | "string"
            | "str"
            | "bytes"
            | "bool"
            | "nil"
            | "any"
            | "coroutine"
            | "func"
            | "function"
            | "array"
            | "list"
            | "table"
            | "userdata"
    )
}

/// A `where` clause a builtin writes, as the macro reads it.
#[derive(Debug)]
pub(crate) enum WhereClauseMeta {
    /// `bound(T: int)` -- what a type parameter has to accept
    Bound { var: String, ty: ParamTypeName },
    /// `bind(V: int)` -- a type-level binding, the `type V = T` of a `where`
    Bind { var: String, ty: ParamTypeName },
    /// `concept(Name, args...)` -- a concept to discharge
    Concept {
        name: String,
        args: Vec<ParamTypeName>,
    },
}

/// A constraint a builtin's values have to satisfy, as the macro reads it.
#[derive(Debug)]
pub(crate) enum ConstraintClauseMeta {
    Is { arg: usize, ty: ParamTypeName },
    IsNot { arg: usize, ty: ParamTypeName },
    Sub { arg: usize, ty: ParamTypeName },
    Instance { arg: usize, ty: ParamTypeName },
    Has { arg: usize, field: String },
    Never { arg: usize },
}

/// The `where` clauses a builtin writes.
///
/// A clause is one of three things: a bound on a type parameter, a type-level
/// binding, or a concept to discharge. A concept is named and given its
/// arguments rather than written out, because a builtin writes no expressions.
fn parse_where_clauses(tokens: TokenStream, span: Span) -> Result<Vec<WhereClauseMeta>> {
    let inner = unwrap_paren(&tokens)?;
    let mut out = vec![];
    for entry in split_commas(inner) {
        if entry.is_empty() {
            continue;
        }
        let mut toks: Vec<TokenTree> = entry.into_iter().collect();
        let Some(TokenTree::Ident(head)) = toks.first().cloned() else {
            return Err(Error::new(span, "expected a `where` clause"));
        };
        let head = head.to_string();
        toks.remove(0);
        if toks.first().map(is_eq).unwrap_or(false) {
            toks.remove(0);
        }
        let args = unwrap_paren(&toks.into_iter().collect())?;
        match head.as_str() {
            "bound" | "bind" => {
                let (name, text) = named_type_arg(args, span)?;
                let name = name.into_string();
                let ty = parse_type_name(&text, span, "where clause", PARAM_TYPE_HINT)?;
                if head == "bound" {
                    out.push(WhereClauseMeta::Bound { var: name, ty });
                } else {
                    out.push(WhereClauseMeta::Bind { var: name, ty });
                }
            }
            "concept" => {
                let mut parts = split_commas(args);
                let Some(name) = parts.first().cloned() else {
                    return Err(Error::new(span, "expected the name of a concept"));
                };
                parts.remove(0);
                let mut tys = vec![];
                for part in parts {
                    let text = tokens_to_text(&part);
                    tys.push(parse_type_name(
                        &text,
                        span,
                        "concept argument",
                        PARAM_TYPE_HINT,
                    )?);
                }
                out.push(WhereClauseMeta::Concept {
                    name: name.to_string(),
                    args: tys,
                });
            }
            other => {
                return Err(Error::new(
                    span,
                    format!("unknown `where` clause `{other}`; expected bound, bind or concept"),
                ));
            }
        }
    }
    Ok(out)
}

/// The constraints a builtin's values have to satisfy.
///
/// A constraint is one atom about one argument: `is(0, int)` says the first
/// argument has to be an int. The atoms are the ones the flow already applies,
/// so a builtin says what has to hold and the flow works out what to do.
fn parse_constraints(tokens: TokenStream, span: Span) -> Result<Vec<ConstraintClauseMeta>> {
    let inner = unwrap_paren(&tokens)?;
    let mut out = vec![];
    for entry in split_commas(inner) {
        if entry.is_empty() {
            continue;
        }
        let mut toks: Vec<TokenTree> = entry.into_iter().collect();
        let Some(TokenTree::Ident(head)) = toks.first().cloned() else {
            return Err(Error::new(span, "expected a constraint"));
        };
        let head = head.to_string();
        toks.remove(0);
        if toks.first().map(is_eq).unwrap_or(false) {
            toks.remove(0);
        }
        let args = unwrap_paren(&toks.into_iter().collect())?;
        let parts = split_commas(args);
        let Some(first) = parts.first().cloned() else {
            return Err(Error::new(span, "expected an argument"));
        };
        let arg = tokens_to_text(&first)
            .trim()
            .parse::<usize>()
            .map_err(|_| Error::new(span, "expected the number of an argument"))?;
        let rest: Vec<TokenStream> = parts.into_iter().skip(1).collect();
        let ty_of = |text: &str| -> Result<ParamTypeName> {
            parse_type_name(text, span, "constraint", PARAM_TYPE_HINT)
        };
        let Some(rest0) = rest.first() else {
            return Err(Error::new(span, "expected a type"));
        };
        out.push(match head.as_str() {
            "is" => ConstraintClauseMeta::Is {
                arg,
                ty: ty_of(&tokens_to_text(rest0))?,
            },
            "is_not" => ConstraintClauseMeta::IsNot {
                arg,
                ty: ty_of(&tokens_to_text(rest0))?,
            },
            "sub" => ConstraintClauseMeta::Sub {
                arg,
                ty: ty_of(&tokens_to_text(rest0))?,
            },
            "instance" => ConstraintClauseMeta::Instance {
                arg,
                ty: ty_of(&tokens_to_text(rest0))?,
            },
            "has" => {
                let field = tokens_to_text(rest0);
                let field = field.trim().trim_matches('"').to_owned();
                ConstraintClauseMeta::Has {
                    arg,
                    field,
                }
            }
            "never" => ConstraintClauseMeta::Never { arg },
            other => {
                return Err(Error::new(
                    span,
                    format!(
                        "unknown constraint `{other}`; expected is, is_not, sub, instance, has or never"
                    ),
                ));
            }
        });
    }
    Ok(out)
}

/// A `where` clause, as the metadata carries it.
fn where_clause_tokens(clause: &WhereClauseMeta) -> Result<TokenStream> {
    let root = root_tokens()?;
    let base = quote! { #root::duka_shared::docs::WhereMeta };
    Ok(match clause {
        WhereClauseMeta::Bound { var, ty } => {
            let var = LitStr::new(var, Span::call_site());
            let ty = ty.to_doc_type()?;
            quote! { #base::Bound { var: #var, ty: #ty } }
        }
        WhereClauseMeta::Bind { var, ty } => {
            let var = LitStr::new(var, Span::call_site());
            let ty = ty.to_doc_type()?;
            quote! { #base::Bind { var: #var, ty: #ty } }
        }
        WhereClauseMeta::Concept { name, args } => {
            let name = LitStr::new(name, Span::call_site());
            let args = args
                .iter()
                .map(|a| a.to_doc_type())
                .collect::<Result<Vec<_>>>()?;
            quote! { #base::Concept { name: #name, args: &[#(#args),*] } }
        }
    })
}

/// A constraint, as the metadata carries it.
fn constraint_tokens(constraint: &ConstraintClauseMeta) -> Result<TokenStream> {
    let root = root_tokens()?;
    let base = quote! { #root::duka_shared::docs::ConstraintMeta };
    Ok(match constraint {
        ConstraintClauseMeta::Is { arg, ty } => {
            let ty = ty.to_doc_type()?;
            quote! { #base::Is { arg: #arg, ty: #ty } }
        }
        ConstraintClauseMeta::IsNot { arg, ty } => {
            let ty = ty.to_doc_type()?;
            quote! { #base::IsNot { arg: #arg, ty: #ty } }
        }
        ConstraintClauseMeta::Sub { arg, ty } => {
            let ty = ty.to_doc_type()?;
            quote! { #base::Sub { arg: #arg, ty: #ty } }
        }
        ConstraintClauseMeta::Instance { arg, ty } => {
            let ty = ty.to_doc_type()?;
            quote! { #base::Instance { arg: #arg, ty: #ty } }
        }
        ConstraintClauseMeta::Has { arg, field } => {
            let field = LitStr::new(field, Span::call_site());
            quote! { #base::Has { arg: #arg, field: #field } }
        }
        ConstraintClauseMeta::Never { arg } => {
            quote! { #base::Never { arg: #arg } }
        }
    })
}

/// The text a group of tokens is written as.
fn tokens_to_text(tokens: &TokenStream) -> String {
    tokens
        .clone()
        .into_iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join("")
}

/// A name and the type it is given, the `T: int` of a `where` clause.
fn named_type_arg(tokens: TokenStream, span: Span) -> Result<(Box<str>, String)> {
    let mut name = String::new();
    let mut ty = String::new();
    let mut colon = false;
    for tt in tokens {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ':' && !colon => colon = true,
            _ if colon => ty.push_str(&tt.to_string()),
            _ => name.push_str(&tt.to_string()),
        }
    }
    if name.is_empty() {
        return Err(Error::new(span, "expected a name"));
    }
    if ty.trim().is_empty() {
        return Err(Error::new(span, "expected a type"));
    }
    Ok((name.into_boxed_str(), ty))
}

fn check_const_type(meta: &ParamTypeName, span: Span) -> Result<()> {
    match meta {
        ParamTypeName::String
        | ParamTypeName::Int
        | ParamTypeName::Num
        | ParamTypeName::Bool
        | ParamTypeName::Nil
        | ParamTypeName::Any
        | ParamTypeName::Literal(..)
        | ParamTypeName::VarArg => Ok(()),
        ParamTypeName::Array(inner) => match inner {
            Some(inner) => check_const_type(inner, span),
            None => Ok(()),
        },
        ParamTypeName::Table(k, v) => {
            if let Some(k) = k {
                check_const_type(k, span)?;
            }
            if let Some(v) = v {
                check_const_type(v, span)?;
            }
            Ok(())
        }
        ParamTypeName::Function(sig) => match sig {
            None => Ok(()),
            Some(sig) => {
                for ty in sig.params.iter().chain(sig.returns.iter()) {
                    check_const_type(ty, span)?;
                }
                Ok(())
            }
        },
        other => Err(Error::new(
            span,
            format!(
                "unsupported constant type `{}`; expected one of: {CONST_TYPE_HINT}",
                other.name()
            ),
        )),
    }
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
    for_each_attr_key(tokens, AttrShape::Constant, |key, rest, span| match key {
        "type" => {
            let ty = lit_str(&rest)?;
            args.ty = const_type(&ty, span)?;
            Ok(())
        }
        "name" => {
            args.name = lit_str(&rest)?;
            Ok(())
        }
        "doc" => {
            args.doc = lit_str(&rest)?;
            Ok(())
        }
        "example" => {
            args.example = Some(lit_str(&rest)?);
            Ok(())
        }
        "value" => {
            args.val = Some(lit_str(&rest)?);
            Ok(())
        }
        "flags" => {
            let inner = unwrap_paren(&rest)?;
            args.flags = syn::parse2::<MetaInfoFlags>(inner)?;
            Ok(())
        }
        _ => Err(Error::new(
            span,
            format!("unhandled constant attribute `{key}`"),
        )),
    })?;
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
        type_params: vec![],
        where_clauses: vec![],
        constraints: vec![],
        returns: vec![],
        return_var_arg: false,
        flags: MetaInfoFlags::default(),
    };
    for_each_attr_key(tokens, AttrShape::Func, |key, rest, span| match key {
        "name" => {
            args.name = Some(lit_str(&rest)?);
            Ok(())
        }
        "doc" => {
            args.doc = lit_str(&rest)?;
            Ok(())
        }
        "return_doc" => {
            args.return_doc = lit_str(&rest)?;
            Ok(())
        }
        "example" => {
            args.example = Some(lit_str(&rest)?);
            Ok(())
        }
        "returns" => {
            let inner = unwrap_paren(&rest)?;
            let (items, var_arg) = parse_returns(inner)?;
            for item in items {
                args.returns.push(RawReturn { ty: item });
            }
            args.return_var_arg = var_arg;
            Ok(())
        }
        "params" => {
            let inner = unwrap_paren(&rest)?;
            for p in parse_params(inner)? {
                args.params.push(p);
            }
            Ok(())
        }
        "type_params" => {
            let inner = unwrap_paren(&rest)?;
            for entry in split_commas(inner) {
                if entry.is_empty() {
                    continue;
                }
                let mut entry: Vec<TokenTree> = entry.into_iter().collect();
                // `literal` is a marker on the variable rather than part of its
                // name, so it comes off the front before the name is read
                let mut literal = false;
                if let Some(TokenTree::Ident(i)) = entry.first()
                    && i == "const"
                {
                    literal = true;
                    entry.remove(0);
                }
                // `...Ts` takes the rest of the type arguments as a list, so
                // the dots come off the front too, the way they do in the
                // language
                let mut pack = false;
                if entry.len() >= 3
                    && entry[..3]
                        .iter()
                        .all(|tt| matches!(tt, TokenTree::Punct(p) if p.as_char() == '.'))
                {
                    pack = true;
                    entry.drain(..3);
                }
                let mut name = String::new();
                let mut bound = String::new();
                let mut default = String::new();
                let mut colon = false;
                let mut assign = false;
                for tt in entry {
                    match &tt {
                        // everything after the colon is the bound, read back as
                        // source text so `int | nil` survives as written
                        TokenTree::Punct(p) if p.as_char() == ':' && !colon && !assign => {
                            colon = true;
                        }
                        // and everything after the equals sign is the default,
                        // which may follow a bound or stand on its own
                        TokenTree::Punct(p) if p.as_char() == '=' && !assign => {
                            assign = true;
                        }
                        _ if assign => default.push_str(&tt.to_string()),
                        _ if colon => bound.push_str(&tt.to_string()),
                        _ => name.push_str(&tt.to_string()),
                    }
                }
                let bound = (!bound.trim().is_empty()).then_some(bound);
                let default = (!default.trim().is_empty()).then_some(default);
                if name.is_empty() {
                    return Err(Error::new(span, "a type parameter needs a name"));
                }
                args.type_params.push(VarDecl {
                    name,
                    bound,
                    default,
                    literal,
                    pack,
                });
            }
            Ok(())
        }
        "where" => {
            args.where_clauses = parse_where_clauses(rest, span)?;
            Ok(())
        }
        "constraint" => {
            args.constraints = parse_constraints(rest, span)?;
            Ok(())
        }
        "flags" => {
            let inner = unwrap_paren(&rest)?;
            args.flags = syn::parse2::<MetaInfoFlags>(inner)?;
            Ok(())
        }
        _ => Err(Error::new(
            span,
            format!("unhandled function attribute `{key}`"),
        )),
    })?;
    // The type parser reads any bare name as a parameter, so that `as.<T>` can
    // say `T`. A name no `type_params(...)` declared is a misspelling, and it is
    // here rather than in the parser that it can be told apart from one that was.
    let declared: Vec<&str> = args.type_params.iter().map(|v| v.name.as_str()).collect();
    let texts = args
        .params
        .iter()
        // the receiver's `userdata` is written by the macro, not parsed, so it
        // never reaches the type parser and is not a parameter reference
        .filter(|p| !p.is_userdata)
        .filter_map(|p| p.ty.as_deref())
        .chain(args.returns.iter().map(|r| r.ty.as_str()));
    let mut used: Vec<String> = vec![];
    for text in texts {
        if let Ok(ty) = parse_type_name(text, Span::call_site(), "parameter", PARAM_TYPE_HINT) {
            collect_params(&ty, &mut used);
        }
    }
    for name in used {
        if !declared.contains(&name.as_str()) {
            return Err(Error::new(
                Span::call_site(),
                format!("`{name}` is not a type parameter; declare it in type_params(...)"),
            ));
        }
    }
    Ok(args)
}

/// Every parameter name a type mentions, in order and without repeats
fn collect_params(ty: &ParamTypeName, out: &mut Vec<String>) {
    match ty {
        ParamTypeName::Param(name) => {
            if !out.iter().any(|i| i == name) {
                out.push(name.clone());
            }
        }
        ParamTypeName::Array(Some(inner)) => collect_params(inner, out),
        ParamTypeName::Table(k, v) => {
            if let Some(k) = k {
                collect_params(k, out);
            }
            if let Some(v) = v {
                collect_params(v, out);
            }
        }
        ParamTypeName::Function(Some(sig)) => {
            for t in sig.params.iter().chain(sig.returns.iter()) {
                collect_params(t, out);
            }
        }
        ParamTypeName::Union(items) => {
            for t in items {
                collect_params(t, out);
            }
        }
        // the arguments a named type is applied to may name a variable
        ParamTypeName::NamedType(_, args) => {
            for t in args {
                collect_params(t, out);
            }
        }
        _ => {}
    }
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
        let Some(first_tok) = toks.first() else {
            continue;
        };
        let name_ident = match first_tok {
            TokenTree::Ident(i) => i.clone(),
            TokenTree::Punct(p) if p.as_char() == '@' => {
                let Some(TokenTree::Ident(i)) = toks.get(1) else {
                    return Err(Error::new_spanned(
                        p,
                        "expected an annotation name after `@` in params",
                    ));
                };
                let ann = i.to_string();
                let Some(vals) = toks.get(3..) else {
                    return Err(Error::new_spanned(
                        i,
                        format!("expected `@{ann} = \"...\"` in params"),
                    ));
                };
                let val: LitStr = syn::parse2(vals.iter().cloned().collect()).map_err(|e| {
                    Error::new_spanned(i, format!("expected `@{ann} = \"...\"` in params: {e}"))
                })?;
                match ann.as_str() {
                    "default" => {
                        if let Some(i) = last_param {
                            out[i].default_display = Some(val.value());
                        } else {
                            return Err(Error::new_spanned(
                                i,
                                "`@default` must follow a parameter in params",
                            ));
                        }
                    }
                    "doc" => {
                        if let Some(i) = last_param {
                            out[i].doc = Some(val.value());
                        } else {
                            return Err(Error::new_spanned(
                                i,
                                "`@doc` must follow a parameter in params",
                            ));
                        }
                    }
                    _ => {
                        return Err(Error::new_spanned(
                            i,
                            format!("unknown params annotation `@{ann}`; allowed: @default, @doc"),
                        ));
                    }
                }
                continue;
            }
            other => {
                return Err(Error::new_spanned(
                    other,
                    "expected `name: type` or `@annotation = \"...\"` in params",
                ));
            }
        };
        let name = name_ident.to_string();
        if out.iter().any(|p| p.name == name) {
            return Err(Error::new_spanned(
                &name_ident,
                format!("duplicate parameter `{name}` in params"),
            ));
        }

        if !toks.get(1).map(is_colon).unwrap_or(false) {
            return Err(Error::new_spanned(
                &name_ident,
                format!("expected `: type` after parameter `{name}`"),
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
            name,
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

pub(crate) fn parse_type(s: &str) -> Result<Type> {
    let ts: TokenStream = s
        .parse()
        .map_err(|e| Error::new(Span::call_site(), format!("invalid type `{s}`: {e}")))?;
    syn::parse2(ts.clone()).map_err(|e| Error::new_spanned(&ts, format!("invalid type `{s}`: {e}")))
}

pub(crate) fn parse_str_tokens(s: &str, what: &str) -> Result<TokenStream> {
    s.parse::<TokenStream>()
        .map_err(|e| Error::new(Span::call_site(), format!("invalid {what} `{s}`: {e}")))
}

pub(crate) fn root_tokens() -> Result<TokenStream> {
    let s = resolve_root_str();
    s.parse()
        .map_err(|e| Error::new(Span::call_site(), format!("invalid crate path `{s}`: {e}")))
}

macro_rules! parse_type_or {
    ($s:expr) => {
        crate::attr::or_compile_error!(crate::attr::parse_type($s))
    };
}
pub(crate) use parse_type_or;

macro_rules! or_compile_error {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e.into_compile_error(),
        }
    };
}
pub(crate) use or_compile_error;

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
) -> Result<TokenStream> {
    let vararg = meta.vararg;
    let name = LitStr::new(&meta.name, Span::call_site());
    let ty = kind.meta.to_doc_type()?;
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
    Ok(quote! {
        #krate::duka_shared::docs::ParamMeta {
            name: #name,
            ty: #ty,
            optional: #optional,
            default: #default,
            var_arg: #vararg,
            doc: #doc,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn err_of<T>(r: Result<T>) -> Error {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => e,
        }
    }

    #[test]
    fn unknown_attr_key_lists_allowed_keys() {
        let err = err_of(parse_builtin_args(quote! { name = "f", bogus = 1 }));
        let msg = err.to_string();
        assert!(msg.contains("`bogus`"), "{msg}");
        assert!(msg.contains("allowed keys"), "{msg}");
        assert!(msg.contains("params"), "{msg}");
    }

    #[test]
    fn struct_attr_unknown_key_rejected() {
        let err = err_of(for_each_attr_key(
            quote! { name = "X", ty = "int" },
            AttrShape::Struct,
            |_, _, _| Ok(()),
        ));
        assert!(err.to_string().contains("allowed keys"), "{err}");
    }

    #[test]
    fn const_type_whitelist() {
        assert!(const_type("fn", Span::call_site()).is_ok());
        assert!(const_type("str", Span::call_site()).is_ok());
        assert!(const_type("list", Span::call_site()).is_ok());
        for bad in ["bytes", "number", "vararg", "union", "bogus"] {
            let err = err_of(const_type(bad, Span::call_site()));
            assert!(
                err.to_string().contains("unsupported constant type"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn func_args_parse() {
        let args = parse_builtin_args(quote! {
            name = "f",
            doc = "d",
            params(a: int, b: fn | nil = nil, rest: vararg),
            returns(bool, string, vararg),
            flags(@feature(platform))
        })
        .unwrap();
        assert_eq!(args.name.as_deref(), Some("f"));
        assert_eq!(args.doc, "d");
        assert_eq!(args.params.len(), 3);
        assert_eq!(args.params[0].name, "a");
        assert_eq!(args.params[1].ty.as_deref(), Some("fn|nil"));
        assert!(args.params[1].default.is_some());
        assert!(args.params[2].vararg);
        assert_eq!(args.returns.len(), 2);
        assert!(args.return_var_arg);
    }

    #[test]
    fn const_args_require_name() {
        let err = err_of(parse_builtin_const_args(quote! { doc = "d" }));
        assert!(err.to_string().contains("`name`"), "{err}");
    }

    #[test]
    fn params_duplicate_name_rejected() {
        let err = err_of(parse_params(quote! { x: int, y: int, x: int }));
        assert!(err.to_string().contains("duplicate parameter `x`"), "{err}");
    }

    #[test]
    fn params_annotation_binds_previous_param() {
        let ps = parse_params(quote! { s: string = " ".to_owned(), @default = "\" \"" }).unwrap();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].name, "s");
        assert_eq!(ps[0].default_display.as_deref(), Some("\" \""));
        assert!(ps[0].default.is_some());
    }

    #[test]
    fn params_annotation_without_value_rejected() {
        let err = err_of(parse_params(quote! { s: string, @default }));
        assert!(err.to_string().contains("@default"), "{err}");
    }

    #[test]
    fn params_missing_colon_rejected() {
        let err = err_of(parse_params(quote! { x int }));
        assert!(
            err.to_string()
                .contains("expected `: type` after parameter `x`"),
            "{err}"
        );
    }

    #[test]
    fn params_annotation_before_any_param_rejected() {
        let err = err_of(parse_params(quote! { @doc = "x" }));
        assert!(err.to_string().contains("must follow a parameter"), "{err}");
    }

    #[test]
    fn returns_parse() {
        let (items, var_arg) = parse_returns(quote! { bool, string, vararg }).unwrap();
        assert_eq!(items, vec!["bool".to_string(), "string".to_string()]);
        assert!(var_arg);
    }

    #[test]
    fn split_commas_keeps_nested_groups() {
        let parts = split_commas(quote! { a = f(x, y), b = 2 });
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn strip_impl_prefix_strips_once() {
        let one: Ident = syn::parse_str("impl_foo").unwrap();
        let twice: Ident = syn::parse_str("impl_impl_foo").unwrap();
        let plain: Ident = syn::parse_str("foo").unwrap();
        assert_eq!(strip_impl_prefix(&one), "foo");
        assert_eq!(strip_impl_prefix(&twice), "impl_foo");
        assert_eq!(strip_impl_prefix(&plain), "foo");
    }

    #[test]
    fn user_attrs_whitelisted() {
        let f: syn::ItemFn = syn::parse_quote! {
            #[allow(unused)]
            #[doc = "x"]
            #[inline]
            fn f() {}
        };
        assert_eq!(filter_user_attrs(&f.attrs, "function").unwrap().len(), 3);

        let g: syn::ItemFn = syn::parse_quote! {
            #[cfg(feature = "x")]
            fn g() {}
        };
        let err = err_of(filter_user_attrs(&g.attrs, "function"));
        assert!(err.to_string().contains("`#[cfg]`"), "{err}");
        assert!(err.to_string().contains("allowed"), "{err}");
    }

    #[test]
    fn parse_type_reports_invalid_input() {
        let err = err_of(parse_type("Vec<"));
        assert!(err.to_string().contains("invalid type"), "{err}");
    }

    fn parse_name(ty: &str) -> Result<String> {
        Ok(parse_type_name(ty, Span::call_site(), "parameter", PARAM_TYPE_HINT)?.name())
    }

    #[test]
    fn parse_generic_types() {
        let cases = [
            ("int", "int"),
            ("list", "array"),
            ("table", "table"),
            ("array<int>", "array<int>"),
            ("table<string,int>", "table<string, int>"),
            ("fn", "fn"),
            ("fn(int)", "fn(int)"),
            ("fn(int,string)->bool", "fn(int, string) -> bool"),
            ("fn()->int|nil", "fn() -> int | nil"),
            ("array<table<string,int>>", "array<table<string, int>>"),
            ("array<int>|nil", "array<int> | nil"),
            ("*", "any"),
        ];
        for (input, want) in cases {
            assert_eq!(parse_name(input).unwrap(), want, "{input}");
        }
    }

    #[test]
    fn parse_type_rejects_bad_generics() {
        for bad in ["array<int", "array<>", "table<int>", "int<str>", "fn(", ""] {
            let err = err_of(parse_name(bad));
            assert!(!err.to_string().is_empty(), "{bad}");
        }
    }

    /// A group of types in parentheses is one type, so what is inside it can be
    /// a union of its own.
    #[test]
    fn a_group_of_types_is_one_type() {
        assert_eq!(parse_name("(int | string)").unwrap(), "int | string");
        assert_eq!(
            parse_name("(fn() -> int) | nil").unwrap(),
            "fn() -> int | nil"
        );
        assert_eq!(parse_name("((int))").unwrap(), "int");
        assert_eq!(
            parse_name("(int, string)").unwrap_err().to_string().len() > 0,
            true
        );
    }

    /// The question mark adds nil to a type and the exclamation mark takes it
    /// out, which is what the language means by them.
    #[test]
    fn a_question_mark_makes_a_type_nilable() {
        assert_eq!(parse_name("int?").unwrap(), "int | nil");
        assert_eq!(parse_name("int!").unwrap(), "int");
        assert_eq!(parse_name("(int | nil)!").unwrap(), "int");
        assert_eq!(parse_name("(int | nil)?").unwrap(), "int | nil");
        assert_eq!(parse_name("nil!").unwrap(), "any");
        assert_eq!(parse_name("any?").unwrap(), "any");
        assert_eq!(
            parse_name("fn(int?) -> bool").unwrap(),
            "fn(int | nil) -> bool"
        );
    }

    /// A literal is a type of its own: the value it names is the only one of
    /// its kind.
    #[test]
    fn a_literal_is_a_type() {
        assert_eq!(parse_name("\"name\"").unwrap(), "\"name\"");
        assert_eq!(parse_name("42").unwrap(), "42");
        assert_eq!(parse_name("true").unwrap(), "true");
        assert_eq!(parse_name("false").unwrap(), "false");
        let doc = parse_type_name("\"name\"", Span::call_site(), "parameter", PARAM_TYPE_HINT)
            .unwrap()
            .to_doc_type_str()
            .unwrap();
        assert!(doc.contains("ConstValue::String"), "{doc}");
        let doc = parse_type_name("42", Span::call_site(), "parameter", PARAM_TYPE_HINT)
            .unwrap()
            .to_doc_type_str()
            .unwrap();
        assert!(doc.contains("ConstValue::Int(42)"), "{doc}");
    }

    /// A type that may be nil is a union, so the reader reads it as one.
    #[test]
    fn a_nilable_type_is_read_as_a_union() {
        let kind = ty_to_kind("int?", Span::call_site()).unwrap();
        assert_eq!(kind.helper, "take_union");
        assert_eq!(kind.union_members.as_deref(), Some(&["INT", "NIL"][..]));
    }

    /// A type parameter may say what it has to accept, what it is when nothing
    /// else says, and whether it takes the rest of the type arguments.
    #[test]
    fn type_params_read_a_bound_a_default_and_a_pack() {
        let args = parse_builtin_args(quote! {
            name = "f",
            type_params(T: int, U = string, ...Ts),
            params(a: T, b: U, c: Ts),
            returns(T)
        })
        .expect("parsed");
        assert_eq!(args.type_params.len(), 3);
        assert_eq!(args.type_params[0].name, "T");
        assert_eq!(args.type_params[0].bound.as_deref(), Some("int"));
        assert_eq!(args.type_params[0].default, None);
        assert!(!args.type_params[0].pack);
        assert_eq!(args.type_params[1].name, "U");
        assert_eq!(args.type_params[1].bound, None);
        assert_eq!(args.type_params[1].default.as_deref(), Some("string"));
        assert_eq!(args.type_params[2].name, "Ts");
        assert!(args.type_params[2].pack);
    }

    /// A bound and a default may be written together, the way the language
    /// writes them.
    #[test]
    fn a_type_parameter_may_have_a_bound_and_a_default() {
        let args = parse_builtin_args(quote! {
            name = "f",
            type_params(T: int = string),
            params(a: T),
            returns(T)
        })
        .expect("parsed");
        assert_eq!(args.type_params[0].bound.as_deref(), Some("int"));
        assert_eq!(args.type_params[0].default.as_deref(), Some("string"));
    }

    /// A parameter whose type is a variable the signature bounds is read the
    /// way the bound says, when the bound says it exactly.
    #[test]
    fn a_bounded_type_parameter_is_read_the_way_its_bound_says() {
        let args = parse_builtin_args(quote! {
            name = "f",
            type_params(T: int | float),
            params(val: T),
            returns(T)
        })
        .expect("parsed");
        let kind = ty_to_kind_bounded("T", &args.type_params, Span::call_site()).unwrap();
        assert_eq!(kind.helper, "take_union");
        assert_eq!(kind.union_members.as_deref(), Some(&["INT", "NUM"][..]));
        assert_eq!(kind.want.as_deref(), Some("int|float"));

        // a variable with no bound says nothing about the value, so the value
        // is read as any
        let args = parse_builtin_args(quote! {
            name = "f",
            type_params(T),
            params(val: T),
            returns(T)
        })
        .expect("parsed");
        let kind = ty_to_kind_bounded("T", &args.type_params, Span::call_site()).unwrap();
        assert_eq!(kind.helper, "take_any");
        assert_eq!(kind.union_members, None);
    }

    /// A function type carries the variables it declares, so a call site can
    /// solve for them without knowing where the type was written.
    #[test]
    fn a_function_type_carries_its_variables() {
        let parse = |ty: &str| {
            parse_type_name(ty, Span::call_site(), "parameter", PARAM_TYPE_HINT).unwrap()
        };
        assert_eq!(parse("fn<T>(T) -> T").name(), "fn<T>(T) -> T");
        assert_eq!(parse("fn<T: int>(T) -> T").name(), "fn<T>(T) -> T");
        assert_eq!(parse("fn<...Ts>(Ts) -> Ts").name(), "fn<...Ts>(Ts) -> Ts");
        let doc = parse("fn<T>(T) -> T").to_doc_type_str().unwrap();
        assert!(doc.contains("DocType::Function(&["), "{doc}");
        assert!(doc.contains("VarMeta"), "{doc}");
        assert!(doc.contains("name: \"T\""), "{doc}");
    }

    /// A `where` list is what a call has to discharge, and a constraint is
    /// what the call's values have to satisfy.
    #[test]
    fn a_where_list_and_a_constraint_are_read() {
        let args = parse_builtin_args(quote! {
            name = "f",
            type_params(T),
            params(a: T),
            returns(T),
            where(bound(T: int), bind(V: string), concept(IsList, T)),
            constraint(is(0, int))
        })
        .expect("parsed");
        assert_eq!(args.where_clauses.len(), 3);
        assert_eq!(args.constraints.len(), 1);
        match &args.where_clauses[0] {
            WhereClauseMeta::Bound { var, ty } => {
                assert_eq!(var, "T");
                assert_eq!(ty.name(), "int");
            }
            other => panic!("expected a bound, got {other:?}"),
        }
        match &args.where_clauses[2] {
            WhereClauseMeta::Concept { name, args } => {
                assert_eq!(name, "IsList");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].name(), "T");
            }
            other => panic!("expected a concept, got {other:?}"),
        }
        match &args.constraints[0] {
            ConstraintClauseMeta::Is { arg, ty } => {
                assert_eq!(*arg, 0);
                assert_eq!(ty.name(), "int");
            }
            other => panic!("expected a constraint, got {other:?}"),
        }
    }

    /// A `where` clause is one of three things, and anything else is a
    /// misspelling.
    #[test]
    fn an_unknown_where_clause_is_refused() {
        let err = err_of(parse_builtin_args(quote! {
            name = "f",
            params(a: int),
            returns(int),
            where(nonsense(T))
        }));
        assert!(err.to_string().contains("unknown `where` clause"), "{err}");
    }

    /// A type the language declares is written `type(Name)`, and a name applied
    /// to arguments is a type function call, the way a type position reads it.
    #[test]
    fn a_named_type_is_read_as_a_name_and_not_as_a_variable() {
        let parse = |ty: &str| {
            parse_type_name(ty, Span::call_site(), "parameter", PARAM_TYPE_HINT).unwrap()
        };
        assert_eq!(parse("type(CoStatus)").name(), "CoStatus");
        assert_eq!(parse("Optional(int)").name(), "Optional(int)");
        // a bare name is a variable of the signature, so it is only a type when
        // it says so
        assert_eq!(parse("CoStatus").name(), "CoStatus");
        let doc = parse("type(CoStatus)").to_doc_type_str().unwrap();
        assert!(doc.contains("DocType::Named(\"CoStatus\""), "{doc}");
        let doc = parse("Optional(int)").to_doc_type_str().unwrap();
        assert!(doc.contains("DocType::Named(\"Optional\""), "{doc}");
        // a type that is a type already does not take arguments
        assert!(
            parse_type_name("int(1)", Span::call_site(), "parameter", PARAM_TYPE_HINT).is_err()
        );
    }

    /// A name that is not a variable of the signature is a mistake, and the
    /// mistake is reported rather than guessed at.
    #[test]
    fn a_name_no_type_params_declares_is_rejected() {
        let err = err_of(parse_builtin_args(quote! {
            name = "f",
            params(who: bogus),
            returns(bogus)
        }));
        assert!(err.to_string().contains("not a type parameter"), "{err}");
        // declared, the same signature is what `as` is written as
        let ok = parse_builtin_args(quote! {
            name = "f",
            type_params(T),
            params(who: T),
            returns(T)
        })
        .expect("declared");
        assert_eq!(ok.type_params.len(), 1);
        assert_eq!(ok.type_params[0].name, "T");
    }

    #[test]
    fn fn_signature_parses_vararg() {
        let parse = |ty: &str| {
            parse_type_name(ty, Span::call_site(), "parameter", PARAM_TYPE_HINT)
                .unwrap()
                .name()
        };
        assert_eq!(parse("fn(int, ...)"), "fn(int, vararg)");
        assert_eq!(parse("fn(...)"), "fn(vararg)");
        assert_eq!(parse("fn(vararg)"), "fn(vararg)");
        assert_eq!(
            parse("fn(int, ...) -> int, ..."),
            "fn(int, vararg) -> int, vararg"
        );
        assert_eq!(parse("fn() -> ..."), "fn() -> vararg");
        assert_eq!(parse("fn(int)->bool"), "fn(int) -> bool");
    }

    #[test]
    fn fn_signature_vararg_must_be_last() {
        let err = |ty: &str| {
            err_of(parse_type_name(
                ty,
                Span::call_site(),
                "parameter",
                PARAM_TYPE_HINT,
            ))
        };
        assert!(
            err("fn(vararg, int)")
                .to_string()
                .contains("`vararg` must be the last parameter"),
            "params"
        );
        assert!(
            err("fn(int, ..., bool)")
                .to_string()
                .contains("`vararg` must be the last parameter"),
            "params"
        );
        assert!(
            err("fn(int) -> vararg, int")
                .to_string()
                .contains("`vararg` must be the last return"),
            "returns"
        );
        assert!(
            err("fn(int) -> int, ..., bool")
                .to_string()
                .contains("`vararg` must be the last return"),
            "returns"
        );
    }

    #[test]
    fn fn_vararg_reaches_doc_type() {
        let ty = parse_type_name(
            "fn(int, ...)->int, ...",
            Span::call_site(),
            "parameter",
            PARAM_TYPE_HINT,
        )
        .unwrap();
        let doc = ty.to_doc_type_str().unwrap();
        assert_eq!(doc.matches("DocType::VarArg").count(), 2, "{doc}");
        assert_eq!(
            const_type("fn(int, vararg)->...", Span::call_site())
                .unwrap()
                .name(),
            "fn(int, vararg) -> vararg"
        );
    }

    #[test]
    fn to_doc_type_str_renders_generics() {
        let parse = |ty: &str| {
            parse_type_name(ty, Span::call_site(), "parameter", PARAM_TYPE_HINT).unwrap()
        };
        let array = parse("array<int>").to_doc_type_str().unwrap();
        assert_eq!(
            array,
            "crate::duka_shared::docs::DocType::Array(&crate::duka_shared::docs::DocType::Base(crate::duka_shared::dtype::Type::Int))"
        );
        let table = parse("table<string,int>").to_doc_type_str().unwrap();
        assert!(table.contains("DocType::Table(Some(&"), "{table}");
        let func = parse("fn(int)->bool").to_doc_type_str().unwrap();
        assert!(func.contains("DocType::Function(&["), "{func}");
        let bare = parse("fn").to_doc_type_str().unwrap();
        assert!(bare.contains("Type::Function(None)"), "{bare}");
        let doc_union = parse("array<int>|nil").to_doc_type_str().unwrap();
        assert!(doc_union.contains("DocType::Union(&["), "{doc_union}");
    }

    #[test]
    fn const_type_allows_generic_containers() {
        assert!(const_type("array<int>", Span::call_site()).is_ok());
        assert!(const_type("table<string,int>", Span::call_site()).is_ok());
        assert!(const_type("fn(int)->int", Span::call_site()).is_ok());
        for bad in ["array<bytes>", "table<number, int>", "int|nil"] {
            let err = err_of(const_type(bad, Span::call_site()));
            assert!(
                err.to_string().contains("unsupported constant type"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn split_commas_keeps_angle_groups() {
        let parts = split_commas(quote! { x: table<string, int>, y: int });
        assert_eq!(parts.len(), 2, "{parts:#?}");
        let parts = split_commas(quote! { cb: fn(int, string)->bool, n: int });
        assert_eq!(parts.len(), 2, "{parts:#?}");
        let parts = split_commas(quote! { a: array<int> | nil, b: int });
        assert_eq!(parts.len(), 2, "{parts:#?}");
    }

    #[test]
    fn params_parse_generic_types() {
        let ps = parse_params(quote! { m: table<string, array<int>>, cb: fn(int)->bool }).unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].ty.as_deref(), Some("table<string,array<int>>"));
        assert_eq!(ps[1].ty.as_deref(), Some("fn(int)->bool"));

        let kind = ty_to_kind(ps[0].ty.as_deref().unwrap(), Span::call_site()).unwrap();
        assert_eq!(kind.helper, "take_table");
        assert_eq!(kind.meta.name(), "table<string, array<int>>");

        let kind = ty_to_kind(ps[1].ty.as_deref().unwrap(), Span::call_site()).unwrap();
        assert_eq!(kind.helper, "take_function");

        let union = ty_to_kind("array<int>|nil", Span::call_site()).unwrap();
        assert_eq!(union.helper, "take_union");
        assert_eq!(union.union_members.as_deref(), Some(&["ARR", "NIL"][..]));
    }
}
