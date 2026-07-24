use std::fmt::{self, Display};

/// A problem that stops a page from being exported to a static site.
///
/// Every variant names the page's route path, so a failure points at the page
/// whose `generate_static` function produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StaticExportError {
    /// A generated parameter set left one of the page's dynamic segments
    /// unnamed.
    MissingParam {
        /// The page's route path, e.g. `/blog/{year}/{slug}`.
        route: String,
        /// The name of the unnamed segment.
        name: String,
    },
    /// A generated parameter set named something the page's path does not
    /// declare.
    UnknownParam {
        /// The page's route path.
        route: String,
        /// The unrecognized parameter name.
        name: String,
    },
    /// A generated parameter set named the same parameter twice.
    DuplicateParam {
        /// The page's route path.
        route: String,
        /// The repeated parameter name.
        name: String,
    },
    /// A generated parameter had an empty value, which addresses no URL
    /// segment.
    EmptyParam {
        /// The page's route path.
        route: String,
        /// The parameter with the empty value.
        name: String,
    },
    /// A `{name}` parameter's value spanned more than one URL segment. Only a
    /// `{*name}` catch-all may contain a `/`.
    InvalidParam {
        /// The page's route path.
        route: String,
        /// The offending parameter name.
        name: String,
        /// The value that could not be placed in a single segment.
        value: String,
    },
    /// One page generated the same URL twice.
    DuplicatePath {
        /// The page's route path.
        route: String,
        /// The URL generated more than once.
        path: String,
    },
    /// Two different pages generated the same URL.
    ConflictingPaths {
        /// The route path of the page that generated the URL first.
        first: String,
        /// The route path of the page that generated it again.
        second: String,
        /// The URL both pages generated.
        path: String,
    },
    /// A page generated a URL that is already served as a static file, so the
    /// two would be written to the same place.
    ConflictsWithStaticFile {
        /// The page's route path.
        route: String,
        /// The URL both claim.
        path: String,
    },
    /// A page declared `generate_static` but does not answer `GET`, so it has
    /// no static representation.
    NotExportable {
        /// The page's route path.
        route: String,
    },
    /// A page's `generate_static` function returned an error.
    Generator {
        /// The page's route path.
        route: String,
        /// The error the generator reported.
        message: String,
    },
}

impl Display for StaticExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingParam { route, name } => write!(
                f,
                "page `{route}`: generated parameters do not include `{name}`; \
                 every set must name each dynamic segment of the page's path, \
                 including the segments its parent modules contribute"
            ),
            Self::UnknownParam { route, name } => write!(
                f,
                "page `{route}`: generated parameters include `{name}`, which the page's path does not declare"
            ),
            Self::DuplicateParam { route, name } => write!(
                f,
                "page `{route}`: generated parameters name `{name}` more than once"
            ),
            Self::EmptyParam { route, name } => write!(
                f,
                "page `{route}`: generated parameter `{name}` is empty, which matches no URL segment"
            ),
            Self::InvalidParam { route, name, value } => write!(
                f,
                "page `{route}`: generated parameter `{name}` is `{value}`, which spans more than \
                 one URL segment; only a `{{*{name}}}` catch-all may contain a `/`"
            ),
            Self::DuplicatePath { route, path } => write!(
                f,
                "page `{route}` generates the static path `{path}` more than once"
            ),
            Self::ConflictingPaths {
                first,
                second,
                path,
            } => write!(
                f,
                "pages `{first}` and `{second}` both generate the static path `{path}`"
            ),
            Self::ConflictsWithStaticFile { route, path } => write!(
                f,
                "page `{route}` generates the static path `{path}`, which is already served as a static file"
            ),
            Self::NotExportable { route } => write!(
                f,
                "page `{route}` declares `generate_static` but does not answer `GET`, \
                 so it cannot be exported"
            ),
            Self::Generator { route, message } => {
                write!(f, "page `{route}`: `generate_static` failed: {message}")
            }
        }
    }
}

impl std::error::Error for StaticExportError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_names_the_page_and_the_parameter() {
        let error = StaticExportError::MissingParam {
            route: "/blog/{slug}".to_owned(),
            name: "slug".to_owned(),
        };
        let message = error.to_string();
        assert!(message.contains("/blog/{slug}"), "{message}");
        assert!(message.contains("`slug`"), "{message}");
    }

    #[test]
    fn conflicting_paths_names_both_pages() {
        let error = StaticExportError::ConflictingPaths {
            first: "/blog/{slug}".to_owned(),
            second: "/blog/hello".to_owned(),
            path: "/blog/hello".to_owned(),
        };
        let message = error.to_string();
        assert!(message.contains("/blog/{slug}"), "{message}");
        assert!(message.contains("/blog/hello"), "{message}");
    }

    #[test]
    fn invalid_param_suggests_a_catch_all() {
        let error = StaticExportError::InvalidParam {
            route: "/files/{name}".to_owned(),
            name: "name".to_owned(),
            value: "a/b".to_owned(),
        };
        assert!(error.to_string().contains("{*name}"), "{error}");
    }
}
