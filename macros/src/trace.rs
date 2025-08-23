use quote::quote;
use syn::{Data, DeriveInput};

pub fn generate_trace(input: DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let body = match &input.data {
        Data::Struct(data) => {
            let fields = data.fields.iter().map(|field| {
                let name = field.ident.clone().unwrap();
                quote! {
                    self.#name.trace(tracer);
                }
            });
            quote! {
                fn trace(&self, tracer: &mut dyn FnMut(&GcObject)) {
                    #(#fields)*
                }
            }
        }
        _ => quote! {
            fn trace(&self, tracer: &mut dyn FnMut(&GcObject)) {
                // empty
            }
        },
    };

    quote! {
        impl Trace for #name {
            #body
        }
    }
}
