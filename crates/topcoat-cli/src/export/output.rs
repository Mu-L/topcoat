use std::fmt;
use std::path::{Path, PathBuf};

use clap::ValueEnum;

/// How a page URL becomes a file in the exported site.
///
/// Static hosts serve a directory URL either by looking for an `index.html`
/// inside a directory of that name, or by appending `.html` to the URL. The
/// two conventions cover essentially every host, so the export writes one or
/// the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Clean directory URLs: `/about` is written to `about/index.html`, so the
    /// site serves it at `/about` without a file extension. This is the
    /// default, and what GitHub Pages, Netlify, Cloudflare Pages, and S3
    /// website hosting expect.
    #[default]
    Directory,
    /// HTML files: `/about` is written to `about.html`, for hosts that resolve
    /// an extensionless URL by appending `.html`.
    File,
}

impl OutputFormat {
    /// The file, relative to the output directory, that `url` is written to.
    ///
    /// The root URL is always `index.html`, whichever format is in use, since
    /// a site's entry point has no name to take.
    ///
    /// # Errors
    ///
    /// Returns [`OutputPathError`] if a URL segment cannot be written to a
    /// file safely, which a page's generated parameters could otherwise cause.
    pub fn file_for(self, url: &str) -> Result<PathBuf, OutputPathError> {
        let segments = segments(url)?;
        // The root URL names no file of its own, so it is the site's index
        // whichever convention the rest of the pages follow.
        let Some((last, parents)) = segments.split_last() else {
            return Ok(PathBuf::from("index.html"));
        };

        let mut path: PathBuf = parents.iter().collect();
        match self {
            Self::Directory => {
                path.push(last);
                path.push("index.html");
            }
            Self::File => path.push(format!("{last}.html")),
        }
        Ok(path)
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Directory => "directory",
            Self::File => "file",
        })
    }
}

/// The file, relative to the output directory, that a verbatim URL (an asset)
/// is written to.
///
/// Assets keep the exact URL the application serves them at, so their
/// content-hashed names line up with the links in the exported pages.
///
/// # Errors
///
/// Returns [`OutputPathError`] if a URL segment cannot be written to a file
/// safely.
pub fn file_for_asset(url: &str) -> Result<PathBuf, OutputPathError> {
    let segments = segments(url)?;
    if segments.is_empty() {
        return Err(OutputPathError::Empty {
            url: url.to_owned(),
        });
    }
    Ok(segments.iter().collect())
}

/// Splits a URL path into the segments naming its file, rejecting anything
/// that would escape the output directory or name an unwritable file.
///
/// The URLs come from the application's own route listing, but a page's
/// generated parameters are application data: `..` in one would otherwise
/// write outside the output directory.
fn segments(url: &str) -> Result<Vec<&str>, OutputPathError> {
    let mut segments = Vec::new();
    for segment in url.trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." || segment.contains(['\\', '\0']) {
            return Err(OutputPathError::UnsafeSegment {
                url: url.to_owned(),
                segment: segment.to_owned(),
            });
        }
        segments.push(segment);
    }
    Ok(segments)
}

/// A URL that cannot be turned into a file in the exported site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPathError {
    /// A URL segment would escape the output directory or cannot name a file.
    UnsafeSegment {
        /// The URL the segment came from.
        url: String,
        /// The rejected segment.
        segment: String,
    },
    /// A verbatim URL had no segments at all, so it names no file.
    Empty {
        /// The offending URL.
        url: String,
    },
}

impl fmt::Display for OutputPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeSegment { url, segment } => write!(
                f,
                "`{url}` cannot be written to a file: the segment `{segment}` would escape the output directory"
            ),
            Self::Empty { url } => write!(f, "`{url}` names no file to write"),
        }
    }
}

impl std::error::Error for OutputPathError {}

/// The staging directory an export is written to before it replaces `out`.
///
/// It sits beside the output directory so the final move is a rename within
/// one filesystem, which either happens or does not.
#[must_use]
pub fn staging_dir(out: &Path) -> PathBuf {
    sibling(out, "topcoat-export")
}

/// Where the previous export is moved to while the fresh one takes its place.
#[must_use]
pub fn backup_dir(out: &Path) -> PathBuf {
    sibling(out, "topcoat-export-prev")
}

fn sibling(out: &Path, suffix: &str) -> PathBuf {
    let name = out.file_name().map_or_else(
        || "export".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    out.with_file_name(format!(".{name}.{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(url: &str) -> String {
        OutputFormat::Directory
            .file_for(url)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn file(url: &str) -> String {
        OutputFormat::File
            .file_for(url)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    }

    #[test]
    fn the_root_is_always_the_index() {
        assert_eq!(directory("/"), "index.html");
        assert_eq!(file("/"), "index.html");
    }

    #[test]
    fn directory_format_nests_an_index() {
        assert_eq!(directory("/about"), "about/index.html");
        assert_eq!(directory("/blog/hello"), "blog/hello/index.html");
    }

    #[test]
    fn file_format_appends_html() {
        assert_eq!(file("/about"), "about.html");
        assert_eq!(file("/blog/hello"), "blog/hello.html");
    }

    #[test]
    fn a_trailing_slash_does_not_add_a_segment() {
        assert_eq!(directory("/about/"), "about/index.html");
        assert_eq!(file("/about/"), "about.html");
    }

    #[test]
    fn assets_keep_their_url() {
        assert_eq!(
            file_for_asset("/_topcoat/assets/logo-1a2b.png")
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
            "_topcoat/assets/logo-1a2b.png"
        );
    }

    #[test]
    fn traversal_segments_are_rejected() {
        for url in ["/../secret", "/blog/../../etc/passwd", "/a/./b"] {
            assert!(
                OutputFormat::Directory.file_for(url).is_err(),
                "accepted `{url}`"
            );
        }
    }

    #[test]
    fn an_asset_url_must_name_a_file() {
        assert_eq!(
            file_for_asset("/"),
            Err(OutputPathError::Empty {
                url: "/".to_owned()
            })
        );
    }

    #[test]
    fn the_staging_and_backup_directories_sit_beside_the_output() {
        let out = Path::new("/tmp/site/dist");
        assert_eq!(
            staging_dir(out),
            Path::new("/tmp/site/.dist.topcoat-export")
        );
        assert_eq!(
            backup_dir(out),
            Path::new("/tmp/site/.dist.topcoat-export-prev")
        );
    }
}
