use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{
    Error, ExprClosure, Ident, Index, LitInt, Path, Token, parse::Parse, punctuated::Punctuated,
};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

mod kw {
    use syn::custom_keyword;
    custom_keyword!(mode);
    custom_keyword!(flags);
    custom_keyword!(signed);
    custom_keyword!(address);
    custom_keyword!(bool);
}

pub(crate) struct Instructions {
    name: Ident,
    mode: Vec<Mode>,
    flags: Vec<Ident>,
    name_bits_used: u8,
    items: Vec<Instruction>,
}

struct Instruction {
    name: Ident,
    mode: Ident,
    flags: Vec<Ident>,
    display: Option<ExprClosure>,
}

struct Mode {
    name: Ident,
    params: Vec<Param>,
}

#[derive(PartialEq, Clone)]
struct Param {
    name: Ident,
    bits_used: u8,
    param_type: ParamType,
}

#[derive(PartialEq, Clone)]
enum ParamType {
    Bool,
    Signed,
    Unsigned,
    Address,
    Enum(Path),
}

impl Parse for Param {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse::<Ident>()?;
        let content;
        syn::bracketed!(content in input);
        Ok(if let Ok(lit) = content.parse::<LitInt>() {
            let bits = lit.base10_parse::<u8>()?;
            Self {
                name,
                bits_used: bits,
                param_type: content
                    .parse::<kw::signed>()
                    .is_ok()
                    .then_some(ParamType::Signed)
                    .unwrap_or(ParamType::Unsigned),
            }
        } else if content.parse::<kw::address>().is_ok() {
            Self {
                name,
                bits_used: 8,
                param_type: ParamType::Address,
            }
        } else if content.parse::<Token![enum]>().is_ok() {
            let enum_name = content.parse::<Path>()?;
            let bits;
            syn::bracketed!(bits in content);
            let bits_used = bits.parse::<LitInt>()?.base10_parse::<u8>()?;
            Self {
                name,
                bits_used,
                param_type: ParamType::Enum(enum_name),
            }
        } else if content.parse::<kw::bool>().is_ok() {
            Self {
                name,
                bits_used: 1,
                param_type: ParamType::Bool,
            }
        } else {
            return Err(err!("Unsupported bits pattern", name.span()));
        })
    }
}

impl Parse for Mode {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse::<Ident>()?;
        let content;
        syn::parenthesized!(content in input);
        let params = Punctuated::<Param, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        Ok(Self { name, params })
    }
}

impl Parse for Instructions {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<kw::mode>()?;
        let content;
        syn::braced!(content in input);
        let mode = Punctuated::<Mode, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        let flags = if input.parse::<kw::flags>().is_ok() {
            let content;
            syn::parenthesized!(content in input);
            Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
                .into_iter()
                .collect()
        } else {
            vec![]
        };
        input.parse::<Token![impl]>()?;
        let content;
        syn::bracketed!(content in input);
        let bits = content.parse::<LitInt>()?.base10_parse::<u8>()?;
        let content;
        syn::braced!(content in input);
        let items = Punctuated::<Instruction, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        input.parse::<Token![as]>()?;
        let name = input.parse::<Ident>()?;
        Ok(Self {
            name,
            mode,
            name_bits_used: bits,
            flags,
            items,
        })
    }
}

impl Parse for Instruction {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse::<Ident>()?;
        let content;
        syn::bracketed!(content in input);
        let mode = content.parse::<Ident>()?;
        let content;
        syn::parenthesized!(content in input);
        let flags = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();
        let display = input
            .parse::<Token![->]>()
            .ok()
            .and_then(|_| input.parse::<ExprClosure>().ok());
        Ok(Self {
            name,
            mode,
            flags,
            display,
        })
    }
}

