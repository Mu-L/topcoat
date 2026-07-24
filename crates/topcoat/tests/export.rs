use std::path::{Path, PathBuf};

use topcoat::{
    ExportConfig, ExportPathStyle, Result,
    context::Cx,
    export,
    router::{Router, is_static_export, page},
    view::view,
};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("topcoat-export-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[page("/")]
async fn home(cx: &Cx) -> Result {
    view! {
        <!DOCTYPE html>
        <html><body>"export: " (is_static_export(cx))</body></html>
    }
}

#[page("/about")]
async fn about() -> Result {
    view! {
        <!DOCTYPE html>
        <html><body>"about"</body></html>
    }
}

#[page("/404")]
async fn not_found_page() -> Result {
    view! {
        <!DOCTYPE html>
        <html><body>"custom not found"</body></html>
    }
}

#[tokio::test]
async fn exports_pages_and_registered_static_files() {
    let root = TestDir::new();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "static contents").unwrap();
    let out = root.path().join("dist");
    let router = Router::builder()
        .page(home)
        .page(about)
        .page(not_found_page)
        .static_file("/assets/source.txt", &source)
        .build();

    let report = export(router, ExportConfig::new(&out)).await.unwrap();

    assert_eq!(report.pages(), 3);
    assert_eq!(report.files(), 1);
    assert!(
        std::fs::read_to_string(out.join("index.html"))
            .unwrap()
            .contains("export: true")
    );
    assert!(
        std::fs::read_to_string(out.join("about/index.html"))
            .unwrap()
            .contains("about")
    );
    assert!(
        std::fs::read_to_string(out.join("404.html"))
            .unwrap()
            .contains("custom not found")
    );
    assert_eq!(
        std::fs::read_to_string(out.join("assets/source.txt")).unwrap(),
        "static contents"
    );
}

#[tokio::test]
async fn html_file_style_writes_flat_page_names_and_a_default_404() {
    let root = TestDir::new();
    let out = root.path().join("dist");
    let router = Router::builder().page(home).page(about).build();

    let report = export(
        router,
        ExportConfig::new(&out).path_style(ExportPathStyle::HtmlFile),
    )
    .await
    .unwrap();

    assert_eq!(report.pages(), 3);
    assert!(out.join("index.html").is_file());
    assert!(out.join("about.html").is_file());
    assert!(out.join("404.html").is_file());
}

#[tokio::test]
async fn reports_page_and_static_file_output_collisions_before_writing() {
    let root = TestDir::new();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "static contents").unwrap();
    let out = root.path().join("dist");
    let router = Router::builder()
        .page(about)
        .static_file("/about", &source)
        .build();

    let error = export(router, ExportConfig::new(&out)).await.unwrap_err();

    assert!(error.to_string().contains("static outputs"));
    assert!(!out.exists());
}
