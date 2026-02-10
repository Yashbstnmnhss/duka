use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Fields, FieldsNamed, FieldsUnnamed, Ident, Index, Token, Variant,
    parse::Parse, punctuated::Punctuated,
};

enum VisitType {
    Expr,
    Stmt,
    None,
}

pub fn generate_visitors(input: DeriveInput, mutable: bool) -> proc_macro2::TokenStream {
    let name = input.ident;
    let self_type = get_self_type(&input.attrs);
    let codes = if check_ignore(&input.attrs) {
        quote! {}
    } else {
        match input.data {
            Data::Enum(e) => gen_enum(e.variants, mutable, self_type),
            Data::Struct(s) => gen_struct(s.fields, mutable, self_type),
            _ => unimplemented!(),
        }
    };

    let (impl_, ty_, where_) = &input.generics.split_for_impl();

    macro_rules! constant {
        ($n: ident = $p: literal) => {
            let $n: syn::Path = syn::parse_str($p).unwrap();
        };
    }

    constant!(visit_path = "Visit");
    constant!(visit_mut_path = "VisitMut");
    constant!(visitor_path = "Visitor");
    constant!(visitor_mut_path = "VisitorMut");

    if mutable {
        quote! {
            impl #impl_ #visit_mut_path for #name #ty_ #where_ {
                fn visit_mut<V: #visitor_mut_path>(&mut self, visitor: &mut V) {
                    #codes
                }
            }
        }
    } else {
        quote! {
            impl #impl_ #visit_path for #name #ty_ #where_ {
                fn visit<V: #visitor_path>(&self, visitor: &mut V) {
                    #codes
                }
            }
        }
    }
}

fn gen_prop_call<T: ToTokens>(
    prop_name: T,
    mutable: bool,
    has_self: bool,
) -> proc_macro2::TokenStream {
    let self_ = if has_self {
        quote! {self.}
    } else {
        quote! {}
    };
    if mutable {
        quote! {
            #self_ #prop_name.visit_mut(visitor);
        }
    } else {
        quote! {
            #self_ #prop_name.visit(visitor);
        }
    }
}
fn gen_self_call(self_type: VisitType) -> proc_macro2::TokenStream {
    match self_type {
        VisitType::Expr => quote! {
            visitor.visit_expr(self);
        },
        VisitType::Stmt => quote! {
            visitor.visit_stmt(self);
        },
        VisitType::None => quote! {},
    }
}
fn gen_block_call(
    block: Option<Ident>,
    inner: proc_macro2::TokenStream,
    mutable: bool,
) -> proc_macro2::TokenStream {
    if mutable {
        return if block
            .is_some() { {
                quote! {
                    visitor.visit_block(true);
                    #inner
                    visitor.visit_block(false);
                }
            } } else { inner };
    }

    let Some(block_name) = block else {
        return inner;
    };

    let block_func = format_ident!("visit_{}_block", block_name);
    quote! {
        visitor.#block_func(self, true);
        #inner
        visitor.#block_func(self, false);
    }
}

fn gen_enum(
    variants: Punctuated<Variant, Token![,]>,
    mutable: bool,
    self_type: VisitType,
) -> proc_macro2::TokenStream {
    let arms = variants
        .into_iter()
        .filter(|variant| !check_ignore(&variant.attrs))
        .map(|variant| {
            let has_pattern = !matches!(variant.fields, Fields::Unit);
            let names: Vec<_> = match variant.fields {
                Fields::Named(FieldsNamed {
                    brace_token: _,
                    named,
                }) => named
                    .into_iter()
                    .map(|f| {
                        (
                            (!check_ignore(&f.attrs)).then_some(f.ident.unwrap()),
                            get_block(&f.attrs),
                        )
                    })
                    .collect(),
                Fields::Unnamed(FieldsUnnamed {
                    paren_token: _,
                    unnamed,
                }) => unnamed
                    .into_iter()
                    .enumerate()
                    .map(|(i, f)| {
                        (
                            (!check_ignore(&f.attrs)).then_some(format_ident!("_{}", i)),
                            get_block(&f.attrs),
                        )
                    })
                    .collect(),
                Fields::Unit => vec![],
            };
            let name = variant.ident;

            let vars: Vec<_> = names
                .clone()
                .into_iter()
                .map(|(o, _)| o.unwrap_or(Ident::new("_", Span::call_site())))
                .collect();
            let calls = names
                .into_iter()
                .filter_map(|(o, f)| o.map(|o| (o, f)))
                .map(|(i, block)| {
                    let inner = gen_prop_call(i, mutable, false);
                    gen_block_call(block, inner, mutable)
                });

            let pattern = if has_pattern {
                quote! {(#(#vars),*)}
            } else {
                quote! {}
            };
            quote! {
                Self::#name #pattern => {
                    #(#calls)*
                }
            }
        });

    let self_call = gen_self_call(self_type);

    quote! {
        match self {
            #(#arms)*
            _ => {}
        }
        #self_call
    }
}
fn gen_struct(fields: Fields, mutable: bool, self_type: VisitType) -> proc_macro2::TokenStream {
    let inner = match fields {
        Fields::Named(FieldsNamed {
            named,
            brace_token: _,
        }) => {
            let calls = named
                .into_iter()
                .filter(|n| !check_ignore(&n.attrs))
                .map(|n| (n.ident.unwrap(), get_block(&n.attrs)))
                .map(|(name, block)| {
                    let inner = gen_prop_call(name, mutable, true);
                    gen_block_call(block, inner, mutable)
                });

            quote! {
                #(#calls)*
            }
        }

        Fields::Unnamed(FieldsUnnamed {
            unnamed,
            paren_token: _,
        }) => {
            let calls = unnamed
                .into_iter()
                .enumerate()
                .filter(|(_, n)| !check_ignore(&n.attrs))
                .map(|(i, n)| (Index::from(i), get_block(&n.attrs)))
                .map(|(index, block)| {
                    let inner = gen_prop_call(index, mutable, true);
                    gen_block_call(block, inner, mutable)
                });

            quote! {
                #(#calls)*

            }
        }
        _ => return quote! {},
    };

    let self_call = gen_self_call(self_type);
    quote! {
        #inner
        #self_call
    }
}

fn check_ignore(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("nonvisiting"))
}
fn get_block(attrs: &[Attribute]) -> Option<Ident> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("block"))
        .map(|attr| {
            attr.meta
                .require_list()
                .expect("It must contain one parameter") // this could be better, but im lazy
                .parse_args_with(Ident::parse)
                .expect("Must be only one ident")
        })
}
fn get_self_type(attrs: &[Attribute]) -> VisitType {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("ast"))
        .map(|attr| {
            match attr
                .meta
                .require_list()
                .expect("It must contain one parameter") // this could be better, but im lazy
                .parse_args_with(Ident::parse)
                .expect("Must be only one ident")
                .to_string()
                .as_str()
            {
                "expr" => VisitType::Expr,
                "stmt" => VisitType::Stmt,
                _ => VisitType::None,
            }
        })
        .unwrap_or(VisitType::None)
}
