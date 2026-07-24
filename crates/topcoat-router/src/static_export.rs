use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::{Path as FsPath, PathBuf as FsPathBuf};
use std::pin::Pin;
use std::sync::Arc;

use http::Method;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use topcoat_core::context::{Cx, CxBuilder, try_request_context};
use topcoat_core::error::Result;

use crate::{Body, Path, PathBuf, PathSegment, RawPathParams, Request, Router};

const PATH_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Marks a request context created while statically exporting a page.
#[derive(Debug, Clone, Copy)]
pub struct StaticExportMarker;

/// Returns whether `cx` belongs to a static export.
#[must_use]
pub fn is_static_export(cx: &Cx) -> bool {
    try_request_context::<StaticExportMarker>(cx).is_some()
}

/// One generated value for a dynamic route segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticSegmentValue {
    components: Box<[String]>,
}

impl StaticSegmentValue {
    /// Creates a value for an ordinary one-component path parameter.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned values make the builder and iterator adapters ergonomic"
    )]
    pub fn param(value: impl ToString) -> Self {
        Self {
            components: vec![value.to_string()].into_boxed_slice(),
        }
    }

    /// Creates a value for a catch-all parameter.
    #[must_use]
    pub fn catch_all(values: impl IntoIterator<Item = impl ToString>) -> Self {
        Self {
            components: values.into_iter().map(|value| value.to_string()).collect(),
        }
    }

    fn components(&self) -> &[String] {
        &self.components
    }
}

/// A named collection of values used to expand an explicitly pathed page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticParams {
    values: Vec<(Cow<'static, str>, StaticSegmentValue)>,
}

impl StaticParams {
    /// Creates an empty parameter collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an ordinary one-component path parameter.
    #[must_use]
    pub fn param(mut self, name: impl Into<Cow<'static, str>>, value: impl ToString) -> Self {
        self.values
            .push((name.into(), StaticSegmentValue::param(value)));
        self
    }

    /// Adds a catch-all path parameter.
    #[must_use]
    pub fn catch_all(
        mut self,
        name: impl Into<Cow<'static, str>>,
        values: impl IntoIterator<Item = impl ToString>,
    ) -> Self {
        self.values
            .push((name.into(), StaticSegmentValue::catch_all(values)));
        self
    }

    fn into_map(
        self,
        route: &Path,
    ) -> std::result::Result<HashMap<Cow<'static, str>, StaticSegmentValue>, StaticExportError>
    {
        let mut values = HashMap::new();
        for (name, value) in self.values {
            if values.insert(name.clone(), value).is_some() {
                return Err(StaticExportError::new(format!(
                    "static parameters for route `{route}` contain duplicate `{name}` values"
                )));
            }
        }
        Ok(values)
    }
}

/// Future returned by an erased static segment generator.
pub type StaticSegmentGeneratorFuture<'cx> =
    Pin<Box<dyn Future<Output = Result<Vec<StaticSegmentValue>>> + Send + 'cx>>;

/// Erased async function that generates the values of one dynamic segment.
#[derive(Clone, Copy)]
pub struct StaticSegmentGenerator {
    handler: for<'cx> fn(&'cx Cx) -> StaticSegmentGeneratorFuture<'cx>,
}

impl StaticSegmentGenerator {
    /// Creates a generator from a macro-generated adapter.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(handler: for<'cx> fn(&'cx Cx) -> StaticSegmentGeneratorFuture<'cx>) -> Self {
        Self { handler }
    }

    async fn generate(&self, cx: &Cx) -> Result<Vec<StaticSegmentValue>> {
        (self.handler)(cx).await
    }

    fn identity(self) -> usize {
        self.handler as usize
    }
}

impl fmt::Debug for StaticSegmentGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticSegmentGenerator")
            .finish_non_exhaustive()
    }
}

/// Future returned by an erased explicit-page parameter generator.
pub type StaticParamsGeneratorFuture<'cx> =
    Pin<Box<dyn Future<Output = Result<Vec<StaticParams>>> + Send + 'cx>>;