impl Instructions {
    pub fn generate(&self) -> proc_macro2::TokenStream {
        let as_name = &self.name;
        let item_len = self.items.len();
        let decode_define_name = format_ident!("Decode{}", as_name);
        let flags = &self.flags;
        let mode_define_name = format_ident!("{}Mode", as_name);
        let modes = &self.mode;
        let modes_name: Vec<&Ident> = modes.iter().map(|m| &m.name).collect();
        let define_name = format_ident!("{}Name", as_name);
        let items = &self.items;
        let item_names: Vec<&Ident> = items.iter().map(|i| &i.name).collect();

        let mut name_mapper = Vec::with_capacity(item_len);
        let mut decode_mapper = Vec::with_capacity(item_len);
        let mut mode_mapper = Vec::with_capacity(item_len);
        let mut constructors = Vec::with_capacity(item_len);
        let mut decode_items = Vec::with_capacity(item_len);
        let mut decode_display = Vec::with_capacity(item_len);
        let name_mask = (1u32 << self.name_bits_used) - 1;

        let mut type_alias_map: HashMap<(Path, u8, bool), TokenStream> = HashMap::new();
        let mut flag_func_map: HashMap<&Ident, Vec<TokenStream>> =
            flags.iter().map(|f| (f, Vec::new())).collect();

        for (index, item) in items.iter().enumerate() {
            let name = &item.name;
            let mode = modes.iter().find(|m| m.name == item.mode);
            let Some(Mode {
                name: mode_name,
                params,
            }) = mode
            else {
                return err!("Unknown mode", name.span()).to_compile_error();
            };
            let params_bits_count = params.iter().map(|p| p.bits_used).sum::<u8>();
            if params_bits_count > (u32::BITS as u8 - self.name_bits_used) {
                return err!("Invalid bits pattern", mode_name.span()).to_compile_error();
            }

            constructors.push(gen_constructor(
                self.name_bits_used,
                params,
                &define_name,
                name,
            ));
            decode_items.push(gen_decode_items(name, params));
            decode_display.push(gen_decode_display(params, name, &item.display));

            for flag in &item.flags {
                match flag_func_map.get_mut(flag) {
                    Some(v) => v.push(quote! { #define_name :: #name }),
                    None => return err!("Unknown flag", flag.span()).to_compile_error(),
                }
            }

            type_alias_map.extend(gen_type_alias(params));
            name_mapper.push(gen_name_mapper(index, &define_name, name));
            mode_mapper.push(gen_mode_mapper(
                &define_name,
                name,
                &mode_define_name,
                mode_name,
            ));
            decode_mapper.push(gen_decode_mapper(
                self.name_bits_used,
                params,
                &define_name,
                name,
                &decode_define_name,
            ));
        }

        let flag_checkers = flag_func_map
            .iter()
            .map(|(flag, targets)| gen_flag_func(flag, targets));
        let type_def = type_alias_map.values();

        let type_converter = type_alias_map.keys().map(|(path, bits, signed)| {
            let func_name = format_ident!(
                "Make{}",
                path.get_ident().cloned().unwrap_or_else(|| {
                    format_ident!("{}{}", bits, if *signed { "Signed" } else { "Unsigned" })
                })
            );
            let body = if *signed {
                let min: isize = -(1 << (bits - 1));
                let max: isize = (1 << (bits - 1)) - 1;
                quote! {
                    const MIN: isize = #min;
                    const MAX: isize = #max;
                    (MIN..=MAX).contains(&num).then_some(num as #path)
                }
            } else {
                let max: usize = (1 << bits) - 1;
                quote! {
                    const MAX: usize = #max;
                    (num <= MAX).then_some(num as #path)
                }
            };
            let input_type = if *signed {
                quote! { isize }
            } else {
                quote! { usize }
            };
            quote! {
                #[inline(always)]
                pub fn #func_name(num: #input_type) -> Option<#path> {
                    #body
                }
            }
        });

        quote! {
            #[doc = "Generated instruction type"]
            #[derive(Debug, Clone, PartialEq)]
            pub struct #as_name(u32);

            impl std::fmt::Display for #as_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    match self.decode() {
                        Ok(r) => write!(f, "{r}"),
                        Err(r) => write!(f, "{r}"),
                    }
                }
            }

            #[derive(Debug, Clone, PartialEq)]
            pub enum #decode_define_name {
                #(#decode_items),*
            }

            #[allow(non_snake_case)]
            #[allow(unused_variables)]
            impl std::fmt::Display for #decode_define_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", match self {
                        #(#decode_display),*
                    })
                }
            }

            #(#type_def)*

            #[derive(Debug, Clone, PartialEq)]
            pub enum #mode_define_name {
                #(#modes_name),*
            }

            #[derive(Debug, Clone, PartialEq)]
            pub enum #define_name {
                #(#item_names),*
            }

            #[allow(non_snake_case)]
            #[allow(unused_variables)]
            impl #as_name {
                #(#constructors)*
                #(#type_converter)*

                pub const NAME_MASK: u32 = #name_mask;

                #[inline(always)]
                pub fn from_raw(raw: u32) -> Self {
                    Self(raw)
                }

                #[inline(always)]
                pub fn raw(&self) -> u32 {
                    self.0
                }

                #[inline(always)]
                pub const fn validate(raw: u32) -> bool {
                    (raw & Self::NAME_MASK) < (#item_len as u32)
                }

                pub fn name(&self) -> Result<#define_name, &'static str> {
                    Ok(match self.0 & Self::NAME_MASK {
                        #(#name_mapper),*,
                        _ => return Err("Invalid instruction")
                    })
                }

                #(#flag_checkers)*

                pub fn mode(&self) -> Result<#mode_define_name, &'static str> {
                    Ok(match self.name()? {
                        #(#mode_mapper),*
                    })
                }

                pub fn decode(&self) -> Result<#decode_define_name, &'static str> {
                    Ok(match self.name()? {
                        #(#decode_mapper),*
                    })
                }
            }
        }
    }
}

