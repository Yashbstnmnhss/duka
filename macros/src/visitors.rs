use quote::quote;
use syn::{Data, DeriveInput};

pub fn generate_visitors(input: DeriveInput, mutable: bool) -> proc_macro2::TokenStream {
    let name = input.ident;
    match input.data {
        Data::Enum(e) => {}
        Data::Struct(s) => {}
        _ => unimplemented!(),
    }

    if mutable {
        quote! {
            impl VisitMut for #name {
                fn visit_mut<V: VisitorMut>(&mut self, visitors: &mut V) {

                }
            }
        }
    } else {
        quote! {
            impl Visit for #name {
                fn visit<V: Visitor>(&self, visitors: &mut V) {

                }
            }
        }
    }
}
