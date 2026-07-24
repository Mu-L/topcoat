//! Detection of Topcoat's development tooling.
//!
//! The `topcoat` CLI runs an application as a child process and passes the
//! address of its own local server through the `TOPCOAT_DEV_URL` environment
//! variable. Its presence is what tells a running application that it is being
//! driven by the tooling rather than serving production traffic, and it gates
//! every development-only behavior the framework adds: the live-reload script,
//! the readiness notification, and the development-only routes the router
//! registers (see [`STATIC_ROUTES_PATH`](crate::STATIC_ROUTES_PATH)).
//!
//! A deployed application is started directly, without the variable, so none
//! of those behaviors are reachable in production.

use http::request::Parts;
use topcoat_core::context::{Cx, try_request_context};

/// The environment variable the `topcoat` CLI sets on the applications it
/// starts, holding the HTTP base URL of the CLI's local server.
pub const DEV_URL_ENV: &str = "TOPCOAT_DEV_URL";

/// The header `topcoat export` sets on every request it renders a static page
/// from.
///
/// `topcoat export` drives the application through the same development
/// tooling `topcoat dev` uses, so pages would otherwise render the tooling's
/// own additions -- the live-reload script above all -- into files meant for a
/// static host. Marking the request lets those additions stay out of the
/// exported HTML while everything else renders exactly as it is served.
pub const EXPORT_HEADER: &str = "x-topcoat-export";

/// The HTTP base URL of the `topcoat` CLI server driving this process, or
/// [`None`] when the application was not started by the CLI.
#[must_use]
pub fn server_url() -> Option<String> {
    std::env::var(DEV_URL_ENV).ok()
}

/// Whether Topcoat's development-only tooling is enabled for this process.
///
/// True exactly when the application runs under the `topcoat` CLI, which is
/// never the case for a deployed application.
#[must_use]
pub fn tooling_enabled() -> bool {
    server_url().is_some()
}

/// Whether the request being handled was made by `topcoat export` to render a
/// page into a static site.
///
/// Development-only additions to a rendered page check this and render nothing
/// for such a request, so an exported page carries no tooling.
#[must_use]
pub fn is_export_request(cx: &Cx) -> bool {
    try_request_context::<Parts>(cx).is_some_and(|parts| parts.headers.contains_key(EXPORT_HEADER))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use topcoat_core::context::{ContextMap, CxBuilder};

    use super::*;

    fn cx_with(request: http::Request<()>) -> CxBuilder {
        let mut cx = CxBuilder::new(Arc::new(ContextMap::new()));
        cx.insert(request.into_parts().0);
        cx
    }

    #[test]
    fn an_unmarked_request_is_not_an_export() {
        let cx = cx_with(http::Request::new(()));
        assert!(!is_export_request(&cx));
    }

    #[test]
    fn a_marked_request_is_an_export() {
        let cx = cx_with(
            http::Request::builder()
                .header(EXPORT_HEADER, "1")
                .body(())
                .unwrap(),
        );
        assert!(is_export_request(&cx));
    }

    #[test]
    fn a_request_less_context_is_not_an_export() {
        // Rendering outside a request (a test, say) must not look like one.
        let cx = CxBuilder::new(Arc::new(ContextMap::new()));
        assert!(!is_export_request(&cx));
    }
}