/// Erased async function that generates complete parameter sets for a page.
#[derive(Clone, Copy)]
pub struct StaticParamsGenerator {
    handler: for<'cx> fn(&'cx Cx) -> StaticParamsGeneratorFuture<'cx>,
}

impl StaticParamsGenerator {
    /// Creates a generator from a macro-generated adapter.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(handler: for<'cx> fn(&'cx Cx) -> StaticParamsGeneratorFuture<'cx>) -> Self {
        Self { handler }
    }

    async fn generate(&self, cx: &Cx) -> Result<Vec<StaticParams>> {
        (self.handler)(cx).await
    }
}

impl fmt::Debug for StaticParamsGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticParamsGenerator")
            .finish_non_exhaustive()
    }
}

/// A segment in the static expansion plan of a module-derived page.
#[derive(Debug, Clone)]
pub(crate) enum StaticPageSegment {
    Static(String),
    Group,
    Param {
        name: Cow<'static, str>,
        generator: Option<StaticSegmentGenerator>,
    },
    CatchAll {
        name: Cow<'static, str>,
        generator: Option<StaticSegmentGenerator>,
    },
}

/// How a page's concrete export URLs are generated.
#[derive(Debug, Clone)]
pub(crate) enum StaticPageSource {
    Module(Vec<StaticPageSegment>),
    Explicit(StaticParamsGenerator),
}

/// A page retained by the router for static path generation.
#[derive(Debug, Clone)]
pub(crate) struct StaticPage {
    path: PathBuf,
    source: Option<StaticPageSource>,
}

impl StaticPage {
    pub(crate) fn new(path: PathBuf, source: Option<StaticPageSource>) -> Self {
        Self { path, source }
    }
}

/// A file copied verbatim into a static export.
#[derive(Debug, Clone)]
pub struct StaticFile {
    url_path: String,
    source_path: FsPathBuf,
}

impl StaticFile {
    /// Creates a static file mounted at `url_path`.
    #[must_use]
    pub fn new(url_path: impl Into<String>, source_path: impl Into<FsPathBuf>) -> Self {
        Self {
            url_path: url_path.into(),
            source_path: source_path.into(),
        }
    }

    /// Returns the public URL path of the file.
    #[must_use]
    pub fn url_path(&self) -> &str {
        &self.url_path
    }

    /// Returns the source file copied during export.
    #[must_use]
    pub fn source_path(&self) -> &FsPath {
        &self.source_path
    }
}

/// One concrete page URL selected for static rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticPath {
    url_path: String,
    route: PathBuf,
}

impl StaticPath {
    /// Returns the concrete URL path.
    #[must_use]
    pub fn url_path(&self) -> &str {
        &self.url_path
    }

    /// Returns the route pattern that generated the URL.
    #[must_use]
    pub fn route(&self) -> &Path {
        &self.route
    }
}

/// An error encountered while selecting the URLs of a static export.
#[derive(Debug)]
pub struct StaticExportError {
    message: String,
}

impl StaticExportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for StaticExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StaticExportError {}

#[derive(Clone)]
struct Candidate {
    components: Vec<String>,
    params: Vec<(String, String)>,
}

impl Candidate {
    fn root() -> Self {
        Self {
            components: Vec::new(),
            params: Vec::new(),
        }
    }

    fn url_path(&self) -> String {
        if self.components.is_empty() {
            return "/".to_owned();
        }
        let mut path = String::new();
        for component in &self.components {
            path.push('/');
            path.push_str(&utf8_percent_encode(component, PATH_SEGMENT_ENCODE_SET).to_string());
        }
        path
    }
}

