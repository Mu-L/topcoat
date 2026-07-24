use std::borrow::Cow;
use std::collections::HashMap;

use http::header::CONTENT_TYPE;
use http::{HeaderValue, Method, StatusCode};
use serde::Serialize;
use topcoat_core::context::Cx;

use crate::{
    Body, GenerateStaticFn, Methods, Path, PathSegment, Response, Route, RouteFuture,
    StaticExportError, StaticParams,
};

/// URL path of the development-only route listing everything a static export
/// covers.
pub const STATIC_ROUTES_PATH: &str = "/_topcoat/routes/static";

const APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");
const TEXT_PLAIN: HeaderValue = HeaderValue::from_static("text/plain; charset=utf-8");

/// A page as the static export sees it: its route path, whether it answers
/// `GET`, and the generator that supplies the parameter sets to export it for.
///
/// The router builds one per registered page; a page is exported only if it
/// answers `GET` and either has a fixed path or names a generator with
/// `#[page(generate_static = ...)]`.
#[derive(Debug, Clone)]
pub struct StaticPage {
    /// The page's route path, e.g. `/blog/{slug}`.
    route: Cow<'static, Path>,
    /// Whether the page answers `GET`, the only method a static host serves.
    serves_get: bool,
    /// The page's `generate_static` function, when it declares one.
    generate_static: Option<GenerateStaticFn>,
}

impl StaticPage {
    /// Creates an entry for a page at `route`.
    #[must_use]
    pub fn new(
        route: Cow<'static, Path>,
        serves_get: bool,
        generate_static: Option<GenerateStaticFn>,
    ) -> Self {
        Self {
            route,
            serves_get,
            generate_static,
        }
    }

    /// The page's route path.
    #[must_use]
    pub fn route(&self) -> &Path {
        &self.route
    }

    /// The URLs this page contributes to a static export, running its
    /// generator when it declares one.
    ///
    /// A page that cannot be exported at all -- a dynamic path with no
    /// generator, or a page that does not answer `GET` -- yields no URLs.
    async fn expand(&self, cx: &Cx) -> Result<Vec<String>, StaticExportError> {
        let route = self.route.to_string();

        let Some(generate) = self.generate_static else {
            // A page with a fixed path needs no parameters and is exported as
            // it stands; a dynamic one that never opted in is left out.
            if !self.serves_get
                || self
                    .route
                    .segments()
                    .any(|s| s.is_param() || s.is_catch_all())
            {
                return Ok(Vec::new());
            }
            return Ok(vec![expand_path(&self.route, &StaticParams::new())?]);
        };

        if !self.serves_get {
            return Err(StaticExportError::NotExportable { route });
        }

        let generated = generate(cx)
            .await
            .map_err(|error| StaticExportError::Generator {
                route,
                message: error.to_string(),
            })?;

        generated
            .iter()
            .map(|params| expand_path(&self.route, params))
            .collect()
    }
}

/// One entry of the static route listing: the URL to fetch and the route path
/// it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaticPath {
    /// The concrete URL path to request, e.g. `/blog/hello`.
    pub path: String,
    /// The route path the page is registered at, e.g. `/blog/{slug}`.
    pub route: String,
}

/// The JSON body served at [`STATIC_ROUTES_PATH`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StaticRouteListing {
    /// Every page URL the export should render, sorted by path.
    pub pages: Vec<StaticPath>,
    /// Every additional URL the export should copy verbatim (the application's
    /// served assets), sorted by path.
    pub assets: Vec<String>,
}

/// The development-only [`Route`] serving the static export listing at
/// [`STATIC_ROUTES_PATH`].
///
/// The router registers one automatically when the application runs under the
/// `topcoat` CLI (see [`dev`](crate::dev)); a deployed application never has
/// this route, so the listing is unreachable in production.
///
/// Answering the request runs every page's `generate_static` function inside
/// the request, so generators can read the app context and other
/// request-scoped state. A page whose parameters do not line up with its path,
/// or two pages that generate the same URL, fail the request with a `500` whose
/// body is the [`StaticExportError`] message.
#[derive(Debug, Clone)]
pub struct StaticRoutes {
    /// The registered pages, whether or not they end up being exported.
    pages: Vec<StaticPage>,
    /// URLs served by this router that an export copies as-is.
    assets: Vec<String>,
}

impl StaticRoutes {
    /// Creates the listing route for `pages` and `assets`.
    #[must_use]
    pub fn new(pages: Vec<StaticPage>, assets: Vec<String>) -> Self {
        Self { pages, assets }
    }

