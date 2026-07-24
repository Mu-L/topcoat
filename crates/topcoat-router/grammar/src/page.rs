use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Ident, ItemFn, LitStr, ReturnType, Token, Visibility,
    parse::{Parse, ParseStream},
    parse_quote,
    spanned::Spanned,
};
use topcoat_core_grammar::ParseOption;
use topcoat_core_grammar::paths::{
    topcoat_context, topcoat_inventory, topcoat_router, topcoat_view_macro,
};

use super::handler_args::{HandlerArg, HandlerArgs, request_ident};
use super::method::Methods;

/// The name of the only option `#[page]` accepts after its methods and path.
const GENERATE_STATIC: &str = "generate_static";

pub struct PageAttr {
    /// The declared HTTP methods; the page serves `GET` when omitted.
    methods: Option<Methods>,
    path: Option<LitStr>,
    /// The function supplying the parameter sets the page is statically
    /// exported for, from `generate_static = ...`.
    generate_static: Option<syn::Path>,
}

impl Parse for PageAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            // An HTTP method is never followed by `=`, so a leading
            // `generate_static = ...` is not mistaken for one.
            methods: if input.peek2(Token![=]) {
                None
            } else {
                Methods::parse_option(input)?
            },
            path: input.peek(LitStr).then(|| input.parse()).transpose()?,
            generate_static: parse_generate_static(input)?,
        })
    }
}

/// Parses the trailing `generate_static = path` option, preceded by a comma
/// when methods or a path came before it.
fn parse_generate_static(input: ParseStream) -> syn::Result<Option<syn::Path>> {
    if input.is_empty() {
        return Ok(None);
    }
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    }

    let name: Ident = input.parse()?;
    if name != GENERATE_STATIC {
        return Err(syn::Error::new(
            name.span(),
            format!("unknown `page` option `{name}`; expected `{GENERATE_STATIC}`"),
        ));
    }
    input.parse::<Token![=]>()?;
    let path = input.parse()?;

    if !input.is_empty() {
        return Err(input.error(format!("unexpected tokens after `{GENERATE_STATIC}`")));
    }
    Ok(Some(path))
}

pub struct PageItem {
    item: ItemFn,
    args: HandlerArgs,
}

impl Parse for PageItem {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let item: ItemFn = input.parse()?;
        if item.sig.asyncness.is_none() {
            return Err(syn::Error::new(
                item.sig.fn_token.span(),
                "page functions must be async",
            ));
        }
        if let ReturnType::Default = &item.sig.output {
            return Err(syn::Error::new(
                item.sig.fn_token.span(),
                "page functions must declare a return type",
            ));
        }
        let args = HandlerArgs::parse(&item, "page")?;
        Ok(Self { item, args })
    }
}

pub struct Page(PageAttr, PageItem);

impl Page {
    #[must_use]
    pub fn new(attr: PageAttr, item: PageItem) -> Self {
        Self(attr, item)
    }

    /// Parses a page attribute and item from token streams.
    ///
    /// # Errors
    ///
    /// Returns an error if either token stream fails to parse as a
    /// [`PageAttr`] or [`PageItem`], or if the item is not a valid page
    /// handler.
    pub fn parse(attr: TokenStream, item: TokenStream) -> syn::Result<Self> {
        Ok(Self::new(syn::parse2(attr)?, syn::parse2(item)?))
    }
}

impl ToTokens for Page {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let attr = &self.0;
        let item = &self.1.item;
        let args = &self.1.args;
        let vis = &item.vis;
        let ident = &item.sig.ident;
        let output = &item.sig.output;

