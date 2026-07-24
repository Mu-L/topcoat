//! End-to-end exports: a real application served over HTTP, exported with the
//! same code path `topcoat export` runs, checked on disk.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tokio::net::TcpListener;
use tokio::sync::oneshot;
use topcoat::{
    Result,
    context::Cx,
    router::{
        Router, RouterBuilder, RouterService, StaticParams, StaticRoutes, internal_serve, page,
        route,
    },
    view::view,
};
use topcoat_cli::export::{ExportError, OutputFormat, export_site};

// -- The application under export --

#[page("/")]
async fn home() -> Result {
    view! { <h1>"home"</h1> }
}

#[page("/about")]
async fn about(cx: &Cx) -> Result {
    view! {
        <h1>"about"</h1>
        // The dev tooling renders nothing into an exported page.
        topcoat::dev::script()
        <p>(topcoat::router::uri(cx).path().to_owned())</p>
    }
}

#[allow(
    clippy::unused_async,
    reason = "a generator is async so it can query for its parameters"
)]
async fn posts(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(vec![
        StaticParams::from([("year", "2026"), ("slug", "hello")]),
        StaticParams::from([("year", "2026"), ("slug", "second post")]),
    ])
}

#[page("/blog/{year}/{slug}", generate_static = posts)]
async fn post(cx: &Cx) -> Result {
    let path = topcoat::router::uri(cx).path().to_owned();
    view! {
        <h1>"post"</h1>
        <p>(path)</p>
    }
}

// A dynamic page that never opted in, and a page that does not answer `GET`:
// neither belongs in a static site.
#[page("/users/{id}")]
async fn user() -> Result {
    view! { <h1>"user"</h1> }
}

#[page(POST "/subscribe")]
async fn subscribe() -> Result {
    view! { <p>"subscribed"</p> }
}

/// The bytes the application serves as its one asset.
const ASSET_BODY: &[u8] = b"body { color: rebeccapurple }\n";
const ASSET_URL: &str = "/_topcoat/assets/app-1a2b3c4d.css";

// An asset is a plain route, not a page, exactly as the asset bundle
// registers one. Serving it here keeps the test from needing bundled files on
// disk; what matters to the export is that the URL answers with these bytes.
#[route(GET "/_topcoat/assets/app-1a2b3c4d.css")]
async fn stylesheet() -> Result<&'static str> {
    Ok("body { color: rebeccapurple }\n")
}

/// Builds the application's router.
///
/// The static route listing is registered explicitly: the router adds it
/// itself only when the application runs under the `topcoat` CLI, and a test
/// cannot set the environment variable that says so.
fn app() -> Router {
    let builder = RouterBuilder::new()
        .page(home)
        .page(about)
        .page(post)
        .page(user)
        .page(subscribe)
        .route(stylesheet)
        .static_files([ASSET_URL.to_owned()]);
    let listing = StaticRoutes::new(builder.static_pages(), vec![ASSET_URL.to_owned()]);
    builder.route(listing).build()
}

/// A router whose two pages generate the same URL, which no export can
/// resolve.
fn conflicting_app() -> Router {
    let builder = RouterBuilder::new().page(home).page(post);
    // `/` is claimed twice: once by the page and once by a generator that
    // names no parameters at all.
    let listing = StaticRoutes::new(
        vec![
            topcoat::router::StaticPage::new(
                std::borrow::Cow::Borrowed(topcoat::router::Path::new("/")),
                true,
                None,
            ),
            topcoat::router::StaticPage::new(
                std::borrow::Cow::Borrowed(topcoat::router::Path::new("/(group)")),
                true,
                None,
            ),
        ],
        Vec::new(),
    );
    builder.route(listing).build()
}

// -- Serving the application for the duration of one test --

/// A running application, shut down when dropped.
struct Server {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Server {
    /// Serves `router` on an ephemeral local port.
    fn start(router: Router) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (ready, ready_rx) = std::sync::mpsc::channel();

        runtime.spawn(async move {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            ready.send(listener.local_addr().unwrap()).unwrap();
            let _ = internal_serve(listener, RouterService::new(router), async {
                let _ = shutdown_rx.await;
            })
            .await;
        });