    /// Builds the listing, running every page's generator.
    ///
    /// # Errors
    ///
    /// Returns the first [`StaticExportError`] a page reports, or the conflict
    /// between two pages that generate the same URL.
    pub async fn listing(&self, cx: &Cx) -> Result<StaticRouteListing, StaticExportError> {
        let mut expanded = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            expanded.push((page.route.to_string(), page.expand(cx).await?));
        }

        let mut assets = self.assets.clone();
        assets.sort();
        assets.dedup();

        let pages = collect_paths(expanded)?;

        // A page and a static file at one URL would be written to the same
        // place, with only the order deciding which survived.
        for page in &pages {
            if assets.binary_search(&page.path).is_ok() {
                return Err(StaticExportError::ConflictsWithStaticFile {
                    route: page.route.clone(),
                    path: page.path.clone(),
                });
            }
        }

        Ok(StaticRouteListing { pages, assets })
    }
}

impl Route for StaticRoutes {
    fn methods(&self) -> Methods<'_> {
        Methods::Only(&[Method::GET])
    }

    fn path(&self) -> &Path {
        Path::new(STATIC_ROUTES_PATH)
    }

    fn handle<'cx>(&'cx self, cx: &'cx Cx, _body: Body) -> RouteFuture<'cx> {
        Box::pin(async move {
            // The listing is only ever read by `topcoat export`, so a failure
            // reports the reason in the body rather than hiding it behind a
            // bare 500.
            let (status, content_type, body) = match self.listing(cx).await {
                Ok(listing) => (
                    StatusCode::OK,
                    APPLICATION_JSON,
                    Body::from(serde_json::to_vec(&listing)?),
                ),
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    TEXT_PLAIN,
                    Body::from(error.to_string()),
                ),
            };

            let mut response = Response::new(body);
            *response.status_mut() = status;
            response.headers_mut().insert(CONTENT_TYPE, content_type);
            Ok(response)
        })
    }
}

/// Renders `route` into a concrete URL by substituting `params`, validating
/// that the two describe the same set of dynamic segments.
fn expand_path(route: &Path, params: &StaticParams) -> Result<String, StaticExportError> {
    let route_string = route.to_string();

    // A name given twice is ambiguous, whichever value would win.
    for (index, (name, _)) in params.iter().enumerate() {
        if params.iter().take(index).any(|(seen, _)| seen == name) {
            return Err(StaticExportError::DuplicateParam {
                route: route_string,
                name: name.to_owned(),
            });
        }
    }

    // Anything the path does not declare would silently go nowhere.
    for (name, _) in params.iter() {
        let declared = route.segments().any(|segment| {
            matches!(segment, PathSegment::Param(n) | PathSegment::CatchAll(n) if n == name)
        });
        if !declared {
            return Err(StaticExportError::UnknownParam {
                route: route_string,
                name: name.to_owned(),
            });
        }
    }

    let mut url = String::new();
    for segment in route.segments() {
        match segment {
            // Groups shape layouts and layers, never the URL.
            PathSegment::Group(_) => {}
            PathSegment::Static(name) => {
                url.push('/');
                url.push_str(name);
            }
            PathSegment::Param(name) => {
                let value = value_of(&route_string, params, name)?;
                // A `{name}` stands for exactly one segment, so a value
                // carrying a separator would silently address a different URL.
                if value.contains('/') {
                    return Err(StaticExportError::InvalidParam {
                        route: route_string,
                        name: name.to_owned(),
                        value: value.to_owned(),
                    });
                }
                url.push('/');
                url.push_str(value);
            }
            PathSegment::CatchAll(name) => {
                // A `{*name}` stands for the whole remainder, separators
                // included; surrounding ones would double up against the `/`
                // this writes.
                let value = value_of(&route_string, params, name)?.trim_matches('/');
                if value.is_empty() {
                    return Err(StaticExportError::EmptyParam {
                        route: route_string,
                        name: name.to_owned(),
                    });
                }
                url.push('/');
                url.push_str(value);
            }
        }
    }

    // A path made only of groups, or the root itself, addresses `/`.
    if url.is_empty() {
        url.push('/');
    }
    Ok(url)
}

/// Reads `name` out of `params`, rejecting a set that omits it or gives it an
/// empty value.
fn value_of<'a>(
    route: &str,
    params: &'a StaticParams,
    name: &str,
) -> Result<&'a str, StaticExportError> {
    match params.get(name) {
        None => Err(StaticExportError::MissingParam {
            route: route.to_owned(),
            name: name.to_owned(),
        }),
        Some("") => Err(StaticExportError::EmptyParam {
            route: route.to_owned(),
            name: name.to_owned(),
        }),
        Some(value) => Ok(value),
    }
}

