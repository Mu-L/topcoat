use std::fmt::Display;
use std::pin::Pin;

use topcoat_core::{context::Cx, error::Result};

/// One complete set of path parameters a page is exported for.
///
/// A generator named with `#[page(generate_static = ...)]` returns one
/// `StaticParams` per page the static export should produce. Each set must name
/// **every** dynamic segment in the page's path, including the segments
/// contributed by parent modules: a page at `/blog/{year}/{slug}` is exported
/// once per set naming both `year` and `slug`, whichever module each segment
/// came from.
///
/// Values are rendered into the URL as-is. A `{name}` parameter takes a single
/// non-empty URL segment, so its value must not contain a `/`; a `{*name}`
/// catch-all stands for the remainder of the URL, so its value may.
///
/// # Examples
///
/// ```rust
/// use topcoat::router::StaticParams;
///
/// let params = StaticParams::from([("year", "2026"), ("slug", "hello")]);
/// assert_eq!(params.get("year"), Some("2026"));
///
/// // Values are anything that displays, so ids need no conversion.
/// let params = StaticParams::new().with("id", 42);
/// assert_eq!(params.get("id"), Some("42"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticParams {
    /// The `(name, value)` pairs in insertion order. A [`Vec`] rather than a
    /// map so a name inserted twice is reported as an error instead of
    /// silently overwriting.
    params: Vec<(String, String)>,
}

impl StaticParams {
    /// Creates an empty parameter set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a parameter, returning the set, for building one in an expression.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Display) -> Self {
        self.insert(name, value);
        self
    }

    /// Adds a parameter to this set.
    pub fn insert(&mut self, name: impl Into<String>, value: impl Display) -> &mut Self {
        self.params.push((name.into(), value.to_string()));
        self
    }

    /// Returns the value of `name`, or [`None`] when the set does not name it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Iterates over the `(name, value)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// The number of parameters in this set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns `true` if this set names no parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }
}

impl<K, V> FromIterator<(K, V)> for StaticParams
where
    K: Into<String>,
    V: Display,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let mut params = Self::new();
        for (name, value) in iter {
            params.insert(name, value);
        }
        params
    }
}

impl<K, V, const N: usize> From<[(K, V); N]> for StaticParams
where
    K: Into<String>,
    V: Display,
{
    fn from(value: [(K, V); N]) -> Self {
        value.into_iter().collect()
    }
}

/// The async function backing `#[page(generate_static = ...)]`.
///
/// The attribute accepts the name of an `async fn` taking the request
/// [`Cx`] and returning every parameter set the page is exported for:
///
/// ```rust
/// # use topcoat::{Result, context::Cx, router::{StaticParams, page}, view::view};
/// async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> {
///     Ok(vec![StaticParams::from([("slug", "hello")])])
/// }
///
/// #[page("/blog/{slug}", generate_static = generate_static_params)]
/// async fn post() -> Result {
///     view! { <h1>"Post"</h1> }
/// }
/// ```
///
/// The generator runs inside a regular request to the development-only static
/// route listing, so it can read the app context and any other request-scoped
/// state through `cx`.
pub type GenerateStaticFn =
    for<'cx> fn(
        cx: &'cx Cx,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<StaticParams>>> + Send + 'cx>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let params = StaticParams::new();
        assert!(params.is_empty());
        assert_eq!(params.len(), 0);
        assert_eq!(params.get("id"), None);
    }

    #[test]
    fn with_builds_a_set() {
        let params = StaticParams::new().with("year", 2026).with("slug", "hello");
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("year"), Some("2026"));
        assert_eq!(params.get("slug"), Some("hello"));
    }

    #[test]
    fn from_array_preserves_order() {
        let params = StaticParams::from([("a", "1"), ("b", "2")]);
        assert_eq!(
            params.iter().collect::<Vec<_>>(),
            vec![("a", "1"), ("b", "2")]
        );
    }

    #[test]
    fn collects_from_an_iterator() {
        let params: StaticParams = vec![("id", 7)].into_iter().collect();
        assert_eq!(params.get("id"), Some("7"));
    }

    #[test]
    fn a_repeated_name_is_kept_rather_than_overwritten() {
        // Resolution reports the repeat as an error, so both entries survive.
        let params = StaticParams::new().with("id", 1).with("id", 2);
        assert_eq!(params.len(), 2);
    }
}
