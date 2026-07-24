use std::collections::HashSet;

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{
    Ident, LitStr, Path, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};
use topcoat_core_grammar::QuoteOption;
use topcoat_core_grammar::paths::{topcoat_inventory, topcoat_router};

pub struct Segment {
    attrs: Punctuated<SegmentAttr, Token![,]>,
}

impl Segment {
    fn find_kind(&self) -> Option<&Ident> {
        self.attrs.iter().find_map(SegmentAttr::as_kind)
    }

    fn find_rename(&self) -> Option<&LitStr> {
        self.attrs.iter().find_map(SegmentAttr::as_rename)
    }

    fn find_generate_static(&self) -> Option<&Path> {
        self.attrs.iter().find_map(SegmentAttr::as_generate_static)
    }
}

impl Parse for Segment {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs: Punctuated<SegmentAttr, Token![,]> =
            input.parse_terminated(SegmentAttr::parse, Token![,])?;

        // Check for duplicates.
        let mut keys = HashSet::new();
        for attr in &attrs {
            if !keys.insert(attr.keyword()) {
                return Err(syn::Error::new(
                    attr.span(),
                    format_args!("duplicate attribute `{}`", attr.keyword()),
                ));
            }
        }

        if let Some(generate_static) = attrs.iter().find_map(SegmentAttr::as_generate_static) {
            let Some(kind) = attrs.iter().find_map(SegmentAttr::as_kind) else {
                return Err(syn::Error::new_spanned(
                    generate_static,
                    "`generate_static` requires `kind = Param` or `kind = CatchAll`",
                ));
            };
            if kind != "Param" && kind != "CatchAll" {
                return Err(syn::Error::new_spanned(
                    kind,
                    "`generate_static` is only valid for `Param` and `CatchAll` segments",
                ));
            }
        }

        Ok(Self { attrs })
    }
}

impl ToTokens for Segment {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if cfg!(feature = "discover") {
            let kind = self.find_kind();
            let rename = self.find_rename();
            let generate_static = self.find_generate_static();
            let generate_static_kind = kind.map(ToString::to_string);

            let kind =
                QuoteOption::new(kind.map(|kind| quote! { #topcoat_router::SegmentKind::#kind }));
            let rename = QuoteOption::new(
                rename.map(|rename| quote! { ::std::borrow::Cow::Borrowed(#rename) }),
            );
            let generate_static = QuoteOption::new(generate_static.map(|generate_static| {
                match generate_static_kind
                    .as_deref()
                    .expect("validated generate_static kind")
                {
                    "Param" => quote! {
                        #topcoat_router::StaticSegmentGenerator::new(|cx| {
                            ::std::boxed::Box::pin(async move {
                                let values: ::std::vec::Vec<::std::string::String> =
                                    #generate_static(cx).await?;
                                Ok(values
                                    .into_iter()
                                    .map(#topcoat_router::StaticSegmentValue::param)
                                    .collect())
                            })
                        })
                    },
                    "CatchAll" => quote! {
                        #topcoat_router::StaticSegmentGenerator::new(|cx| {
                            ::std::boxed::Box::pin(async move {
                                let values: ::std::vec::Vec<::std::vec::Vec<::std::string::String>> =
                                    #generate_static(cx).await?;
                                Ok(values
                                    .into_iter()
                                    .map(#topcoat_router::StaticSegmentValue::catch_all)
                                    .collect())
                            })
                        })
                    },
                    _ => unreachable!("validated generate_static kind"),
                }
            }));
            quote! {
                #topcoat_inventory::submit! {
                    #topcoat_router::Segment::new(
                        module_path!(),
                        #kind,
                        #rename,
                        #generate_static,
                    )
                }
            }
            .to_tokens(tokens);
        }
    }
}

mod kw {
    use syn::custom_keyword;

    custom_keyword!(kind);
    custom_keyword!(rename);
    custom_keyword!(generate_static);
}

#[allow(
    dead_code,
    reason = "parsed for syntax validation; not yet consumed by code generation"
)]
pub enum SegmentAttr {
    Kind {
        kind_kw: kw::kind,
        eq_token: Token![=],
        value: Ident,
    },
    Rename {
        rename_kw: kw::rename,
        eq_token: Token![=],
        value: LitStr,
    },
    GenerateStatic {
        generate_static_kw: kw::generate_static,
        eq_token: Token![=],
        value: Path,
    },
}

impl SegmentAttr {
    fn keyword(&self) -> &'static str {
        match self {
            Self::Kind { .. } => "kind",
            Self::Rename { .. } => "rename",
            Self::GenerateStatic { .. } => "generate_static",
        }
    }

    fn span(&self) -> Span {
        match self {
            Self::Kind { kind_kw, .. } => kind_kw.span,
            Self::Rename { rename_kw, .. } => rename_kw.span,
            Self::GenerateStatic {
                generate_static_kw, ..
            } => generate_static_kw.span,
        }
    }

    fn as_kind(&self) -> Option<&Ident> {
        match self {
            Self::Kind { value, .. } => Some(value),
            Self::Rename { .. } | Self::GenerateStatic { .. } => None,
        }
    }

    fn as_rename(&self) -> Option<&LitStr> {
        match self {
            Self::Rename { value, .. } => Some(value),
            Self::Kind { .. } | Self::GenerateStatic { .. } => None,
        }
    }

    fn as_generate_static(&self) -> Option<&Path> {
        match self {
            Self::GenerateStatic { value, .. } => Some(value),
            Self::Kind { .. } | Self::Rename { .. } => None,
        }
    }
}

impl Parse for SegmentAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(kw::kind) {
            Ok(Self::Kind {
                kind_kw: input.parse()?,
                eq_token: input.parse()?,
                value: input.parse()?,
            })
        } else if lookahead.peek(kw::rename) {
            Ok(Self::Rename {
                rename_kw: input.parse()?,
                eq_token: input.parse()?,
                value: input.parse()?,
            })
        } else if lookahead.peek(kw::generate_static) {
            Ok(Self::GenerateStatic {
                generate_static_kw: input.parse()?,
                eq_token: input.parse()?,
                value: input.parse()?,
            })
        } else {
            Err(lookahead.error())
        }
    }
}
