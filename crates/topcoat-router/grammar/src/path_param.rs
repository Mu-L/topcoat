use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Data, DeriveInput, Fields, Path, Token, Type,
    parse::{Parse, ParseStream},
};
use topcoat_core_grammar::paths::{
    topcoat_context, topcoat_context_macro, topcoat_error, topcoat_router, topcoat_router_macro,
};

use super::error_attr::ErrorAttr;

pub struct PathParamAttr {
    error: Option<ErrorAttr>,
    generate_static: Option<Path>,
}

impl Parse for PathParamAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.parse_terminated(PathParamAttrItem::parse, Token![,])?;
        let mut error = None;
        let mut generate_static = None;
        for attr in attrs {
            match attr {
                PathParamAttrItem::Error(value) => {
                    if error.replace(value).is_some() {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            "duplicate attribute `error`",
                        ));
                    }
                }
                PathParamAttrItem::GenerateStatic { value, .. } => {
                    if generate_static.replace(value).is_some() {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            "duplicate attribute `generate_static`",
                        ));
                    }
                }
            }
        }
        Ok(Self {
            error,
            generate_static,
        })
    }
}

mod kw {
    syn::custom_keyword!(generate_static);
}

#[allow(dead_code, reason = "tokens are retained for parsing diagnostics")]
enum PathParamAttrItem {
    Error(ErrorAttr),
    GenerateStatic {
        generate_static_kw: kw::generate_static,
        eq_token: Token![=],
        value: Path,
    },
}

impl Parse for PathParamAttrItem {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if ErrorAttr::peek(input) {
            Ok(Self::Error(input.parse()?))
        } else if input.peek(kw::generate_static) {
            Ok(Self::GenerateStatic {
                generate_static_kw: input.parse()?,
                eq_token: input.parse()?,
                value: input.parse()?,
            })
        } else {
            Err(input.lookahead1().error())
        }
    }
}

pub struct PathParamItem {
    item: DeriveInput,
    inner_ty: Type,
}

impl PathParamItem {
    /// Whether the parameter borrows the raw segment (a `str` inner type)
    /// rather than parsing it.
    fn borrows_raw_segment(&self) -> bool {
        matches!(
            &self.inner_ty,
            Type::Path(path) if path.qself.is_none() && path.path.is_ident("str")
        )
    }
}

impl Parse for PathParamItem {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let item: DeriveInput = input.parse()?;
        let Data::Struct(data_struct) = &item.data else {
            return Err(syn::Error::new_spanned(
                &item.ident,
                "path_param can only be applied to a tuple struct with one unnamed field",
            ));
        };
        let Fields::Unnamed(unnamed) = &data_struct.fields else {
            return Err(syn::Error::new_spanned(
                &data_struct.fields,
                "path_param can only be applied to a tuple struct with one unnamed field",
            ));
        };
        if unnamed.unnamed.len() != 1 {
            return Err(syn::Error::new_spanned(
                &unnamed.unnamed,
                "path_param structs must have exactly one unnamed field",
            ));
        }
        let inner_ty = unnamed.unnamed.first().unwrap().ty.clone();
        Ok(Self { item, inner_ty })
    }
}

pub struct PathParam(PathParamAttr, PathParamItem);

impl PathParam {
    /// Combines a parsed attribute and item.
    ///
    /// # Errors
    ///
    /// Returns an error if the attribute declares `error = ...` for a `&str`
    /// parameter, which borrows the raw segment and cannot fail.
    pub fn new(attr: PathParamAttr, item: PathParamItem) -> syn::Result<Self> {
        if let Some(error) = &attr.error
            && item.borrows_raw_segment()
        {
            return Err(syn::Error::new(
                error.span(),
                "`error` cannot be used with a `&str` path parameter, \
                     which borrows the raw segment and cannot fail",
            ));
        }
        if let Some(generate_static) = &attr.generate_static
            && !item.item.generics.params.is_empty()
        {
            return Err(syn::Error::new_spanned(
                generate_static,
                "`generate_static` cannot be used on a generic path parameter",
            ));
        }
        Ok(Self(attr, item))
    }

