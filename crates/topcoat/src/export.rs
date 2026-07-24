#![doc = include_str!("../docs/export.md")]

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use topcoat_router::{
    Body, Method, Request, RouterService, StaticExportMarker, StatusCode, header, to_bytes,
};

const INTERNAL_COMMAND_ENV: &str = "TOPCOAT_INTERNAL_COMMAND";
const EXPORT_COMMAND: &str = "export";
const EXPORT_PROTOCOL_ENV: &str = "TOPCOAT_EXPORT_PROTOCOL";
const EXPORT_PROTOCOL: &str = "1";
const EXPORT_OUT_ENV: &str = "TOPCOAT_EXPORT_OUT";
const EXPORT_PATH_STYLE_ENV: &str = "TOPCOAT_EXPORT_PATH_STYLE";
const DEFAULT_404: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Not Found</title></head><body><h1>404 Not Found</h1></body></html>";

/// How page URLs map to files in a static export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExportPathStyle {
    /// Write `/about` to `about/index.html`.
    #[default]
    Directory,
    /// Write `/about` to `about.html`.
    HtmlFile,
}

impl ExportPathStyle {
    fn from_env(value: &str) -> Result<Self, ExportError> {
        match value {
            "directory" => Ok(Self::Directory),
            "html-file" => Ok(Self::HtmlFile),
            _ => Err(ExportError::new(format!(
                "{EXPORT_PATH_STYLE_ENV} must be `directory` or `html-file`, got `{value}`"
            ))),
        }
    }
}

/// Configuration for rendering a router into static files.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    out_dir: PathBuf,
    path_style: ExportPathStyle,
}

impl ExportConfig {
    /// Creates an export writing into `out_dir`.
    #[must_use]
    pub fn new(out_dir: impl Into<PathBuf>) -> Self {
        Self {
            out_dir: out_dir.into(),
            path_style: ExportPathStyle::default(),
        }
    }

    /// Selects how non-root page URLs map to HTML files.
    #[must_use]
    pub fn path_style(mut self, path_style: ExportPathStyle) -> Self {
        self.path_style = path_style;
        self
    }

    /// Returns the destination directory.
    #[must_use]
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }
}

/// Summary of a completed static export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    out_dir: PathBuf,
    pages: usize,
    files: usize,
}

impl ExportReport {
    /// Returns the destination directory.
    #[must_use]
    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    /// Returns the number of rendered pages, including the generated 404 page.
    #[must_use]
    pub fn pages(&self) -> usize {
        self.pages
    }

    /// Returns the number of copied static files.
    #[must_use]
    pub fn files(&self) -> usize {
        self.files
    }
}

/// An error encountered while producing a static export.
#[derive(Debug)]
pub struct ExportError {
    message: String,
}

impl ExportError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExportError {}

