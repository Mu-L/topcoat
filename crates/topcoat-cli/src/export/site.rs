use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;

use super::output::{OutputFormat, OutputPathError, backup_dir, file_for_asset, staging_dir};

/// URL path of the application's development-only static route listing.
const STATIC_ROUTES_PATH: &str = "/_topcoat/routes/static";

/// Header marking a request as one made by the export, so a page renders
/// without the development tooling's own additions.
const EXPORT_HEADER: &str = "x-topcoat-export";

/// Characters escaped when a listed URL is turned back into a request.
///
/// The listing carries decoded URL paths, since those are what the exported
/// files are named after, so anything with a meaning in a URL is escaped on the
/// way back out. `/` is left alone: it separates the path's segments.
const PATH_ESCAPES: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'^')
    .add(b'|')
    .add(b'\\');

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    /// The number of pages rendered into the site.
    pub pages: usize,
    /// The number of assets copied into the site.
    pub assets: usize,
}

/// One page in the application's static route listing.
#[derive(Debug, Deserialize)]
struct ListedPage {
    /// The concrete URL to render, e.g. `/blog/hello`.
    path: String,
    /// The route the page is registered at, e.g. `/blog/{slug}`.
    route: String,
}

/// The application's static route listing.
#[derive(Debug, Deserialize)]
struct Listing {
    pages: Vec<ListedPage>,
    assets: Vec<String>,
}

/// Export the site the application at `base_url` describes into `out`.
///
/// The application is asked what it can export, every listed page is rendered
/// through a regular HTTP request, and every listed asset is copied with the
/// bytes the application serves. Nothing lands in `out` until the whole site
/// has been written: the files go to a staging directory beside it, which
/// replaces `out` only once every page and asset succeeded. A failed export
/// therefore leaves a previous, working one in place.
///
/// This blocks on HTTP and filesystem I/O, so callers on an async runtime
/// should run it on a blocking task.
///
/// # Errors
///
/// Returns an [`ExportError`] if the listing cannot be read, if any page or
/// asset fails to fetch, or if the site cannot be written to disk.
pub fn export_site(
    base_url: &str,
    out: &Path,
    format: OutputFormat,
) -> Result<ExportSummary, ExportError> {
    let agent = agent();
    let listing = fetch_listing(&agent, base_url)?;

    let staging = staging_dir(out);
    // A staging directory left behind by an interrupted run holds stale files.
    remove_dir(&staging)?;
    create_dir(&staging)?;

    let result = write_site(&agent, base_url, &listing, &staging, format);
    if result.is_err() {
        // The previous export, if any, is still whole; drop the partial one.
        let _ = fs::remove_dir_all(&staging);
    }
    result?;

    swap(&staging, out)?;

    Ok(ExportSummary {
        pages: listing.pages.len(),
        assets: listing.assets.len(),
    })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(concat!("topcoat-cli/", env!("CARGO_PKG_VERSION")))
        // A failed page must be reported with its status and, for the listing,
        // its body, so responses are inspected rather than raised as errors.
        .http_status_as_error(false)
        .build()
        .into()
}

/// Ask the application what a static export covers.
fn fetch_listing(agent: &ureq::Agent, base_url: &str) -> Result<Listing, ExportError> {
    let url = format!("{base_url}{STATIC_ROUTES_PATH}");
    let (status, body) = get(agent, &url)?;

    match status {
        200 => serde_json::from_slice(&body).map_err(|source| ExportError::Listing {
            source: Box::new(source),
        }),
        // The route only exists while the application runs under the topcoat
        // tooling, which is exactly how the export starts it.
        404 => Err(ExportError::ListingUnavailable),
        // The application rejected its own export: the body says why.
        500 => Err(ExportError::Rejected {
            message: String::from_utf8_lossy(&body).into_owned(),
        }),
        status => Err(ExportError::Status { url, status }),
    }
}