impl Router {
    /// Generates every concrete URL selected for static page rendering.
    ///
    /// Fixed GET pages are included automatically. Dynamic module-derived
    /// pages use their segment generators, while explicitly pathed pages use
    /// the generator declared on `#[page]`.
    ///
    /// # Errors
    ///
    /// Returns an error when a dynamic segment has no generator, a generator
    /// fails, generated values do not fit their route, or two pages generate
    /// the same concrete URL.
    pub async fn generate_static_paths(
        &self,
    ) -> std::result::Result<Vec<StaticPath>, StaticExportError> {
        let mut generated = Vec::new();
        let mut segment_cache = HashMap::new();

        for page in &self.static_pages {
            let paths = match &page.source {
                Some(StaticPageSource::Module(segments)) => {
                    self.expand_module_page(&page.path, segments, &mut segment_cache)
                        .await?
                }
                Some(StaticPageSource::Explicit(generator)) => {
                    self.expand_explicit_page(&page.path, *generator).await?
                }
                None => Self::expand_automatic_page(&page.path)?,
            };
            generated.extend(paths.into_iter().map(|url_path| StaticPath {
                url_path,
                route: page.path.clone(),
            }));
        }

        generated.sort_by(|left, right| left.url_path.cmp(&right.url_path));
        let mut seen = HashSet::new();
        for path in &generated {
            if !seen.insert(path.url_path.clone()) {
                return Err(StaticExportError::new(format!(
                    "multiple pages generate the static URL `{}`",
                    path.url_path
                )));
            }
        }
        Ok(generated)
    }

    /// Returns every file registered for copying into a static export.
    #[must_use]
    pub fn static_files(&self) -> &[StaticFile] {
        &self.static_files
    }

    async fn expand_module_page(
        &self,
        route: &Path,
        segments: &[StaticPageSegment],
        segment_cache: &mut HashMap<(usize, String), Vec<StaticSegmentValue>>,
    ) -> std::result::Result<Vec<String>, StaticExportError> {
        let mut candidates = vec![Candidate::root()];

        for segment in segments {
            match segment {
                StaticPageSegment::Static(value) => {
                    for candidate in &mut candidates {
                        candidate.components.push(value.clone());
                    }
                }
                StaticPageSegment::Group => {}
                StaticPageSegment::Param { name, generator } => {
                    candidates = self
                        .expand_generated_segment(
                            route,
                            candidates,
                            name.as_ref(),
                            *generator,
                            false,
                            segment_cache,
                        )
                        .await?;
                }
                StaticPageSegment::CatchAll { name, generator } => {
                    candidates = self
                        .expand_generated_segment(
                            route,
                            candidates,
                            name.as_ref(),
                            *generator,
                            true,
                            segment_cache,
                        )
                        .await?;
                }
            }
        }

        Ok(candidates
            .into_iter()
            .map(|candidate| candidate.url_path())
            .collect())
    }

    async fn expand_generated_segment(
        &self,
        route: &Path,
        candidates: Vec<Candidate>,
        name: &str,
        generator: Option<StaticSegmentGenerator>,
        catch_all: bool,
        segment_cache: &mut HashMap<(usize, String), Vec<StaticSegmentValue>>,
    ) -> std::result::Result<Vec<Candidate>, StaticExportError> {
        let Some(generator) = generator else {
            return Err(StaticExportError::new(format!(
                "dynamic segment `{name}` in route `{route}` has no `generate_static` function"
            )));
        };
        let mut expanded = Vec::new();

        for candidate in candidates {
            let cache_key = (generator.identity(), candidate.url_path());
            let values = if let Some(values) = segment_cache.get(&cache_key) {
                values.clone()
            } else {
                let cx = self.generator_context(&candidate);
                let values = generator.generate(&cx).await.map_err(|error| {
                    StaticExportError::new(format!(
                        "failed to generate static values for `{name}` in route `{route}`: {error}"
                    ))
                })?;
                segment_cache.insert(cache_key, values.clone());
                values
            };
            for value in values {
                validate_segment_value(route, name, &value, catch_all)?;
                let mut next = candidate.clone();
                next.components.extend(value.components().iter().cloned());
                next.params
                    .push((name.to_owned(), value.components().join("/")));
                expanded.push(next);
            }
        }

        Ok(expanded)
    }