        // Component face: renders the page inline from `view!`. It always
        // takes `cx` (feeding the function's injected context parameter), and
        // a page that reads a request body takes the already-parsed value as a
        // `body` prop instead. The marker struct this expands to is a unit
        // struct, so `#ident` stays a value usable directly in
        // `router.page(...)`.
        let body_param = args.request().map(|ty| quote! { , body: #ty });
        let body_arg = args.request().map(|_| quote! { , body });
        quote! {
            #[#topcoat_view_macro::component]
            #vis async fn #ident(cx: &#topcoat_context::Cx #body_param) #output {
                #ident::handler(cx #body_arg).await
            }
        }
        .to_tokens(tokens);

        // The user's function, re-emitted under its original name inside the
        // anonymous const below to keep the module namespace clean. Its own
        // name shadows the marker within its body, so bindings named after the
        // page keep working. The injected `__cx` parameter carries the ambient
        // context that `view!` bodies read.
        let mut inner = item.clone();
        inner.vis = Visibility::Inherited;
        inner.sig.generics.params.insert(0, parse_quote! { '__cx });
        inner
            .sig
            .inputs
            .insert(0, parse_quote! { __cx: &'__cx #topcoat_context::Cx });
        inner
            .attrs
            .push(parse_quote! { #[allow(clippy::unused_async)] });

        // The bridge the component face calls: associated items are reached
        // through the type rather than lexical scope, so `#ident::handler` is
        // callable from outside the anonymous const. It forwards to the user's
        // function positionally, in declared parameter order.
        let forward_args = args.iter().map(|arg| match arg {
            HandlerArg::Cx => quote! { cx },
            HandlerArg::Request(_) => quote! { body },
        });
        let handler = quote! {
            impl #ident {
                async fn handler(cx: &#topcoat_context::Cx #body_param) #output {
                    #ident(cx #(, #forward_args)*).await
                }
            }
        };

        // The render function backing the registered page: it parses the
        // request body (when the page takes one) and calls the user's function
        // directly.
        let parse_request = args.request().map(|request_ty| {
            let request_ident = request_ident();
            quote! {
                let #request_ident = <#request_ty as #topcoat_router::FromRequest>::from_request(cx, body).await?;
            }
        });
        let call_args = args.call_args();
        let render = quote! {
            |cx, body| ::std::boxed::Box::pin(async move {
                #parse_request
                #ident(cx #(, #call_args)*).await
            })
        };

        // The erased page is built once in a `const` so it can be used from
        // both the `From` impl (backing manual `router.page(#ident)`
        // registration) and the discovery submission (which expands to a
        // `static`, requiring a const initializer).
        let methods = attr.methods.as_ref().map_or_else(
            || quote! { #topcoat_router::OwnedMethods::One(#topcoat_router::Method::GET) },
            ToTokens::to_token_stream,
        );
        // The `generate_static` function, wrapped so it matches the erased
        // `GenerateStaticFn` signature the router stores. Both the explicit-
        // and module-path forms carry it, so a page opts into the static
        // export the same way however its URL is derived.
        let generate_static = attr.generate_static.as_ref().map(|path| {
            quote! {
                .with_generate_static(|cx| ::std::boxed::Box::pin(async move {
                    #path(cx).await
                }))
            }
        });

        let erased = if let Some(path) = attr.path.as_ref() {
            quote! {
                const ERASED: #topcoat_router::PageFn = #topcoat_router::PageFn::const_new(
                    #methods,
                    ::std::borrow::Cow::Borrowed(#topcoat_router::Path::new(#path)),
                    #render,
                )#generate_static;

                impl ::core::convert::From<#ident> for #topcoat_router::PageFn {
                    fn from(_: #ident) -> Self {
                        ERASED
                    }
                }
            }
        } else {
            quote! {
                const ERASED: #topcoat_router::ModulePageFn =
                    #topcoat_router::ModulePageFn::new(#methods, module_path!(), #render)
                        #generate_static;

                impl ::core::convert::From<#ident> for #topcoat_router::ModulePageFn {
                    fn from(_: #ident) -> Self {
                        ERASED
                    }
                }
            }
        };

        let submit =
            cfg!(feature = "discover").then(|| quote! { #topcoat_inventory::submit! { ERASED } });

        quote! {
            const _: () = {
                #inner

                #handler

                #erased

                #submit
            };
        }
        .to_tokens(tokens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr_err(source: &str) -> String {
        match syn::parse_str::<PageAttr>(source) {
            Ok(_) => panic!("expected parse error for `{source}`"),
            Err(err) => err.to_string(),
        }
    }

    fn parse_err(source: &str) -> String {
        match syn::parse_str::<PageItem>(source) {
            Ok(_) => panic!("expected parse error for `{source}`"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn attr_without_methods_leaves_them_unset() {
        let attr: PageAttr = syn::parse_str("\"/about\"").unwrap();
        assert!(attr.methods.is_none());
        assert!(attr.path.is_some());

        let attr: PageAttr = syn::parse_str("").unwrap();
        assert!(attr.methods.is_none());
        assert!(attr.path.is_none());
    }

    #[test]
    fn attr_accepts_methods_before_the_path() {
        let attr: PageAttr = syn::parse_str("POST \"/submit\"").unwrap();
        assert!(attr.methods.is_some());
        assert!(attr.path.is_some());
    }

    #[test]
    fn attr_accepts_methods_without_a_path() {
        for source in ["POST", "[GET, POST]", "*"] {
            let attr: PageAttr = syn::parse_str(source).unwrap();
            assert!(attr.methods.is_some());
            assert!(attr.path.is_none());
        }
    }

    #[test]
    fn attr_accepts_generate_static_on_its_own() {
        let attr: PageAttr = syn::parse_str("generate_static = posts").unwrap();
        assert!(attr.methods.is_none());
        assert!(attr.path.is_none());
        assert!(attr.generate_static.is_some());
    }

    #[test]
    fn attr_accepts_generate_static_after_an_explicit_path() {
        let attr: PageAttr = syn::parse_str("\"/blog/{slug}\", generate_static = posts").unwrap();
        assert!(attr.path.is_some());
        assert!(attr.generate_static.is_some());
    }

    #[test]
    fn attr_accepts_generate_static_after_methods_and_a_path() {
        let attr: PageAttr =
            syn::parse_str("[GET, POST] \"/blog/{slug}\", generate_static = posts").unwrap();
        assert!(attr.methods.is_some());
        assert!(attr.path.is_some());
        assert!(attr.generate_static.is_some());
    }

    #[test]
    fn attr_accepts_a_qualified_generate_static_path() {
        let attr: PageAttr =
            syn::parse_str("generate_static = crate::blog::generate_static_params").unwrap();
        assert!(attr.generate_static.is_some());
    }

    #[test]
    fn attr_rejects_an_unknown_option() {
        let err = attr_err("\"/x\", revalidate = 60");
        assert!(err.contains("unknown `page` option"), "{err}");
    }

    #[test]
    fn attr_rejects_trailing_tokens_after_generate_static() {
        let err = attr_err("generate_static = posts, extra");
        assert!(err.contains("unexpected tokens"), "{err}");
    }

    #[test]
    fn accepts_async_fn_with_return_type() {
        syn::parse_str::<PageItem>("async fn home(cx: &Cx) -> Result { todo!() }").unwrap();
    }

    #[test]
    fn accepts_a_destructured_request_parameter() {
        syn::parse_str::<PageItem>(
            "async fn search(Form(input): Form<Search>, cx: &Cx) -> Result { todo!() }",
        )
        .unwrap();
    }

    #[test]
    fn rejects_non_async_fn() {
        assert!(parse_err("fn home() -> Result { todo!() }").contains("must be async"));
    }

    #[test]
    fn rejects_missing_return_type() {
        assert!(parse_err("async fn home() {}").contains("must declare a return type"));
    }

    #[test]
    fn rejects_self_receiver() {
        let err = parse_err("async fn home(&self) -> Result { todo!() }");
        assert!(err.contains("cannot take a `self` receiver"));
    }

    #[test]
    fn rejects_multiple_request_parameters() {
        let err = parse_err("async fn home(a: Form<A>, b: Form<B>) -> Result { todo!() }");
        assert!(err.contains("more than one request body parameter"));
    }
}
