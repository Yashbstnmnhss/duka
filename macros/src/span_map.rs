use quote::{ToTokens, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, FieldsNamed, FieldsUnnamed, Index, Result, Token,
    Variant, punctuated::Punctuated,
};

use crate::attr::or_compile_error;

pub fn generate_span_map(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = input.ident;
    let body = if check_ignore(&input.attrs) {
        quote! {}
    } else {
        match input.data {
            Data::Enum(e) => or_compile_error!(gen_enum(e.variants)),
            Data::Struct(s) => gen_struct(s.fields),
            Data::Union(_) => quote! {},
        }
    };
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param
            .bounds
            .push(syn::parse_quote!(crate::lexer::span_mut::SpanMut));
    }
    let (impl_, ty_, where_) = generics.split_for_impl();
    quote! {
        impl #impl_ crate::lexer::span_mut::SpanMut for #name #ty_ #where_ {
            fn span_mut(&mut self, f: &mut dyn FnMut(&mut duka_shared::errors::Span)) {
                #body
            }
        }
    }
}

fn field_call<T: ToTokens>(name: T, has_self: bool) -> proc_macro2::TokenStream {
    if has_self {
        quote! { self.#name.span_mut(f); }
    } else {
        quote! { #name.span_mut(f); }
    }
}

fn check_ignore(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("span_ignore"))
}

fn gen_enum(variants: Punctuated<Variant, Token![,]>) -> Result<proc_macro2::TokenStream> {
    let arms = variants
        .into_iter()
        .filter(|variant| !check_ignore(&variant.attrs))
        .map(|variant| -> Result<proc_macro2::TokenStream> {
            let variant_name = variant.ident;
            match variant.fields {
                Fields::Named(FieldsNamed { named, .. }) => {
                    let fields: Vec<_> = named
                        .clone()
                        .into_iter()
                        .filter(|field| !check_ignore(&field.attrs))
                        .map(|field| field.ident.unwrap())
                        .collect();
                    let all: Vec<_> = named
                        .into_iter()
                        .map(|field| field.ident.unwrap())
                        .collect();
                    let calls = fields.iter().map(|ident| quote! { #ident.span_mut(f); });
                    Ok(quote! {
                        Self::#variant_name { #(#all),* } => { #(#calls)* }
                    })
                }
                Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                    let count = unnamed.len();
                    let bindings: Vec<_> = (0..count)
                        .map(|index| quote::format_ident!("__span_{}", index))
                        .collect();
                    let calls = bindings
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !check_ignore(&unnamed[*index].attrs))
                        .map(|(_, binding)| quote! { #binding.span_mut(f); });
                    Ok(quote! {
                        Self::#variant_name( #(#bindings),* ) => { #(#calls)* }
                    })
                }
                Fields::Unit => Ok(quote! {
                    Self::#variant_name => {}
                }),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(quote! {
        match self {
            #(#arms)*
        }
    })
}

fn gen_struct(fields: Fields) -> proc_macro2::TokenStream {
    let calls = match fields {
        Fields::Named(FieldsNamed { named, .. }) => named
            .into_iter()
            .filter(|field| !check_ignore(&field.attrs))
            .filter_map(|field| field.ident.map(|ident| field_call(ident, true)))
            .collect::<Vec<_>>(),
        Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => unnamed
            .into_iter()
            .enumerate()
            .filter(|(_, field)| !check_ignore(&field.attrs))
            .map(|(index, _)| field_call(Index::from(index), true))
            .collect::<Vec<_>>(),
        Fields::Unit => vec![],
    };
    quote! { #(#calls)* }
}
