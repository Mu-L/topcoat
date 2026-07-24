use std::error::Error;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Args;
use console::style;
use tokio::process::Command;

use crate::cargo::{BuildFlags, BuildOpts};

const INTERNAL_COMMAND_ENV: &str = "TOPCOAT_INTERNAL_COMMAND";
const EXPORT_PROTOCOL_ENV: &str = "TOPCOAT_EXPORT_PROTOCOL";
const EXPORT_OUT_ENV: &str = "TOPCOAT_EXPORT_OUT";
const EXPORT_PATH_STYLE_ENV: &str = "TOPCOAT_EXPORT_PATH_STYLE";

#[derive(Args)]
pub struct ExportCommand {
    #[command(flatten)]
    build: BuildFlags,
    /// Output directory
    #[arg(short, long, default_value = "dist")]
    out: PathBuf,
    /// Write `/about` to `about.html` instead of `about/index.html`
    #[arg(long)]
    html_files: bool,
}

impl ExportCommand {
    pub async fn run(self) {
        if let Err(error) = self.try_run().await {
            eprintln!(
                "{}",
                style(format!("static export failed: {error}")).red().bold()
            );
            std::process::exit(1);
        }
    }

    async fn try_run(self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let opts: BuildOpts = self.build.into();
        let out_dir = safe_output_path(&self.out)?;

        eprintln!("  {}", style("building application...").dim());
        let (executable, bytes) = crate::cargo::build_and_read(&opts, |_, _| {}).await?;

        eprintln!("  {}", style("bundling assets...").dim());
        crate::asset::run_bundle(&bytes, None).await?;

        let staging = staging_path(&out_dir)?;
        let path_style = if self.html_files {
            "html-file"
        } else {
            "directory"
        };

        eprintln!("  {}", style("rendering static pages...").dim());
        let status = Command::new(&executable)
            .env(INTERNAL_COMMAND_ENV, "export")
            .env(EXPORT_PROTOCOL_ENV, "1")
            .env(EXPORT_OUT_ENV, &staging)
            .env(EXPORT_PATH_STYLE_ENV, path_style)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await?;

        if !status.success() {
            remove_path_if_present(&staging)?;
            return Err(format!("application exited with {status}").into());
        }

        publish_staging(&staging, &out_dir)?;
        println!("exported site to {}", out_dir.display());
        Ok(())
    }
}

fn safe_output_path(path: &Path) -> Result<PathBuf, io::Error> {
    let path = std::path::absolute(path)?;
    let current_dir = std::env::current_dir()?.canonicalize()?;
    let comparable = if path.exists() {
        path.canonicalize()?
    } else {
        path.clone()
    };
    if comparable == current_dir || current_dir.starts_with(&comparable) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to replace `{}` because it contains the current workspace",
                path.display()
            ),
        ));
    }
    if path.parent().is_none() || path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{}` is not a directory path", path.display()),
        ));
    }
    Ok(path)
}

fn staging_path(out_dir: &Path) -> Result<PathBuf, io::Error> {
    let parent = out_dir.parent().expect("output path has a parent");
    std::fs::create_dir_all(parent)?;
    let file_name = out_dir
        .file_name()
        .expect("output path has a file name")
        .to_string_lossy();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = parent.join(format!(
        ".{file_name}.topcoat-export-{}-{nonce}",
        std::process::id()
    ));
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("staging path `{}` already exists", path.display()),
        ));
    }
    Ok(path)
}

fn publish_staging(staging: &Path, out_dir: &Path) -> Result<(), io::Error> {
    if !out_dir.exists() {
        return std::fs::rename(staging, out_dir);
    }

    let mut backup_name = OsString::from(".");
    backup_name.push(out_dir.file_name().expect("output path has a file name"));
    backup_name.push(format!(".topcoat-backup-{}", std::process::id()));
    let backup = out_dir
        .parent()
        .expect("output path has a parent")
        .join(backup_name);
    if backup.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("backup path `{}` already exists", backup.display()),
        ));
    }

    std::fs::rename(out_dir, &backup)?;
    if let Err(error) = std::fs::rename(staging, out_dir) {
        let _ = std::fs::rename(&backup, out_dir);
        return Err(error);
    }
    remove_path_if_present(&backup)
}

fn remove_path_if_present(path: &Path) -> Result<(), io::Error> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
