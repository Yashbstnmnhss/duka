use proc_macro2::Span;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, FieldsNamed, FieldsUnnamed, Ident, Index, Meta,
    Path, Result, Token, Variant, parse::Parse, punctuated::Punctuated,
};

use crate::attr::or_compile_error;

enum VisitType {
    Expr,
    Stmt,
    None,
}

pub fn generate_visitors(input: DeriveInput, mutable: bool) -> proc_macro2::TokenStream {
    let name = input.ident;
    let self_type = or_compile_error!(get_self_type(&input.attrs));
    let codes = if check_ignore(&input.attrs) {
        quote! {}
    } else {
        match input.data {
            Data::Enum(e) => or_compile_error!(gen_enum(e.variants, mutable, self_type)),
            Data::Struct(s) => or_compile_error!(gen_struct(s.fields, mutable, self_type)),
            Data::Union(_) => quote! {},
        }
    };

    let (impl_, ty_, where_) = &input.generics.split_for_impl();

    let visit_path = or_compile_error!(get_trait_path(&input.attrs, "visit_trait", "Visit"));
    let visit_mut_path =
        or_compile_error!(get_trait_path(&input.attrs, "visit_mut_trait", "VisitMut"));
    let visitor_path = or_compile_error!(get_trait_path(&input.attrs, "visitor_trait", "Visitor"));
    let visitor_mut_path = or_compile_error!(get_trait_path(
        &input.attrs,
        "visitor_mut_trait",
        "VisitorMut"
    ));

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
        return inner;
    }

    let Some(block_name) = block else {
        return inner;
    };

    let block_func = format_ident!("visit_{}_block", block_name);
    quote! {
        visitor.visit_block(true);
        visitor.#block_func(self, true);
        #inner
        visitor.#block_func(self, false);
        visitor.visit_block(false);
    }
}

fn gen_enum(
    variants: Punctuated<Variant, Token![,]>,
    mutable: bool,
    self_type: VisitType,
) -> Result<proc_macro2::TokenStream> {
    let arms = variants
        .into_iter()
        .filter(|variant| !check_ignore(&variant.attrs))
        .map(|variant| -> Result<proc_macro2::TokenStream> {
            let has_pattern = !matches!(variant.fields, Fields::Unit);
            // field name, block name (for mutable is empty ident)
            let names: Vec<_> = match variant.fields {
                Fields::Named(FieldsNamed {
                    brace_token: _,
                    named,
                }) => named
                    .into_iter()
                    .map(|f| {
                        let block = get_block(&f.attrs, mutable)?;
                        let ident = match f.ident {
                            Some(i) => i,
                            None => {
                                return Err(Error::new(
                                    Span::call_site(),
                                    "named field must have an identifier",
                                ));
                            }
                        };
                        Ok(((!check_ignore(&f.attrs)).then_some(ident), block))
                    })
                    .collect::<Result<Vec<_>>>()?,
                Fields::Unnamed(FieldsUnnamed {
                    paren_token: _,
                    unnamed,
                }) => unnamed
                    .into_iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let block = get_block(&f.attrs, mutable)?;
                        Ok((
                            (!check_ignore(&f.attrs)).then_some(format_ident!("_{}", i)),
                            block,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?,
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
                    if mutable && block.is_some() {
                        quote! {
                            visitor.visit_block(#i);
                        }
                    } else {
                        let inner = gen_prop_call(i, mutable, false);
                        gen_block_call(block, inner, mutable)
                    }
                });

            let pattern = if has_pattern {
                quote! {(#(#vars),*)}
            } else {
                quote! {}
            };
            Ok(quote! {
                Self::#name #pattern => {
                    #(#calls)*
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let self_call = gen_self_call(self_type);

    Ok(quote! {
        match self {
            #(#arms)*
            _ => {}
        }
        #self_call
    })
}
fn gen_struct(
    fields: Fields,
    mutable: bool,
    self_type: VisitType,
) -> Result<proc_macro2::TokenStream> {
    let inner = match fields {
        Fields::Named(FieldsNamed {
            named,
            brace_token: _,
        }) => {
            let calls = named
                .into_iter()
                .filter(|n| !check_ignore(&n.attrs))
                .map(|n| -> Result<proc_macro2::TokenStream> {
                    let name = match n.ident {
                        Some(i) => i,
                        None => {
                            return Err(Error::new(
                                Span::call_site(),
                                "named field must have an identifier",
                            ));
                        }
                    };
                    let block = get_block(&n.attrs, mutable)?;
                    let inner = gen_prop_call(name, mutable, true);
                    Ok(gen_block_call(block, inner, mutable))
                })
                .collect::<Result<Vec<_>>>()?;

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
                .map(|(i, n)| -> Result<proc_macro2::TokenStream> {
                    let block = get_block(&n.attrs, mutable)?;
                    let inner = gen_prop_call(Index::from(i), mutable, true);
                    Ok(gen_block_call(block, inner, mutable))
                })
                .collect::<Result<Vec<_>>>()?;

            quote! {
                #(#calls)*

            }
        }
        _ => quote! {},
    };

    let self_call = gen_self_call(self_type);
    Ok(quote! {
        #inner
        #self_call
    })
}

fn check_ignore(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("nonvisiting"))
}
fn get_block(attrs: &[Attribute], mutable: bool) -> Result<Option<Ident>> {
    if mutable {
        return Ok(attrs
            .iter()
            .find(|attr| attr.path().is_ident("block_mut"))
            .map(|_| format_ident!("__")));
    }
    let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("block")) else {
        return Ok(None);
    };
    let list = attr.meta.require_list()?;
    Ok(Some(list.parse_args_with(Ident::parse)?))
}
fn get_self_type(attrs: &[Attribute]) -> Result<VisitType> {
    let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("ast")) else {
        return Ok(VisitType::None);
    };
    let list = attr.meta.require_list()?;
    let ident = list.parse_args_with(Ident::parse)?;
    match ident.to_string().as_str() {
        "expr" => Ok(VisitType::Expr),
        "stmt" => Ok(VisitType::Stmt),
        "none" => Ok(VisitType::None),
        _ => Err(Error::new_spanned(
            &ident,
            "`ast(...)` must be `expr`, `stmt` or `none`",
        )),
    }
}

fn get_trait_path(attrs: &[Attribute], attr_name: &str, default_path: &str) -> Result<Path> {
    if let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident(attr_name))
        && let Meta::NameValue(name_value) = &attr.meta
        && let syn::Expr::Path(path_expr) = &name_value.value
    {
        return Ok(path_expr.path.clone());
    }
    syn::parse_str(default_path)
        .map_err(|e| Error::new(Span::call_site(), format!("invalid `{attr_name}`: {e}")))
}
