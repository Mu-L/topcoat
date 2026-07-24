//! A site built to be exported with `topcoat export`.
//!
//! Run `topcoat export -p static-export` from the workspace root to write the
//! whole site into `dist/`, or `topcoat dev -p static-export` to serve it.

mod app;
mod posts;

#[tokio::main]
async fn main() {
    topcoat::start(app::router()).await.unwrap();
}