/// Flattens the URLs every page generated into one sorted listing, rejecting
/// any URL two pages (or one page twice) would write to the same file.
fn collect_paths(
    expanded: Vec<(String, Vec<String>)>,
) -> Result<Vec<StaticPath>, StaticExportError> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut paths = Vec::new();

    for (route, urls) in expanded {
        for path in urls {
            if let Some(first) = seen.get(&path) {
                return Err(if *first == route {
                    StaticExportError::DuplicatePath { route, path }
                } else {
                    StaticExportError::ConflictingPaths {
                        first: first.clone(),
                        second: route,
                        path,
                    }
                });
            }
            seen.insert(path.clone(), route.clone());
            paths.push(StaticPath {
                path,
                route: route.clone(),
            });
        }
    }

    paths.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Arc;

    use topcoat_core::context::{ContextMap, CxBuilder};
    use topcoat_core::error::{Error, Result};

    use super::*;

    fn params(pairs: &[(&str, &str)]) -> StaticParams {
        pairs.iter().copied().collect()
    }

    fn expand(route: &str, pairs: &[(&str, &str)]) -> Result<String, StaticExportError> {
        expand_path(Path::new(route), &params(pairs))
    }

    // -- expand_path --

    #[test]
    fn fixed_path_expands_to_itself() {
        assert_eq!(expand("/about", &[]).unwrap(), "/about");
        assert_eq!(expand("/", &[]).unwrap(), "/");
    }

    #[test]
    fn group_segments_are_stripped() {
        assert_eq!(expand("/(marketing)/pricing", &[]).unwrap(), "/pricing");
        // A page that is only groups serves the root.
        assert_eq!(expand("/(marketing)", &[]).unwrap(), "/");
    }

    #[test]
    fn params_are_substituted() {
        assert_eq!(
            expand("/blog/{year}/{slug}", &[("year", "2026"), ("slug", "hi")]).unwrap(),
            "/blog/2026/hi"
        );
    }

    #[test]
    fn parent_and_child_params_are_both_required() {
        // The page only declares `{slug}` itself; `{year}` comes from a parent
        // segment, and a set naming just one of them is rejected.
        let error = expand("/blog/{year}/{slug}", &[("slug", "hi")]).unwrap_err();
        assert_eq!(
            error,
            StaticExportError::MissingParam {
                route: "/blog/{year}/{slug}".to_owned(),
                name: "year".to_owned(),
            }
        );
    }

    #[test]
    fn unknown_params_are_rejected() {
        let error = expand("/blog/{slug}", &[("slug", "hi"), ("id", "1")]).unwrap_err();
        assert_eq!(
            error,
            StaticExportError::UnknownParam {
                route: "/blog/{slug}".to_owned(),
                name: "id".to_owned(),
            }
        );
    }

    #[test]
    fn a_fixed_page_rejects_any_param() {
        let error = expand("/about", &[("id", "1")]).unwrap_err();
        assert!(matches!(error, StaticExportError::UnknownParam { .. }));
    }

    #[test]
    fn repeated_params_are_rejected() {
        let error = expand("/blog/{slug}", &[("slug", "a"), ("slug", "b")]).unwrap_err();
        assert_eq!(
            error,
            StaticExportError::DuplicateParam {
                route: "/blog/{slug}".to_owned(),
                name: "slug".to_owned(),
            }
        );
    }

    #[test]
    fn empty_values_are_rejected() {
        let error = expand("/blog/{slug}", &[("slug", "")]).unwrap_err();
        assert!(matches!(error, StaticExportError::EmptyParam { .. }));
    }

    #[test]
    fn a_param_may_not_span_segments() {
        let error = expand("/files/{name}", &[("name", "a/b")]).unwrap_err();
        assert!(matches!(error, StaticExportError::InvalidParam { .. }));
    }

    #[test]
    fn a_catch_all_may_span_segments() {
        assert_eq!(
            expand("/files/{*rest}", &[("rest", "a/b/c")]).unwrap(),
            "/files/a/b/c"
        );
        // Surrounding slashes would double up against the separator.
        assert_eq!(
            expand("/files/{*rest}", &[("rest", "/a/b/")]).unwrap(),
            "/files/a/b"
        );
    }

    // -- collect_paths --

    fn collect(pages: &[(&str, &[&str])]) -> Result<Vec<StaticPath>, StaticExportError> {
        collect_paths(
            pages
                .iter()
                .map(|(route, urls)| {
                    (
                        (*route).to_owned(),
                        urls.iter().map(|url| (*url).to_owned()).collect(),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn paths_are_sorted() {
        let paths = collect(&[("/{slug}", &["/b", "/a"])]).unwrap();
        assert_eq!(
            paths.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(),
            vec!["/a", "/b"]
        );
        assert_eq!(paths[0].route, "/{slug}");
    }

    #[test]
    fn one_page_generating_a_path_twice_is_rejected() {
        let error = collect(&[("/{slug}", &["/a", "/a"])]).unwrap_err();
        assert_eq!(
            error,
            StaticExportError::DuplicatePath {
                route: "/{slug}".to_owned(),
                path: "/a".to_owned(),
            }
        );
    }

    #[test]
    fn two_pages_generating_the_same_path_are_rejected() {
        let error = collect(&[("/{slug}", &["/about"]), ("/about", &["/about"])]).unwrap_err();
        assert_eq!(
            error,
            StaticExportError::ConflictingPaths {
                first: "/{slug}".to_owned(),
                second: "/about".to_owned(),
                path: "/about".to_owned(),
            }
        );
    }

    // -- StaticPage::expand --

    type ParamsFuture<'cx> = Pin<Box<dyn Future<Output = Result<Vec<StaticParams>>> + Send + 'cx>>;

    fn two_posts(_cx: &Cx) -> ParamsFuture<'_> {
        Box::pin(async {
            Ok(vec![
                StaticParams::from([("slug", "a")]),
                StaticParams::from([("slug", "b")]),
            ])
        })
    }

    fn failing(_cx: &Cx) -> ParamsFuture<'_> {
        Box::pin(async { Err(Error::from(std::io::Error::other("database unreachable"))) })
    }

    fn page(
        route: &'static str,
        serves_get: bool,
        generate: Option<GenerateStaticFn>,
    ) -> StaticPage {
        StaticPage::new(Cow::Borrowed(Path::new(route)), serves_get, generate)
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// A bare request context, which the generators run inside.
    fn cx() -> CxBuilder {
        CxBuilder::new(Arc::new(ContextMap::new()))
    }

    fn expand_page(page: &StaticPage) -> Result<Vec<String>, StaticExportError> {
        let cx = cx();
        block_on(page.expand(&cx))
    }

    #[test]
    fn a_fixed_page_exports_without_a_generator() {
        let urls = expand_page(&page("/about", true, None)).unwrap();
        assert_eq!(urls, vec!["/about".to_owned()]);
    }

    #[test]
    fn a_dynamic_page_without_a_generator_is_skipped() {
        let urls = expand_page(&page("/blog/{slug}", true, None)).unwrap();
        assert!(urls.is_empty());
    }

    #[test]
    fn a_non_get_page_without_a_generator_is_skipped() {
        let urls = expand_page(&page("/signup", false, None)).unwrap();
        assert!(urls.is_empty());
    }

    #[test]
    fn a_generator_expands_every_parameter_set() {
        let urls = expand_page(&page("/blog/{slug}", true, Some(two_posts))).unwrap();
        assert_eq!(urls, vec!["/blog/a".to_owned(), "/blog/b".to_owned()]);
    }

    #[test]
    fn a_non_get_page_with_a_generator_is_an_error() {
        let error = expand_page(&page("/signup/{id}", false, Some(two_posts))).unwrap_err();
        assert!(matches!(error, StaticExportError::NotExportable { .. }));
    }

    #[test]
    fn a_failing_generator_reports_the_page_and_the_error() {
        let error = expand_page(&page("/blog/{slug}", true, Some(failing))).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("/blog/{slug}"), "{message}");
        assert!(message.contains("database unreachable"), "{message}");
    }

    // -- StaticRoutes --

    #[test]
    fn the_listing_sorts_pages_and_assets() {
        let routes = StaticRoutes::new(
            vec![
                page("/blog/{slug}", true, Some(two_posts)),
                page("/about", true, None),
            ],
            vec![
                "/_topcoat/assets/b.css".to_owned(),
                "/_topcoat/assets/a.png".to_owned(),
            ],
        );
        let cx = cx();
        let listing = block_on(routes.listing(&cx)).unwrap();
        assert_eq!(
            listing
                .pages
                .iter()
                .map(|p| p.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/about", "/blog/a", "/blog/b"]
        );
        assert_eq!(
            listing.assets,
            vec!["/_topcoat/assets/a.png", "/_topcoat/assets/b.css"]
        );
    }

    #[test]
    fn a_page_at_a_static_file_url_is_rejected() {
        let routes = StaticRoutes::new(
            vec![page("/_topcoat/assets/a.png", true, None)],
            vec!["/_topcoat/assets/a.png".to_owned()],
        );
        let cx = cx();
        let error = block_on(routes.listing(&cx)).unwrap_err();
        assert!(
            matches!(error, StaticExportError::ConflictsWithStaticFile { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_listing_route_serves_get_only() {
        let routes = StaticRoutes::new(Vec::new(), Vec::new());
        assert_eq!(routes.methods(), Methods::Only(&[Method::GET]));
        assert_eq!(routes.path().to_string(), STATIC_ROUTES_PATH);
    }
}