/// Renders all statically selectable pages and copies registered static files.
///
/// Fixed GET pages are exported automatically. Dynamic module routes must
/// declare `generate_static` on each dynamic segment, and dynamic explicit
/// page paths must declare `generate_static` on `#[page]`.
///
/// # Errors
///
/// Returns an error when path generation or rendering fails, a response is not
/// exportable, an output path collides, or a filesystem operation fails.
pub async fn export(
    service: impl Into<RouterService>,
    config: ExportConfig,
) -> Result<ExportReport, ExportError> {
    let service = service.into();
    let router = service.router();
    let paths = router
        .generate_static_paths()
        .await
        .map_err(|error| ExportError::new(error.to_string()))?;

    let mut outputs = HashMap::<PathBuf, String>::new();
    let mut rendered_404 = false;
    let mut page_outputs = Vec::new();
    for path in &paths {
        let relative = page_output_path(path.url_path(), config.path_style)?;
        reserve_output(&mut outputs, &relative, path.url_path())?;
        page_outputs.push((path, relative));
        rendered_404 |= path.url_path() == "/404";
    }

    if !rendered_404 {
        let relative = PathBuf::from("404.html");
        reserve_output(&mut outputs, &relative, "generated 404 page")?;
    }

    let mut static_outputs = Vec::new();
    for file in router.static_files() {
        let relative = static_file_output_path(file.url_path())?;
        reserve_output(&mut outputs, &relative, file.url_path())?;
        static_outputs.push((file, relative));
    }

    tokio::fs::create_dir_all(&config.out_dir)
        .await
        .map_err(|error| {
            ExportError::new(format!(
                "failed to create export directory `{}`: {error}",
                config.out_dir.display()
            ))
        })?;

    for (path, relative) in page_outputs {
        let bytes = render_page(router, path.url_path()).await?;
        write_output(&config.out_dir, &relative, &bytes).await?;
    }
    if !rendered_404 {
        write_output(
            &config.out_dir,
            Path::new("404.html"),
            DEFAULT_404.as_bytes(),
        )
        .await?;
    }

    for (file, relative) in static_outputs {
        let destination = config.out_dir.join(&relative);
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                ExportError::new(format!(
                    "failed to create static file directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
        tokio::fs::copy(file.source_path(), &destination)
            .await
            .map_err(|error| {
                ExportError::new(format!(
                    "failed to copy static file `{}` to `{}`: {error}",
                    file.source_path().display(),
                    destination.display()
                ))
            })?;
    }

    Ok(ExportReport {
        out_dir: config.out_dir,
        pages: paths.len() + usize::from(!rendered_404),
        files: router.static_files().len(),
    })
}

pub(crate) fn config_from_env() -> Result<Option<ExportConfig>, ExportError> {
    match std::env::var(INTERNAL_COMMAND_ENV) {
        Ok(command) if command == EXPORT_COMMAND => {}
        Ok(_) | Err(std::env::VarError::NotPresent) => return Ok(None),
        Err(error) => {
            return Err(ExportError::new(format!(
                "{INTERNAL_COMMAND_ENV} must be valid Unicode: {error}"
            )));
        }
    }

    match std::env::var(EXPORT_PROTOCOL_ENV) {
        Ok(protocol) if protocol == EXPORT_PROTOCOL => {}
        Ok(protocol) => {
            return Err(ExportError::new(format!(
                "unsupported static export protocol `{protocol}`"
            )));
        }
        Err(std::env::VarError::NotPresent) => {
            return Err(ExportError::new(format!(
                "{EXPORT_PROTOCOL_ENV} is required for export"
            )));
        }
        Err(error) => {
            return Err(ExportError::new(format!(
                "{EXPORT_PROTOCOL_ENV} must be valid Unicode: {error}"
            )));
        }
    }

    let out_dir = std::env::var_os(EXPORT_OUT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| ExportError::new(format!("{EXPORT_OUT_ENV} is required for export")))?;
    let path_style = match std::env::var(EXPORT_PATH_STYLE_ENV) {
        Ok(value) => ExportPathStyle::from_env(&value)?,
        Err(std::env::VarError::NotPresent) => ExportPathStyle::default(),
        Err(error) => {
            return Err(ExportError::new(format!(
                "{EXPORT_PATH_STYLE_ENV} must be valid Unicode: {error}"
            )));
        }
    };
    Ok(Some(ExportConfig::new(out_dir).path_style(path_style)))
}

async fn render_page(
    router: &topcoat_router::Router,
    url_path: &str,
) -> Result<Vec<u8>, ExportError> {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(url_path)
        .body(Body::empty())
        .map_err(|error| {
            ExportError::new(format!(
                "failed to build static request for `{url_path}`: {error}"
            ))
        })?;
    request.extensions_mut().insert(StaticExportMarker);
    let response = router.handle(request).await;

    if response.status() != StatusCode::OK {
        return Err(ExportError::new(format!(
            "static request for `{url_path}` returned {}",
            response.status()
        )));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
    }) {
        return Err(ExportError::new(format!(
            "static request for `{url_path}` must return `text/html`, got `{}`",
            content_type.unwrap_or("<missing>")
        )));
    }
    if response.headers().contains_key(header::SET_COOKIE) {
        return Err(ExportError::new(format!(
            "static request for `{url_path}` returned `Set-Cookie`, which cannot be exported"
        )));
    }
    if response.headers().contains_key(header::CONTENT_ENCODING) {
        return Err(ExportError::new(format!(
            "static request for `{url_path}` returned encoded content, which cannot be exported"
        )));
    }

    to_bytes(response.into_body(), usize::MAX)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| {
            ExportError::new(format!(
                "failed to read static response for `{url_path}`: {error}"
            ))
        })
}

fn page_output_path(url_path: &str, path_style: ExportPathStyle) -> Result<PathBuf, ExportError> {
    if url_path == "/" {
        return Ok(PathBuf::from("index.html"));
    }
    if url_path == "/404" {
        return Ok(PathBuf::from("404.html"));
    }

    let mut path = url_to_relative_path(url_path)?;
    match path_style {
        ExportPathStyle::Directory => path.push("index.html"),
        ExportPathStyle::HtmlFile => {
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| ExportError::new(format!("invalid static URL `{url_path}`")))?;
            path.set_file_name(format!("{file_name}.html"));
        }
    }
    Ok(path)
}

fn static_file_output_path(url_path: &str) -> Result<PathBuf, ExportError> {
    let path = url_to_relative_path(url_path)?;
    if path.as_os_str().is_empty() {
        return Err(ExportError::new(
            "a static file cannot be registered at `/`",
        ));
    }
    Ok(path)
}

fn url_to_relative_path(url_path: &str) -> Result<PathBuf, ExportError> {
    let Some(relative) = url_path.strip_prefix('/') else {
        return Err(ExportError::new(format!(
            "static URL `{url_path}` must start with `/`"
        )));
    };
    if relative.ends_with('/') || relative.contains('?') || relative.contains('#') {
        return Err(ExportError::new(format!(
            "static URL `{url_path}` is not a canonical path"
        )));
    }

    let path = PathBuf::from(relative);
    if path.components().any(|component| {
        !matches!(component, Component::Normal(_))
            || component.as_os_str() == "."
            || component.as_os_str() == ".."
    }) {
        return Err(ExportError::new(format!(
            "static URL `{url_path}` would escape the export directory"
        )));
    }
    Ok(path)
}

fn reserve_output(
    outputs: &mut HashMap<PathBuf, String>,
    relative: &Path,
    source: &str,
) -> Result<(), ExportError> {
    for (existing_path, existing_source) in &*outputs {
        if relative == existing_path
            || relative.starts_with(existing_path)
            || existing_path.starts_with(relative)
        {
            return Err(ExportError::new(format!(
                "static outputs `{}` and `{}` conflict; they are generated by `{existing_source}` and `{source}`",
                existing_path.display(),
                relative.display()
            )));
        }
    }
    outputs.insert(relative.to_owned(), source.to_owned());
    Ok(())
}

async fn write_output(out_dir: &Path, relative: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let destination = out_dir.join(relative);
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            ExportError::new(format!(
                "failed to create page directory `{}`: {error}",
                parent.display()
            ))
        })?;
    }
    tokio::fs::write(&destination, bytes)
        .await
        .map_err(|error| {
            ExportError::new(format!(
                "failed to write static page `{}`: {error}",
                destination.display()
            ))
        })
}
