use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, Index, LitStr, Type, parse::Parse};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

pub fn generate_info(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let im_shy_dont_display_me_pls = has_attr(&input, "shy");
    let we_are_different = match get_attr::<Type>(&input, "idcard") {
        Ok(v) => v,
        Err(e) => return e.into_compile_error(),
    };

    let variants = if let Data::Enum(data_enum) = &input.data {
        &data_enum.variants
    } else {
        return err!("Only available for struct", name.span()).into_compile_error();
    };

    let from_disc_flag = variants.iter().all(|v| v.fields.is_empty());
    let mut name_arms: Vec<TokenStream> = vec![];
    let mut disc_arms: Vec<TokenStream> = vec![];
    let mut disc4name_arms: Vec<TokenStream> = vec![];
    let mut tag_list: HashMap<String, Vec<proc_macro2::TokenStream>> = HashMap::new();
    let mut from_disc_arms: Vec<TokenStream> = vec![];

    for (index, variant) in variants.iter().enumerate() {
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

        let pattern = gen_pattern(fields);

        name_arms.push(quote! {
            #name::#variant_name #pattern => {
                #name_msg
            }
        });

        let i = Index::from(index);
        disc_arms.push(quote! {
            #name::#variant_name #pattern => #i
        });
        disc4name_arms.push(quote! {
            #i => #name_msg
        });

        if from_disc_flag {
            from_disc_arms.push(quote! {
                #i => #name::#variant_name
            });
        }

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

    fn gen_pattern(fields: &Fields) -> proc_macro2::TokenStream {
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

    let (impl_, ty_, where_) = &input.generics.split_for_impl();

    let display = (!im_shy_dont_display_me_pls)
        .then(|| {
            quote! {
                impl #impl_ std::fmt::Display for #name #ty_ #where_ {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "{}", self.name())
                    }
                }
            }
        })
        // ok, i got you bro
        .unwrap_or_default();

    let from_disc = we_are_different
        .clone()
        .filter(|_| from_disc_flag)
        .map(|ty| {
            quote! {
                impl #impl_ #name #ty_ #where_ {
                    pub const fn from_disc(disc: #ty) -> Result<Self, &'static str> {
                        Ok(match disc {
                            #(#from_disc_arms),*,
                            _ => return Err("No such discriminant")
                        })
                    }
                }
            }
        })
        .unwrap_or_default();
    let disc = we_are_different
        .map(|ty| {
            quote! {
                impl #impl_ #name #ty_ #where_ {
                    pub const fn disc(&self) -> #ty {
                        match self {
                            #(#disc_arms),*
                        }
                    }
                    #[doc = "Get name of variant by its discriminant number"]
                    pub const fn disc2name(disc: #ty) -> &'static str {
                        match disc {
                            #(#disc4name_arms),*,
                            _ => panic!("No such discriminant")
                        }
                    }
                }
            }
        })
        .unwrap_or_default();

    let nametags = quote! {
        impl #impl_ #name #ty_ #where_ {
            pub const fn name(&self) -> &'static str {
                match self {
                    #(#name_arms),*
                }
            }
            #(#tag_funcs)*
        }
    };

    quote! {
        #nametags
        #disc
        #from_disc
        #display
    }
}

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
fn has_attr(target: &DeriveInput, ident: &str) -> bool {
    target.attrs.iter().any(|attr| attr.path().is_ident(ident))
}

fn get_attr<T: Parse>(target: &DeriveInput, ident: &str) -> Result<Option<T>, Error> {
    target
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident(ident))
        .map(|attr| attr.parse_args::<T>())
        .transpose()
}
