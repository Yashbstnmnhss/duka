use std::collections::HashMap;

use quote::quote;
use syn::{Ident, Token, Type, parse::Parse, punctuated::Punctuated};

pub(crate) struct Ops {
    name: Ident,
    token_type: Type,
    op_type: Type,
    output_type: Ident,
    ops: Vec<OpLevel>,
}
struct OpLevel {
    pub map: HashMap<Ident, (Ident, bool)>,
}

struct Op {
    pub tk: Ident,
    pub op: Ident,
    pub right: bool,
}

impl Parse for Op {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let tk = input.parse::<Ident>()?;
        let op = if input.parse::<Token![=>]>().is_ok() {
            input.parse::<Ident>()?
        } else {
            tk.clone()
        };
        let right = input.parse::<Ident>().is_ok_and(|i| i == "right");
        Ok(Self { tk, op, right })
    }
}

impl Parse for OpLevel {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let map: HashMap<_, _> = Punctuated::<Op, Token![,]>::parse_separated_nonempty(input)?
            .into_iter()
            .map(|i| (i.tk, (i.op, i.right)))
            .collect();
        Ok(Self { map })
    }
}

impl Parse for Ops {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input.parse::<Token![as]>()?;
        let name = input.parse::<Ident>()?;
        input.parse::<Token![type]>()?;
        let token_type = input.parse::<Type>()?;
        input.parse::<Token![->]>()?;
        let op_type = input.parse::<Type>()?;
        input.parse::<Token![=]>()?;
        let output_type = input.parse::<Ident>()?;
        input.parse::<Token![:]>()?;
        let ops: Vec<_> = Punctuated::<OpLevel, Token![;]>::parse_separated_nonempty(input)?
            .into_iter()
            .collect();

        let _ = input.parse::<Ident>(); //递增

        Ok(Self {
            ops,
            name,
            token_type,
            op_type,
            output_type,
        })
    }
}

impl Ops {
    pub fn generate(&self) -> proc_macro2::TokenStream {
        let Self {
            name,
            token_type,
            op_type,
            ops,
            output_type,
        } = self;

        let mut offset = 1;
        let groups = ops.iter().enumerate().map(|(i, level)| {
            let i = i + offset;
            let group = level.map.iter().map(|(tk, (op, right))| {
                let l = if *right {
                    offset += 1;
                    i + 1
                } else {
                    i
                } as u8;
                let r = i as u8;
                quote! {
                    #token_type::#tk => (#op_type::#op, (#l, #r))
                }
            });
            quote! {
                #(#group),*
            }
        });

        quote! {
            pub type #output_type = (#op_type, (u8, u8));
            #[inline]
            pub fn #name(tk: &#token_type) -> Option<#output_type> {
                Some(match tk {
                    #(#groups),*,
                    _ => return None
                })
            }
        }
    }
}
