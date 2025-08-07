use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, LitStr};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

pub fn generate_info(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    // let val_type = match get_type(&input.attrs) {
    //     Ok(v) => v,
    //     Err(e) => return e.into_compile_error(),
    // };

    let variants = if let Data::Enum(data_enum) = &input.data {
        &data_enum.variants
    } else {
        return err!("Expecting an enum", name.span()).into_compile_error();
    };

    let mut name_arms: Vec<TokenStream> = vec![];
    let mut tag_list: HashMap<String, Vec<proc_macro2::TokenStream>> = HashMap::new();

    for variant in variants {
        let variant_name = &variant.ident;
        let fields = &variant.fields;

        let name_msg = (match get_name(&variant.attrs) {
            Ok(v) => v,
            Err(e) => return e.into_compile_error(),
        })
        .unwrap_or_else(|| format!("{}", variant_name).to_lowercase());

        let tags = match get_tags(&variant.attrs) {
            Ok(v) => v,
            Err(e) => return e.into_compile_error(),
        };

        let pattern = get_pattern(fields);
        let name_arm = quote! {
            #name::#variant_name #pattern => {
                #name_msg
            }
        };
        name_arms.push(name_arm);

        for tag in tags {
            let arm = quote! { #name::#variant_name #pattern };
            if let Some(vec) = tag_list.get_mut(&tag) {
                vec.push(arm);
            } else {
                tag_list.insert(tag, vec![arm]);
            }
        }
    }

    let tag_funcs = tag_list.into_iter().map(|(k, v)| {
        let func_name = format_ident!("is_{}", k);

        quote! {
            #[inline(always)]
            pub const fn #func_name(&self) -> bool {
                matches!(self, #(#v)|*)
            }
        }
    });

    fn get_pattern(fields: &Fields) -> proc_macro2::TokenStream {
        if matches!(fields, Fields::Unit) {
            quote! {}
        } else if fields.is_empty() {
            quote! { () }
        } else {
            quote! {
                (..)
            }
        }
    }

    quote! {
        impl #name {
            pub const fn name(&self) -> &'static str {
                match self {
                    #(#name_arms),*
                }
            }
            #(#tag_funcs)*
        }
        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }
    }
}

// fn get_enum_variant_name(str: &String) -> Ident {
//     format_ident!("{}{}", str[..1].to_uppercase(), str[1..])
// }
fn get_name(attrs: &[Attribute]) -> Result<Option<String>, Error> {
    for attr in attrs {
        if attr.path().is_ident("name") {
            let lit: LitStr = attr.parse_args()?;
            return Ok(Some(lit.value()));
        }
    }
    Ok(None)
}
fn get_tags(attrs: &[Attribute]) -> Result<Vec<String>, Error> {
    let mut res = vec![];

    for attr in attrs {
        if attr.path().is_ident("tag") {
            let lit: Ident = attr.parse_args()?;
            res.push(lit.to_string());
        }
    }

    Ok(res)
}
// fn get_type(attrs: &[Attribute]) -> Result<Option<Type>, Error> {
//     for attr in attrs {
//         if attr.path().is_ident("val") {
//             let ty: Type = attr.parse_args()?;
//             return Ok(Some(ty));
//         }
//     }
//     Ok(None)
// }
