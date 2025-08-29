use proc_macro2::Span;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, LitStr};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

pub fn generate_errors(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let variants = if let Data::Enum(data_enum) = &input.data {
        &data_enum.variants
    } else {
        return err!("Expected enum", name.span()).into_compile_error();
    };

    let arms = variants.iter().map(|variant| {
        let variant_name = &variant.ident;
        let fields = &variant.fields;

        let msg = (match get_error_msg(&variant.attrs) {
            Ok(v) => v,
            Err(e) => return e.into_compile_error(),
        })
        .unwrap_or_else(|| format!("{}", variant_name));

        match fields {
            Fields::Named(fields) => {
                let field_names = fields.named.iter().map(|f| &f.ident);
                let field_names_2 = field_names.clone();
                quote! {
                    #name::#variant_name { #(#field_names_2),* } => {
                        write!(f, #msg, #(#field_names),*)
                    }
                }
            }
            Fields::Unnamed(fields) => {
                let field_patterns = (0..fields.unnamed.len())
                    .map(|i| Ident::new(&format!("_{}", i), Span::call_site()));
                let field_patterns_2 = field_patterns.clone();
                quote! {
                    #name::#variant_name( #(#field_patterns_2),* ) => {
                        write!(f, #msg, #(#field_patterns),*)
                    }
                }
            }
            Fields::Unit => {
                quote! {
                    #name::#variant_name => {
                        write!(f, #msg)
                    }
                }
            }
        }
    });

    let (impl_, ty_, where_) = &input.generics.split_for_impl();

    quote! {
        impl #impl_ std::fmt::Display for #name #ty_ #where_ {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    #(#arms),*
                }
            }
        }

        impl #impl_ std::error::Error for #name #ty_ #where_ {}
    }
}

fn get_error_msg(attrs: &[Attribute]) -> Result<Option<String>, Error> {
    for attr in attrs {
        if attr.path().is_ident("error") {
            let lit: LitStr = attr.parse_args()?;
            return Ok(Some(lit.value()));
        }
    }
    Ok(None)
}