    async fn expand_explicit_page(
        &self,
        route: &Path,
        generator: StaticParamsGenerator,
    ) -> std::result::Result<Vec<String>, StaticExportError> {
        let cx = self.generator_context(&Candidate::root());
        let generated = generator.generate(&cx).await.map_err(|error| {
            StaticExportError::new(format!(
                "failed to generate static parameters for route `{route}`: {error}"
            ))
        })?;
        generated
            .into_iter()
            .map(|params| expand_explicit_params(route, params))
            .collect()
    }

    fn expand_automatic_page(route: &Path) -> std::result::Result<Vec<String>, StaticExportError> {
        let mut candidate = Candidate::root();
        for segment in route.segments() {
            match segment {
                PathSegment::Static(value) => candidate.components.push(value.to_owned()),
                PathSegment::Group(_) => {}
                PathSegment::Param(name) | PathSegment::CatchAll(name) => {
                    return Err(StaticExportError::new(format!(
                        "dynamic segment `{name}` in explicit route `{route}` requires \
                         `generate_static` on `#[page]`"
                    )));
                }
            }
        }
        Ok(vec![candidate.url_path()])
    }

    fn generator_context(&self, candidate: &Candidate) -> Cx {
        let uri = candidate.url_path();
        let (parts, _) = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("generated static request must be valid")
            .into_parts();
        let keys = candidate
            .params
            .iter()
            .map(|(name, _)| Arc::<str>::from(name.as_str()));
        let values = candidate.params.iter().map(|(_, value)| value.as_str());

        let mut cx = CxBuilder::new(self.app_context.clone());
        cx.insert(RawPathParams::from_pairs(keys.zip(values)));
        cx.insert(parts);
        cx.insert(StaticExportMarker);
        cx.build()
    }
}

fn expand_explicit_params(
    route: &Path,
    params: StaticParams,
) -> std::result::Result<String, StaticExportError> {
    let mut values = params.into_map(route)?;
    let mut candidate = Candidate::root();

    for segment in route.segments() {
        match segment {
            PathSegment::Static(value) => candidate.components.push(value.to_owned()),
            PathSegment::Group(_) => {}
            PathSegment::Param(name) => {
                let value = take_param(route, &mut values, name)?;
                validate_segment_value(route, name, &value, false)?;
                candidate
                    .components
                    .extend(value.components().iter().cloned());
            }
            PathSegment::CatchAll(name) => {
                let value = take_param(route, &mut values, name)?;
                validate_segment_value(route, name, &value, true)?;
                candidate
                    .components
                    .extend(value.components().iter().cloned());
            }
        }
    }

    if let Some(name) = values.keys().next() {
        return Err(StaticExportError::new(format!(
            "static parameters for route `{route}` contain unknown parameter `{name}`"
        )));
    }

    let url_path = candidate.url_path();
    if !route.matches(&url_path) {
        return Err(StaticExportError::new(format!(
            "generated URL `{url_path}` does not match route `{route}`"
        )));
    }
    Ok(url_path)
}

fn take_param(
    route: &Path,
    values: &mut HashMap<Cow<'static, str>, StaticSegmentValue>,
    name: &str,
) -> std::result::Result<StaticSegmentValue, StaticExportError> {
    values.remove(name).ok_or_else(|| {
        StaticExportError::new(format!(
            "static parameters for route `{route}` are missing `{name}`"
        ))
    })
}

fn validate_segment_value(
    route: &Path,
    name: &str,
    value: &StaticSegmentValue,
    catch_all: bool,
) -> std::result::Result<(), StaticExportError> {
    let expected = if catch_all {
        "one or more"
    } else {
        "exactly one"
    };
    let valid_len = if catch_all {
        !value.components().is_empty()
    } else {
        value.components().len() == 1
    };
    if !valid_len {
        return Err(StaticExportError::new(format!(
            "static value for `{name}` in route `{route}` must contain {expected} path component"
        )));
    }
    if value.components().iter().any(String::is_empty) {
        return Err(StaticExportError::new(format!(
            "static value for `{name}` in route `{route}` contains an empty path component"
        )));
    }
    Ok(())
}