/// Render every page and copy every asset into `staging`.
fn write_site(
    agent: &ureq::Agent,
    base_url: &str,
    listing: &Listing,
    staging: &Path,
    format: OutputFormat,
) -> Result<(), ExportError> {
    // Two URLs the application serves separately can name one file: a page at
    // `/about` written to `about/index.html` and an asset at
    // `/about/index.html`, say. The whole site is laid out before anything is
    // fetched so a collision is reported rather than one URL overwriting the
    // other.
    let mut claimed = Claimed::default();
    let pages = listing
        .pages
        .iter()
        .map(|page| {
            let file = format.file_for(&page.path)?;
            claimed.claim(&file, &page.path)?;
            Ok((page, file))
        })
        .collect::<Result<Vec<_>, ExportError>>()?;
    let assets = listing
        .assets
        .iter()
        .map(|asset| {
            let file = file_for_asset(asset)?;
            claimed.claim(&file, asset)?;
            Ok((asset, file))
        })
        .collect::<Result<Vec<_>, ExportError>>()?;

    for (page, file) in pages {
        // A page that fails names the route it came from, since that is what
        // the application declared and what has to be fixed.
        let body = fetch(agent, base_url, &page.path).map_err(|source| ExportError::Page {
            path: page.path.clone(),
            route: page.route.clone(),
            source: Box::new(source),
        })?;
        write_file(&staging.join(file), &body)?;
    }

    for (asset, file) in assets {
        write_file(&staging.join(file), &fetch(agent, base_url, asset)?)?;
    }

    Ok(())
}

/// The files the site's URLs have been laid out into, so that no two URLs are
/// written to the same one.
#[derive(Default)]
struct Claimed {
    files: HashMap<PathBuf, String>,
}

impl Claimed {
    /// Record that `url` is written to `file`.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Collision`] if another URL already writes there.
    fn claim(&mut self, file: &Path, url: &str) -> Result<(), ExportError> {
        if let Some(first) = self.files.get(file) {
            return Err(ExportError::Collision {
                file: file.to_path_buf(),
                first: first.clone(),
                second: url.to_owned(),
            });
        }
        self.files.insert(file.to_path_buf(), url.to_owned());
        Ok(())
    }
}

/// Fetch one URL path from the application, requiring a successful response.
fn fetch(agent: &ureq::Agent, base_url: &str, path: &str) -> Result<Vec<u8>, ExportError> {
    let url = format!("{base_url}{}", utf8_percent_encode(path, PATH_ESCAPES));
    let (status, body) = get(agent, &url)?;
    if status != 200 {
        return Err(ExportError::Status {
            url: path.to_owned(),
            status,
        });
    }
    Ok(body)
}

/// Perform one GET, returning the status and the whole body.
fn get(agent: &ureq::Agent, url: &str) -> Result<(u16, Vec<u8>), ExportError> {
    let response = agent
        .get(url)
        .header(EXPORT_HEADER, "1")
        .call()
        .map_err(|source| ExportError::Request {
            url: url.to_owned(),
            source: Box::new(source),
        })?;

    let status = response.status().as_u16();
    let mut body = Vec::new();
    response
        .into_body()
        .as_reader()
        .read_to_end(&mut body)
        .map_err(|source| ExportError::Request {
            url: url.to_owned(),
            source: Box::new(source.into()),
        })?;

    Ok((status, body))
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), ExportError> {
    if let Some(parent) = path.parent() {
        create_dir(parent)?;
    }
    fs::write(path, contents).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir(path: &Path) -> Result<(), ExportError> {
    fs::create_dir_all(path).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_dir(path: &Path) -> Result<(), ExportError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ExportError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Move the finished staging directory into place.
///
/// A previous export is moved aside first and only deleted once the fresh one
/// has taken its name, so an interruption leaves one whole export on disk
/// rather than none.
fn swap(staging: &Path, out: &Path) -> Result<(), ExportError> {
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        create_dir(parent)?;
    }

    let backup = backup_dir(out);
    remove_dir(&backup)?;

    let previous = out.exists();
    if previous {
        rename(out, &backup)?;
    }

    match rename(staging, out) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(error) => {
            // Put the previous export back rather than leave nothing behind.
            if previous {
                let _ = fs::rename(&backup, out);
            }
            Err(error)
        }
    }
}

fn rename(from: &Path, to: &Path) -> Result<(), ExportError> {
    fs::rename(from, to).map_err(|source| ExportError::Io {
        path: to.to_path_buf(),
        source,
    })
}