        Self {
            addr: ready_rx.recv().unwrap(),
            shutdown: Some(shutdown),
            runtime: Some(runtime),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// A temporary directory removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "topcoat-export-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read(dir: &Path, file: &str) -> String {
    fs::read_to_string(dir.join(file))
        .unwrap_or_else(|error| panic!("{}: {error}", dir.join(file).display()))
}

// -- Tests --

#[test]
fn exports_pages_and_assets_with_clean_directory_urls() {
    let server = Server::start(app());
    let temp = TempDir::new("directory");
    let out = temp.path().join("dist");

    let summary = export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    // Two fixed pages plus the two generated ones.
    assert_eq!(summary.pages, 4);
    assert_eq!(summary.assets, 1);

    assert!(read(&out, "index.html").contains("home"));
    assert!(read(&out, "about/index.html").contains("about"));
    assert!(read(&out, "blog/2026/hello/index.html").contains("/blog/2026/hello"));
    // A parameter value with a space is requested percent-encoded, as the URL
    // the page rendered shows, and written under its decoded name.
    assert!(read(&out, "blog/2026/second post/index.html").contains("/blog/2026/second%20post"));
}

#[test]
fn exports_pages_as_html_files() {
    let server = Server::start(app());
    let temp = TempDir::new("file");
    let out = temp.path().join("dist");

    export_site(&server.base_url(), &out, OutputFormat::File).unwrap();

    // The root keeps its index name; everything else takes a `.html` file.
    assert!(read(&out, "index.html").contains("home"));
    assert!(read(&out, "about.html").contains("about"));
    assert!(read(&out, "blog/2026/hello.html").contains("/blog/2026/hello"));
    assert!(!out.join("about/index.html").exists());
}

#[test]
fn assets_keep_their_url_and_their_bytes() {
    let server = Server::start(app());
    let temp = TempDir::new("assets");
    let out = temp.path().join("dist");

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    let asset = out.join("_topcoat/assets/app-1a2b3c4d.css");
    assert_eq!(fs::read(&asset).unwrap(), ASSET_BODY);
}

#[test]
fn pages_that_cannot_be_exported_are_left_out() {
    let server = Server::start(app());
    let temp = TempDir::new("skipped");
    let out = temp.path().join("dist");

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    assert!(
        !out.join("users").exists(),
        "a dynamic page with no generator was exported"
    );
    assert!(
        !out.join("subscribe").exists(),
        "a page that does not answer GET was exported"
    );
}

#[test]
fn the_development_tooling_is_not_rendered_into_an_exported_page() {
    let server = Server::start(app());
    let temp = TempDir::new("no-dev-script");
    let out = temp.path().join("dist");

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    assert!(!read(&out, "about/index.html").contains("<script"));
}

#[test]
fn an_export_creates_the_output_directory() {
    let server = Server::start(app());
    let temp = TempDir::new("nested");
    let out = temp.path().join("nested/site");

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    assert!(out.join("index.html").is_file());
}

#[test]
fn a_second_export_replaces_the_first() {
    let server = Server::start(app());
    let temp = TempDir::new("replace");
    let out = temp.path().join("dist");

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();
    fs::write(out.join("stale.html"), "stale").unwrap();

    export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();

    assert!(out.join("index.html").is_file());
    assert!(
        !out.join("stale.html").exists(),
        "the previous export survived"
    );
    // No staging or backup directory is left behind.
    let leftovers: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, vec![std::ffi::OsString::from("dist")]);
}

#[test]
fn a_failing_export_leaves_the_previous_one_in_place() {
    let temp = TempDir::new("keep-previous");
    let out = temp.path().join("dist");

    // A first, successful export.
    {
        let server = Server::start(app());
        export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap();
    }
    let before = read(&out, "index.html");

    // The application is gone, so the export cannot even read the listing.
    let error = export_site("http://127.0.0.1:1", &out, OutputFormat::Directory).unwrap_err();
    assert!(matches!(error, ExportError::Request { .. }), "{error}");

    assert_eq!(read(&out, "index.html"), before);
}

#[test]
fn a_rejected_listing_reports_the_applications_own_message() {
    let server = Server::start(conflicting_app());
    let temp = TempDir::new("conflict");
    let out = temp.path().join("dist");

    let error = export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("both generate the static path `/`"),
        "{message}"
    );
    // Nothing was written, not even an empty output directory.
    assert!(!out.exists());
}

#[test]
fn an_application_without_the_listing_is_reported_clearly() {
    // A router built the way a deployed application is: no static route
    // listing, since it is not running under the topcoat tooling.
    let server = Server::start(RouterBuilder::new().page(home).build());
    let temp = TempDir::new("no-listing");
    let out = temp.path().join("dist");

    let error = export_site(&server.base_url(), &out, OutputFormat::Directory).unwrap_err();
    assert!(matches!(error, ExportError::ListingUnavailable), "{error}");
    assert!(
        error.to_string().contains("/_topcoat/routes/static"),
        "{error}"
    );
}

/// The `topcoat export` subcommand itself, as the shipped binary exposes it.
///
/// The command's own steps -- compiling the application, starting it, and
/// asking it what to export -- are the same ones `examples/static-export`
/// exercises when run by hand; what this pins down is that the subcommand is
/// wired into the CLI with the documented options.
#[test]
fn the_cli_exposes_the_export_subcommand() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_topcoat"))
        .args(["export", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{:?}", output.status);
    let help = String::from_utf8(output.stdout).unwrap();
    for flag in ["--out", "--format", "directory", "file", "--release"] {
        assert!(help.contains(flag), "`{flag}` missing from:\n{help}");
    }
}
