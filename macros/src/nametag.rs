use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Fields, LitStr};

macro_rules! err {
    ($msg: expr, $span: expr) => {
        Error::new($span, $msg)
    };
}

pub fn generate_nametags(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let variants = if let Data::Enum(data_enum) = &input.data {
        &data_enum.variants
    } else {
        return err!("Expecting enum", name.span()).into_compile_error();
    };

    let arms = variants.iter().map(|variant| {
        let variant_name = &variant.ident;
        let fields = &variant.fields;

        let msg = (match get_nametag(&variant.attrs) {
            Ok(v) => v,
            Err(e) => return e.into_compile_error(),
        })
        .unwrap_or_else(|| format!("{}", variant_name).to_lowercase());

        match fields {
            Fields::Named(..) | Fields::Unnamed(..) => {
                quote! {
                    #name::#variant_name(..) => {
                        #msg
                    }
                }
            }
            Fields::Unit => {
                quote! {
                    #name::#variant_name => {
                        #msg
                    }
                }
            }
        }
    });

    quote! {
        impl #name {
            pub fn name(&self) -> &'static str {
                match self {
                    #(#arms),*
                }
            }
        }
        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.name())
            }
        }
    }
}

fn get_nametag(attrs: &[Attribute]) -> Result<Option<String>, Error> {
    for attr in attrs {
        if attr.path().is_ident("name") {
            let lit: LitStr = attr.parse_args()?;
            return Ok(Some(lit.value()));
        }
    }
    Ok(None)
}
