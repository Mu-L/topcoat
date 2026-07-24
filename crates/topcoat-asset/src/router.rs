// Without the `serve` feature the docs' links to the serving configuration
// cannot resolve; they degrade to plain text instead of failing the build.
#![cfg_attr(not(feature = "serve"), allow(rustdoc::broken_intra_doc_links))]

#[cfg(feature = "serve")]
use topcoat_router::Route;
use topcoat_router::RouterBuilder;

#[cfg(feature = "serve")]
use crate::AssetRoute;
use crate::{AssetConfig, config::Host};

/// Registers assets on a [`RouterBuilder`].
///
/// Implemented for [`RouterBuilder`] so it is in scope wherever a router is
/// being built, enabling the [`assets`](Self::assets) method.
pub trait RouterBuilderAssetExt {
    /// Registers an [`AssetConfig`] on the router.
    ///
    /// The configuration is registered with the app context, where
    /// [`asset_config`](crate::asset_config) and
    /// [`bundled_asset`](crate::bundled_asset) read it back, and
    /// [`Asset`](crate::Asset) handles used as attribute values in the `view!`
    /// macro get rendered as the URL the asset is hosted at.
    ///
    /// A serving configuration ([`AssetConfig::serve`](crate::AssetConfig::serve))
    /// also adds an HTTP route for each bundled asset, served by the
    /// application itself, and declares those URLs as
    /// [static files](topcoat_router::RouterBuilder::static_files) so
    /// `topcoat export` copies them into an exported site. A configuration
    /// hosted externally
    /// ([`AssetConfig::hosted_at`](crate::AssetConfig::hosted_at)) adds no
    /// routes; the bundled files must be hosted at the configured base URL by
    /// other means, and are outside the export.
    ///
    /// Anything convertible into an [`AssetConfig`] is accepted: an
    /// [`AssetBundle`](crate::AssetBundle) registers as the configuration
    /// serving it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[topcoat::router::page("/")] async fn about() -> topcoat::Result { topcoat::view::view! {} }
    /// use topcoat::asset::{AssetBundle, RouterBuilderAssetExt};
    /// use topcoat::router::Router;
    ///
    /// pub fn router() -> Router {
    ///     Router::builder()
    ///         .page(about)
    ///         .assets(AssetBundle::load().unwrap())
    ///         .build()
    /// }
    /// ```
    ///
    /// Hosting the bundled files on a CDN instead of serving them:
    ///
    /// ```rust
    /// use topcoat::asset::{AssetBundle, AssetConfig, RouterBuilderAssetExt};
    /// use topcoat::router::Router;
    ///
    /// pub fn router() -> Router {
    ///     Router::builder()
    ///         .assets(AssetConfig::hosted_at(
    ///             AssetBundle::load().unwrap(),
    ///             "https://cdn.example.com/assets",
    ///         ))
    ///         .build()
    /// }
    /// ```
    #[must_use]
    fn assets(self, config: impl Into<AssetConfig>) -> Self;
}

impl RouterBuilderAssetExt for RouterBuilder {
    fn assets(self, config: impl Into<AssetConfig>) -> Self {
        let config = config.into();

        let builder = match &config.host {
            #[cfg(feature = "serve")]
            Host::Serve { dir } => {
                let mut builder = self;
                let mut urls = Vec::new();
                for asset in config.catalog.assets() {
                    let route = AssetRoute::new(dir, asset);
                    urls.push(route.path().to_string());
                    builder = builder.route(route);
                }
                builder.static_files(urls)
            }
            Host::External { .. } => self,
        };

        builder.app_context(config)
    }
}
