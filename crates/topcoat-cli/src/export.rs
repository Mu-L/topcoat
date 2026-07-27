#![doc = include_str!("../docs/export.md")]

// An export is a short-lived version of what `topcoat dev` does. The
// application is compiled and its assets bundled, then started as a child
// process pointed at a local server through `TOPCOAT_DEV_URL`, exactly as the
// dev server starts it. That enables the development-only route listing at
// `/_topcoat/routes/static`, which reports every page the application can
// render statically (running each page's `generate_static` function) along
// with every asset it serves.
//
// The export then fetches each of those URLs over HTTP and writes the
// responses to disk. The application decides what its own static site
// contains; the CLI only asks, fetches, and writes.

mod app;
mod output;
mod site;

pub use output::*;
pub use site::*;

use std::path::PathBuf;

use clap::Args;
use console::style;

use app::{AppProcess, ReadyServer};

use crate::common::cargo::{BuildFlags, BuildOpts};
use crate::dev::Spinner;

#[derive(Args)]
pub struct ExportCommand {
    #[command(flatten)]
    build: BuildFlags,
    /// Output directory for the exported site
    #[arg(short, long, default_value = "dist", value_name = "DIR")]
    out: PathBuf,
    /// How page URLs map to files: clean directory URLs, or `.html` files
    #[arg(long, value_enum, default_value_t = OutputFormat::Directory)]
    format: OutputFormat,
}

impl ExportCommand {
    /// Build the application, export its site, and report what was written.
    ///
    /// Any failure is reported to the terminal and stops the process without
    /// touching a previous export.
    ///
    /// # Panics
    ///
    /// Panics if the blocking export task panics.
    pub async fn run(self) {
        let opts: BuildOpts = self.build.into();

        eprintln!();
        eprintln!(
            "  {} {}",
            style("topcoat").cyan().bold(),
            style("export").dim()
        );

        let exe = build(&opts).await;

        // The application is driven exactly as the dev server drives it, so
        // the development-only route listing is available to read.
        let mut ready = ReadyServer::start()
            .await
            .unwrap_or_else(|error| fail(&format!("failed to start the export server: {error}")));
        let mut app = AppProcess::spawn(&exe, ready.url())
            .unwrap_or_else(|error| fail(&format!("failed to start the application: {error}")));

        let spinner = Spinner::new("starting the application");
        let addr = ready.wait(&mut app).await;
        drop(spinner);
        let addr = match addr {
            Ok(addr) => addr,
            Err(error) => {
                // An application that never reported in may still be running,
                // and stopping the export runs no destructors, so the child is
                // shut down before then.
                app.shutdown().await;
                fail(&error.to_string());
            }
        };

        let spinner = Spinner::new("exporting");
        let base_url = format!("http://{addr}");
        let out = self.out.clone();
        let format = self.format;
        // The export blocks on HTTP and filesystem I/O.
        let exported = tokio::task::spawn_blocking(move || export_site(&base_url, &out, format))
            .await
            .expect("export task panicked");
        drop(spinner);

        app.shutdown().await;

        let summary = exported.unwrap_or_else(|error| fail(&error.to_string()));
        eprintln!(
            "  {} {} {} and {} {} into {}",
            style("exported").green().bold(),
            summary.pages,
            plural(summary.pages, "page"),
            summary.assets,
            plural(summary.assets, "asset"),
            style(self.out.display()).cyan()
        );
        eprintln!();
    }
}

/// Compile the application and bundle its assets, as the dev server does
/// before starting it.
async fn build(opts: &BuildOpts) -> PathBuf {
    let spinner = Spinner::new("building");
    let progress = spinner.bar();
    let built = opts
        .build_and_read(move |current, total| {
            progress.set_message(format!("building ({current}/{total})"));
        })
        .await;
    drop(spinner);

    let (exe, bytes) = built.unwrap_or_else(|error| error.print_and_exit());

    let spinner = Spinner::new("bundling assets");
    let bundled = crate::asset::run_bundle(&bytes, None).await;
    drop(spinner);

    if let Err(error) = bundled {
        fail(&format!("failed to bundle assets: {error}"));
    }
    exe
}

/// Report a failure and stop, leaving any previous export untouched.
fn fail(message: &str) -> ! {
    eprintln!("  {}", style("export failed").red().bold());
    eprintln!();
    eprintln!("  {}", style(message).red());
    eprintln!();
    std::process::exit(1);
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_owned()
    } else {
        format!("{noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nouns_agree_with_their_count() {
        assert_eq!(plural(1, "page"), "page");
        assert_eq!(plural(0, "page"), "pages");
        assert_eq!(plural(2, "asset"), "assets");
    }
}