/// A reason an export could not be completed.
#[derive(Debug)]
pub enum ExportError {
    /// A request to the application failed to complete.
    Request {
        /// The URL that was requested.
        url: String,
        /// The underlying transport error.
        source: Box<ureq::Error>,
    },
    /// A request completed with a status other than `200 OK`.
    Status {
        /// The URL that was requested.
        url: String,
        /// The status the application answered with.
        status: u16,
    },
    /// The application does not serve the static route listing.
    ListingUnavailable,
    /// The listing could not be parsed.
    Listing {
        /// The underlying deserialization error.
        source: Box<serde_json::Error>,
    },
    /// The application reported that its pages cannot be exported as they
    /// stand.
    Rejected {
        /// The explanation the application gave.
        message: String,
    },
    /// A page could not be rendered.
    Page {
        /// The URL the page was requested at.
        path: String,
        /// The route the page is registered at.
        route: String,
        /// What went wrong fetching it.
        source: Box<ExportError>,
    },
    /// A listed URL cannot be written to a file.
    Path(OutputPathError),
    /// Two listed URLs are written to the same file.
    Collision {
        /// The file both URLs name.
        file: PathBuf,
        /// The URL that claimed it first.
        first: String,
        /// The URL that would have overwritten it.
        second: String,
    },
    /// The site could not be written to disk.
    Io {
        /// The file or directory being written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

impl From<OutputPathError> for ExportError {
    fn from(value: OutputPathError) -> Self {
        Self::Path(value)
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request { url, source } => write!(f, "request to `{url}` failed: {source}"),
            Self::Status { url, status } => {
                write!(f, "`{url}` answered {status}, expected 200")
            }
            Self::ListingUnavailable => f.write_str(
                "the application does not serve `/_topcoat/routes/static`; \
                 it is available only while the application runs under the topcoat tooling",
            ),
            Self::Listing { source } => {
                write!(f, "could not read the static route listing: {source}")
            }
            Self::Rejected { message } => write!(f, "{message}"),
            Self::Page {
                path,
                route,
                source,
            } => {
                write!(f, "page `{route}` at `{path}`: {source}")
            }
            Self::Path(error) => error.fmt(f),
            Self::Collision {
                file,
                first,
                second,
            } => write!(
                f,
                "`{first}` and `{second}` are both written to {}",
                file.display()
            ),
            Self::Io { path, source } => write!(f, "failed to write {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ExportError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_parses() {
        let listing: Listing = serde_json::from_str(
            r#"{"pages":[{"path":"/","route":"/"}],"assets":["/_topcoat/assets/a.png"]}"#,
        )
        .unwrap();
        assert_eq!(listing.pages.len(), 1);
        assert_eq!(listing.pages[0].path, "/");
        assert_eq!(listing.pages[0].route, "/");
        assert_eq!(listing.assets, vec!["/_topcoat/assets/a.png"]);
    }

    #[test]
    fn request_paths_are_escaped_but_keep_their_separators() {
        let encoded = utf8_percent_encode("/blog/hello world", PATH_ESCAPES).to_string();
        assert_eq!(encoded, "/blog/hello%20world");
    }

    #[test]
    fn distinct_files_are_all_claimed() {
        let mut claimed = Claimed::default();
        claimed
            .claim(Path::new("about/index.html"), "/about")
            .unwrap();
        claimed
            .claim(Path::new("_topcoat/assets/a.png"), "/_topcoat/assets/a.png")
            .unwrap();
    }

    #[test]
    fn a_second_url_cannot_claim_a_taken_file() {
        let mut claimed = Claimed::default();
        claimed
            .claim(Path::new("about/index.html"), "/about")
            .unwrap();
        let error = claimed
            .claim(Path::new("about/index.html"), "/about/index.html")
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("`/about`"), "{message}");
        assert!(message.contains("`/about/index.html`"), "{message}");
    }

    #[test]
    fn a_rejection_reports_the_applications_own_message() {
        let error = ExportError::Rejected {
            message: "page `/blog/{slug}`: generated parameters do not include `slug`".to_owned(),
        };
        assert!(
            error.to_string().contains("do not include `slug`"),
            "{error}"
        );
    }
}