fn gen_encode_params(param: &Param, offset: u32) -> proc_macro2::TokenStream {
    let name = &param.name;
    let ty = adapt_btype(param.bits_used, false);
    let for_enum = if let ParamType::Enum(..) = param.param_type {
        quote! {
            let #name: #ty = #name.try_into().expect("Failed to encode enum type");
        }
    } else {
        quote! {}
    };
    let mask = (1u32 << param.bits_used) - 1;
    quote! {{
        #for_enum
        (((#name as u32) & #mask) << #offset)
    }}
}

fn gen_decode_params(param: &Param, offset: u32) -> proc_macro2::TokenStream {
    let mask = (1u32 << param.bits_used as u32) - 1;
    if let ParamType::Enum(p) = &param.param_type {
        let ty = adapt_btype(param.bits_used, false);
        quote! {{
            let v = ((self.0 >> #offset) & #mask) as #ty;
            #p::try_from(v).map_err(|_| "Failed to convert to enum")?
        }}
    } else {
        let convert = match param.param_type {
            ParamType::Bool => quote! { != 0 },
            ParamType::Address => {
                let ty = get_address_type_name(param.name.span());
                quote! { as #ty }
            }
            ParamType::Signed => {
                let ty = adapt_btype(param.bits_used, true);
                let shift = u32::BITS - param.bits_used as u32;
                return quote! {
                    ((((self.0 >> #offset) & #mask) << #shift) as i32 >> #shift) as #ty
                };
            }
            ParamType::Unsigned => {
                let ty = adapt_btype(param.bits_used, false);
                quote! { as #ty }
            }
            _ => unreachable!(),
        };
        quote! {
            ((self.0 >> #offset) & #mask) #convert
        }
    }
}

fn gen_flag_func(flag: &Ident, targets: &[TokenStream]) -> proc_macro2::TokenStream {
    let fn_name = format_ident!("check_{}", flag);
    let matches = if targets.is_empty() {
        quote! { false }
    } else {
        quote! { matches!(self.name()?, #(#targets)|*) }
    };
    quote! {
        pub fn #fn_name(&self) -> Result<bool, &'static str> {
            Ok(#matches)
        }
    }
}

fn gen_type_alias(params: &[Param]) -> Vec<((Path, u8, bool), proc_macro2::TokenStream)> {
    params
        .iter()
        .filter(|p| !matches!(p.param_type, ParamType::Bool | ParamType::Enum(..)))
        .map(|p| {
            let path = get_type_path(&p.param_type, p.bits_used, p.name.span());
            let type_name = get_type(&p.param_type, p.bits_used);
            (
                (
                    path.clone(),
                    p.bits_used,
                    matches!(p.param_type, ParamType::Signed),
                ),
                quote! { pub type #path = #type_name; },
            )
        })
        .collect()
}

fn gen_decode_items(variant_name: &Ident, params: &[Param]) -> proc_macro2::TokenStream {
    let params_type = params
        .iter()
        .map(|p| get_type_path(&p.param_type, p.bits_used, p.name.span()));
    quote! { #variant_name(#(#params_type),*) }
}

fn gen_constructor(
    start_bits: u8,
    params: &[Param],
    def_name: &Ident,
    variant_name: &Ident,
) -> proc_macro2::TokenStream {
    let mut offset = start_bits as u32;
    let params_decoding = params.iter().map(|p| {
        let result = gen_encode_params(p, offset);
        offset += p.bits_used as u32;
        result
    });
    let (params_name, params_type): (Vec<_>, Vec<_>) = params
        .iter()
        .map(|p| {
            (
                &p.name,
                get_type_path(&p.param_type, p.bits_used, p.name.span()),
            )
        })
        .unzip();

    let constructor = if params.is_empty() {
        quote! { #def_name::#variant_name as u32 }
    } else {
        quote! { #(#params_decoding)|* | #def_name::#variant_name as u32 }
    };
    quote! {
        #[inline]
        pub fn #variant_name(#(#params_name: #params_type),*) -> Self {
            Self(#constructor)
        }
    }
}

fn gen_decode_display(
    params: &[Param],
    variant_name: &Ident,
    cl: &Option<ExprClosure>,
) -> proc_macro2::TokenStream {
    let params_name = params.iter().map(|p| &p.name);
    let pats = quote! { (#(#params_name),*) };
    let display = if let Some(c) = cl {
        quote! { (#c)#pats }
    } else {
        quote! { format!("{:?}", self) }
    };
    quote! {
        Self::#variant_name #pats => {
            #display
        }
    }
}

fn gen_decode_mapper(
    start_bits: u8,
    params: &[Param],
    def_name: &Ident,
    variant_name: &Ident,
    decode_def_name: &Ident,
) -> proc_macro2::TokenStream {
    let mut offset = start_bits as u32;
    let params_decoding = params.iter().map(|p| {
        let result = gen_decode_params(p, offset);
        offset += p.bits_used as u32;
        result
    });
    quote! {
        #def_name::#variant_name => #decode_def_name::#variant_name(#(#params_decoding),*)
    }
}

fn gen_mode_mapper(
    def_name: &Ident,
    variant_name: &Ident,
    mode_def_name: &Ident,
    mode_name: &Ident,
) -> proc_macro2::TokenStream {
    quote! { #def_name::#variant_name => #mode_def_name::#mode_name }
}

fn gen_name_mapper(i: usize, def_name: &Ident, variant_name: &Ident) -> proc_macro2::TokenStream {
    let index = Index::from(i);
    quote! { #index => #def_name::#variant_name }
}

fn adapt_btype(bits: u8, signed: bool) -> proc_macro2::TokenStream {
    match bits {
        ..=8 => {
            if signed {
                quote!(i8)
            } else {
                quote!(u8)
            }
        }
        9..=16 => {
            if signed {
                quote!(i16)
            } else {
                quote!(u16)
            }
        }
        _ => {
            if signed {
                quote!(i32)
            } else {
                quote!(u32)
            }
        }
    }
}

fn get_type_path(ty: &ParamType, bits: u8, span: Span) -> Path {
    match ty {
        ParamType::Bool => Path::from(Ident::new("bool", span)),
        ParamType::Signed => Path::from(Ident::new(&format!("SignedBits{}", bits), span)),
        ParamType::Unsigned => Path::from(Ident::new(&format!("Bits{}", bits), span)),
        ParamType::Address => Path::from(get_address_type_name(span)),
        ParamType::Enum(id) => id.clone(),
    }
}

fn get_type(ty: &ParamType, bits: u8) -> proc_macro2::TokenStream {
    match ty {
        ParamType::Bool => quote! { bool },
        ParamType::Address => quote! { u8 },
        ParamType::Enum(id) => quote! { #id },
        _ => adapt_btype(bits, matches!(ty, ParamType::Signed)),
    }
}

const ADDRESS_NAME: &str = "Address";
#[inline]
fn get_address_type_name(span: Span) -> syn::Ident {
    Ident::new(ADDRESS_NAME, span)
}
