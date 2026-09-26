// 公用attribute解析�?

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
    PreserveNumber,
    VarArg,
    UserData,
    Union(Vec<ParamTypeName>),
}

#[derive(Debug)]
pub(crate) struct FnSignature {
    pub(crate) params: Vec<ParamTypeName>,
    pub(crate) returns: Vec<ParamTypeName>,
}

impl ParamTypeName {
    fn helper(&self) -> &'static str {
        match self {
            ParamTypeName::Int => "take_int",
            ParamTypeName::Num => "take_num",
            ParamTypeName::PreserveNumber => "take_number",
            ParamTypeName::String => "take_string",
            ParamTypeName::Bytes => "take_bytes",
            ParamTypeName::Bool => "take_bool",
            ParamTypeName::Array(..) => "take_array",
            ParamTypeName::Table(..) => "take_table",
            ParamTypeName::Function(..) => "take_function",
            ParamTypeName::Nil | ParamTypeName::Any | ParamTypeName::UserData => "take_any",
            ParamTypeName::VarArg => "take_many",
            ParamTypeName::Union(..) => "take_union",
        }
    }

    fn outer_ctype(&self) -> &'static str {
        match self {
            ParamTypeName::Int => "INT",
            ParamTypeName::Num | ParamTypeName::PreserveNumber => "NUM",
            ParamTypeName::String | ParamTypeName::Bytes => "STR",
            ParamTypeName::Bool => "BOO",
            ParamTypeName::Table(..) => "TAB",
            ParamTypeName::Array(..) => "ARR",
            ParamTypeName::Function(..) => "FUN",
            ParamTypeName::Nil => "NIL",
            _ => "ANY",
        }
    }

    pub(crate) fn name(&self) -> String {
        match self {
            ParamTypeName::String => "string".to_owned(),
            ParamTypeName::Int => "int".to_owned(),
            ParamTypeName::Num => "num".to_owned(),
            ParamTypeName::Bool => "bool".to_owned(),
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
                if sig.returns.is_empty() {
                    format!("fn({params})")
                } else {
                    let returns = sig
                        .returns
                        .iter()
                        .map(ParamTypeName::name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("fn({params}) -> {returns}")
                }
            }
            ParamTypeName::Array(None) => "array".to_owned(),
            ParamTypeName::Array(Some(inner)) => format!("array<{}>", inner.name()),
            ParamTypeName::Any => "any".to_owned(),
            ParamTypeName::Nil => "nil".to_owned(),
            ParamTypeName::Bytes => "bytes".to_owned(),
            ParamTypeName::PreserveNumber => "preserve_number".to_owned(),
            ParamTypeName::VarArg => "vararg".to_owned(),
            ParamTypeName::UserData => "userdata".to_owned(),
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
            ParamTypeName::PreserveNumber => format!("{base}::PreserveNumber"),
            ParamTypeName::Bytes => format!("{base}::Bytes"),
            ParamTypeName::String => format!("{base}::Base({ty}::String)"),
            ParamTypeName::Int => format!("{base}::Base({ty}::Int)"),
            ParamTypeName::Num => format!("{base}::Base({ty}::Float)"),
            ParamTypeName::Bool => format!("{base}::Base({ty}::Bool)"),
            ParamTypeName::Any | ParamTypeName::UserData => format!("{base}::Base({ty}::Any)"),
            ParamTypeName::Nil => format!("{base}::Base({ty}::Nil)"),
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
                let params = sig
                    .params
                    .iter()
                    .map(|p| p.to_doc_type_str())
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                let returns = sig
                    .returns
                    .iter()
                    .map(|r| r.to_doc_type_str())
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                format!("{base}::Function(&[{params}], &[{returns}])")
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
    let meta = parse_type_name(ty, span, "parameter", PARAM_TYPE_HINT)?;
    if let ParamTypeName::Union(inner) = &meta {
        let members = inner.iter().map(|i| i.outer_ctype()).collect();
        return Ok(ArgKind {
            helper: "take_union",
            meta,
            union_members: Some(members),
        });
    }
    let helper = meta.helper();
    Ok(ArgKind {
        helper,
        meta,
        union_members: None,
    })
}

const PARAM_TYPE_HINT: &str =
    "int, num, number, string, bytes, bool, array, table, fn, nil, any, or a union like `fn | nil`";
const CONST_TYPE_HINT: &str = "int, float, num, string, bool, nil, any, table, array, fn";

struct TypeParser {
    chars: Vec<char>,
    pos: usize,
    span: Span,
    label: &'static str,
    hint: &'static str,
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

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

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

    fn parse_prim(&mut self) -> Result<ParamTypeName> {
        self.skip_ws();
        if self.peek() == Some('*') {
            self.pos += 1;
            return Ok(ParamTypeName::Any);
        }
        let Some(word) = self.ident() else {
            return Err(self.err(format!(
                "expected a type name, expected one of: {}",
                self.hint
            )));
        };
        if word == "fn" {
            return self.parse_fn_sig();
        }
        if self.peek() == Some('(') {
            return Err(self.err(format!("type `{word}` does not take a function signature")));
        }
        let base = match word.as_str() {
            "int" => ParamTypeName::Int,
            "float" | "num" => ParamTypeName::Num,
            "number" | "preserve_number" => ParamTypeName::PreserveNumber,
            "string" | "str" => ParamTypeName::String,
            "bytes" => ParamTypeName::Bytes,
            "bool" => ParamTypeName::Bool,
            "nil" => ParamTypeName::Nil,
            "any" => ParamTypeName::Any,
            "func" | "function" => ParamTypeName::Function(None),
            "array" | "list" => ParamTypeName::Array(None),
            "table" => ParamTypeName::Table(None, None),
            _ => {
                return Err(self.err(format!(
                    "unsupported {} type `{word}`; expected one of: {}",
                    self.label, self.hint
                )));
            }
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

    fn parse_fn_sig(&mut self) -> Result<ParamTypeName> {
        if !self.eat('(') {
            return Ok(ParamTypeName::Function(None));
        }
        let mut params = vec![];
        if !self.eat(')') {
            loop {
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
        }))))
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
            meta: ParamTypeName::Table(None, None),
            union_members: None,
        }),
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
                "returns",
                "flags",
            ],
            AttrShape::Constant => &["type", "name", "doc", "example", "value", "flags"],
            AttrShape::Struct => &["name", "doc", "example"],
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

fn check_const_type(meta: &ParamTypeName, span: Span) -> Result<()> {
    match meta {
        ParamTypeName::String
        | ParamTypeName::Int
        | ParamTypeName::Num
        | ParamTypeName::Bool
        | ParamTypeName::Nil
        | ParamTypeName::Any => Ok(()),
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
        for bad in [
            "array<int",
            "array<>",
            "table<int>",
            "int<str>",
            "fn(",
            "bogus",
            "",
        ] {
            let err = err_of(parse_name(bad));
            assert!(!err.to_string().is_empty(), "{bad}");
        }
        let err = err_of(parse_name("bogus"));
        assert!(
            err.to_string().contains("unsupported parameter type"),
            "{err}"
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
