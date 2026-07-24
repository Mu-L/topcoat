//! Static export: the page and asset listing `topcoat export` builds a static
//! site from.
//!
//! A page with a fixed path is exported automatically. A page whose path has
//! dynamic segments opts in by naming a generator with
//! `#[page(generate_static = ...)]`, which returns the [`StaticParams`] sets to
//! export it for.
//!
//! The listing itself is served by [`StaticRoutes`], a development-only route
//! the router registers at [`STATIC_ROUTES_PATH`].

mod error;
mod params;
mod route;

pub use error::*;
pub use params::*;
pub use route::*;