    /// Parses a `path_param` attribute and item from token streams.
    ///
    /// # Errors
    ///
    /// Returns an error if either token stream fails to parse as a
    /// [`PathParamAttr`] or [`PathParamItem`], if the item is not a tuple
    /// struct with exactly one unnamed field, or if the attribute and item
    /// disagree as described on [`PathParam::new`].
    pub fn parse(attr: TokenStream, item: TokenStream) -> syn::Result<Self> {
        Self::new(syn::parse2(attr)?, syn::parse2(item)?)
    }
}

impl ToTokens for PathParam {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let borrows_raw_segment = self.1.borrows_raw_segment();

        // Emit a copy of the user's struct. When the parameter borrows the raw
        // segment its tuple field is never constructed, so silence the
        // resulting dead-code warning.
        let mut item = self.1.item.clone();
        if borrows_raw_segment
            && let Data::Struct(data) = &mut item.data
            && let Fields::Unnamed(unnamed) = &mut data.fields
            && let Some(field) = unnamed.unnamed.first_mut()
        {
            field.attrs.push(syn::parse_quote!(#[allow(dead_code)]));
        }

        let ident = &item.ident;
        let inner_ty = &self.1.inner_ty;
        let name_string = ident.to_string().to_snake_case();
        let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
        let static_segment = self
            .0
            .generate_static
            .as_ref()
            .filter(|_| cfg!(feature = "discover"))
            .map(|generate_static| {
                let convert = if borrows_raw_segment {
                    quote! {
                        let values: ::std::vec::Vec<::std::string::String> =
                            #generate_static(cx).await?;
                        Ok(values)
                    }
                } else {
                    quote! {
                        let values: ::std::vec::Vec<#inner_ty> = #generate_static(cx).await?;
                        Ok(values
                            .into_iter()
                            .map(|value| ::std::string::ToString::to_string(&value))
                            .collect())
                    }
                };
                quote! {
                    async fn __topcoat_generate_static(
                        cx: &#topcoat_context::Cx,
                    ) -> #topcoat_error::Result<::std::vec::Vec<::std::string::String>> {
                        #convert
                    }

                    #topcoat_router_macro::segment!(
                        kind = Param,
                        rename = #name_string,
                        generate_static = __topcoat_generate_static,
                    );
                }
            });
        let static_segment = static_segment.unwrap_or_else(|| {
            quote! {
                #topcoat_router_macro::segment!(kind = Param, rename = #name_string);
            }
        });

        let (output_ty, path_param_fn) = if borrows_raw_segment {
            (
                quote! { &'__cx str },
                quote! {
                    fn path_param(
                        cx: &#topcoat_context::Cx,
                        _: #topcoat_router::PathParamSealed,
                    ) -> Self::Output<'_> {
                        for (key, value) in #topcoat_router::raw_path_params(cx) {
                            if key == #name_string {
                                return value;
                            }
                        }
                        panic!("path parameter \"{}\" was not found in request path", #name_string);
                    }
                },
            )
        } else {
            let (error_ty, map_err) = match &self.0.error {
                Some(error) => {
                    let default = format!("invalid value for path parameter \"{name_string}\"");
                    (
                        error.ty(),
                        error.map_err(quote! { |_| #topcoat_router::error::bad_request(#default) }),
                    )
                }
                None => (
                    quote! { &'__cx <#inner_ty as ::core::str::FromStr>::Err },
                    quote! {},
                ),
            };
            (
                quote! {
                    ::core::result::Result<&'__cx #inner_ty, #error_ty>
                },
                quote! {
                    fn path_param(
                        cx: &#topcoat_context::Cx,
                        _: #topcoat_router::PathParamSealed,
                    ) -> Self::Output<'_> {
                        #[#topcoat_context_macro::memoize]
                        fn parse(cx: &#topcoat_context::Cx) -> ::core::result::Result<#ident #ty_generics, <#inner_ty as ::core::str::FromStr>::Err> {
                            for (key, value) in #topcoat_router::raw_path_params(cx) {
                                if key == #name_string {
                                    return ::core::str::FromStr::from_str(value).map(#ident);
                                }
                            }
                            panic!("path parameter \"{}\" was not found in request path", #name_string);
                        }
                        parse(cx).map(|value| &value.0)#map_err
                    }
                },
            )
        };

        quote! {
            #item

            impl #impl_generics #topcoat_router::PathParam for #ident #ty_generics #where_clause {
                type Output<'__cx> = #output_ty;

                #path_param_fn
            }

            #static_segment
        }
        .to_tokens(tokens);
    }
}
