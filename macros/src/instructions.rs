use std::{collections::HashMap, u8};

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Error, Ident, Index, LitInt, Token, parse::Parse, punctuated::Punctuated};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

pub(crate) struct Instructions {
    name: Ident,
    mode: Vec<Mode>,
    flags: Vec<Ident>,
    ins_bits: u8,
    items: Vec<Instruction>,
}

struct Instruction {
    name: Ident,
    mode: Ident,
    flags: Vec<Ident>,
}

struct Mode {
    name: Ident,
    params: Vec<Param>,
}
struct Param {
    name: Ident,
    bits: u8,
    ty: ParamType,
}

enum ParamType {
    Bool,
    Signed,
    Unsigned,
}

impl Parse for Param {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse::<Ident>()?;
        let content;
        syn::bracketed!(content in input);

        if let Ok(lit) = content.parse::<LitInt>() {
            let bits = lit.base10_parse::<u8>()?;

            Ok(Self {
                name,
                bits,
                ty: if let Ok(id) = content.parse::<Ident>()
                    && id == "signed"
                {
                    ParamType::Signed
                } else {
                    ParamType::Unsigned
                },
            })
        } else if let Ok(id) = content.parse::<Ident>()
            && id == "bool"
        {
            Ok(Self {
                name,
                bits: 1,
                ty: ParamType::Bool,
            })
        } else {
            Err(err!("Unsupported bits pattern", name.span()))
        }
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
        if input.parse::<Ident>()?.ne("mode") {
            return Err(err!("Expecting mode keyword", Span::call_site()));
        }

        let content;
        syn::braced!(content in input);
        let mode = Punctuated::<Mode, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();

        if input.parse::<Ident>()?.ne("flags") {
            return Err(err!("Expecting flags keyword", Span::call_site()));
        }

        let content;
        syn::parenthesized!(content in input);
        let flags = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
            .into_iter()
            .collect();

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
            ins_bits: bits,
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

        Ok(Self { name, mode, flags })
    }
}

impl Instructions {
    pub fn generate(&self) -> proc_macro2::TokenStream {
        let def_name = &self.name;
        let len = self.items.len();
        let flags_len = self.flags.len();

        let decode_def_name = format_ident!("Decode{}", def_name);

        let flags = &self.flags;

        let mode_def_name = format_ident!("{}Mode", def_name);
        let modes = &self.mode;

        let modes_name = modes.iter().map(|m| &m.name);
        let modes_name_ = modes_name.clone();

        let ins_def_name = format_ident!("{}Name", def_name);
        let ins = &self.items;

        let ins_name = ins.iter().map(|i| &i.name);
        let ins_name_2 = ins_name.clone();

        let mut ins_to_name: Vec<TokenStream> = Vec::with_capacity(len);
        let mut ins_to_decoded: Vec<TokenStream> = Vec::with_capacity(len);
        let mut ins_to_mode: Vec<TokenStream> = Vec::with_capacity(len);
        let mut constructors: Vec<TokenStream> = Vec::with_capacity(len);

        let mut decoders: Vec<TokenStream> = Vec::with_capacity(len);

        let name_mask = 2u32.pow(self.ins_bits as u32) - 1;

        let mut flag_checker_mapper: HashMap<&Ident, Vec<TokenStream>> = {
            let mut map = HashMap::with_capacity(flags_len);
            flags.iter().for_each(|f| {
                map.insert(f, Vec::with_capacity(len));
            });
            map
        };
        let mut flag_checkers: Vec<TokenStream> = Vec::with_capacity(flags_len);

        for (i, inst) in ins.iter().enumerate() {
            let name = &inst.name;
            let mode = modes.iter().find(|m| m.name == inst.mode);
            if let Some(mode) = mode {
                let mut offset: u32 = self.ins_bits as u32;
                let params_part = mode.params.iter().map(|p| {
                    let name = &p.name;
                    let res = match p.ty {
                        ParamType::Signed => {
                            let sig_mask: u32 = 2u32.pow((p.bits - 1) as u32);
                            let num_mask: u32 = sig_mask - 1;
                            quote! { (if #name < 0 {
                                ((((-#name as u32) & #num_mask ) | #sig_mask)) << #offset
                            }  else {
                                (#name as u32 ) << #offset
                            } )}
                        }
                        _ => quote! { ( (#name as u32 ) << #offset )},
                    };
                    offset += p.bits as u32;
                    res
                });
                if mode.params.iter().map(|p| p.bits).sum::<u8>() != (32 - self.ins_bits) {
                    return err!("Invalid bits pattern", mode.name.span()).to_compile_error();
                }

                let params_name = mode.params.iter().map(|p| &p.name);
                let params_type = mode.params.iter().map(|p| match p.ty {
                    ParamType::Bool => quote! { bool },
                    ParamType::Signed => quote! { i32 },
                    ParamType::Unsigned => {
                        if p.bits > 8 {
                            quote! { u32 }
                        } else {
                            quote! {u8}
                        }
                    }
                });
                let params_type_2 = params_type.clone();
                decoders.push(quote! {
                    #name(#(#params_type_2),*)
                });

                constructors.push(quote! {
                    #[inline]
                    pub const fn #name(#(#params_name: #params_type),*) -> Self {
                        Self(#(#params_part as u32)|* | #ins_def_name::#name as u32)
                    }
                });

                for flag in &inst.flags {
                    if !flag_checker_mapper.contains_key(flag) {
                        return err!("Unknown flag", flag.span()).to_compile_error();
                    }
                    flag_checker_mapper.get_mut(flag).unwrap().push(quote! {
                        #ins_def_name :: #name
                    })
                }

                let index = Index::from(i);
                ins_to_name.push(quote! {
                    #index => #ins_def_name::#name
                });

                let mode_name = &mode.name;
                ins_to_mode.push(quote! {
                    #ins_def_name::#name => #mode_def_name::#mode_name
                });
                let mut offset: u32 = self.ins_bits as u32;
                let params_in = mode.params.iter().map(|p| {
                    let mask = 2u32.pow(p.bits as u32) - 1;
                    let res = match p.ty {
                        ParamType::Bool => quote! {(( self.0 >> #offset ) & #mask) != 0},
                        ParamType::Unsigned => {
                            if p.bits > 8 {
                                quote! { (( self.0 >> #offset ) & #mask) }
                            } else {
                                quote! { (( self.0 >> #offset ) & #mask) as u8 }
                            }
                        }
                        ParamType::Signed => {
                            let sig_mask = 2i32.pow((p.bits - 1) as u32);
                            let num_mask = sig_mask - 1;
                            quote! {((( self.0 >> #offset ) as i32) & #num_mask) *(
                                if ((self.0 >> #offset ) as i32) & #sig_mask != 0 {
                                    -1
                                } else {1 })
                            }
                        }
                    };
                    offset += p.bits as u32;
                    res
                });
                ins_to_decoded.push(quote! {
                    #ins_def_name::#name => #decode_def_name::#name(#(#params_in),*)
                });
            } else {
                return err!("Unknown mode", name.span()).to_compile_error();
            }
        }

        flag_checker_mapper.into_iter().for_each(|(flag, content)| {
            let fn_name = format_ident!("check_{}", flag);

            let true_arm = if content.len() == 0 {
                quote! {}
            } else {
                quote! {
                    #(#content)|* => true,
                }
            };

            flag_checkers.push(quote! {
                pub const fn #fn_name(&self) -> bool {
                    match self.name() {
                        #true_arm
                        _ => false,
                    }
                }
            });
        });

        quote! {
            #[doc = "Generated instruction type"]
            #[derive(Debug, Clone)]
            pub struct #def_name(u32);

            impl std::fmt::Display for #def_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", self.decode())
                }
            }

            #[derive(Debug)]
            pub enum #decode_def_name {
                #(#decoders),*
            }

            impl std::fmt::Display for #decode_def_name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{:?}", self)
                }
            }

            #[derive(Debug)]
            pub enum #mode_def_name {
                #(#modes_name_),*
            }

            #[derive(Debug)]
            pub enum #ins_def_name {
                #(#ins_name_2),*
            }

            #[allow(non_snake_case)]
            impl #def_name {
                #(#constructors)*

                pub fn new(code: u32) -> Self {
                    Self(code)
                }

                pub fn get(&self) -> u32 {
                    self.0
                }

                pub const fn name(&self) -> #ins_def_name {
                    match self.0 & #name_mask {
                        #(#ins_to_name),*,
                        _ => panic!("Invalid instruction")
                    }
                }
                #(#flag_checkers)*
                pub const fn mode(&self) -> #mode_def_name {
                    match self.name() {
                        #(#ins_to_mode),*
                    }
                }
                pub const fn decode(&self) -> #decode_def_name {
                    match self.name() {
                        #(#ins_to_decoded),*
                    }
                }
            }
        }
    }
}
