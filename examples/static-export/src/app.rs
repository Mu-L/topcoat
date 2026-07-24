mod blog;
mod tags;

use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt, asset},
    router::{Router, layout, page},
    view::view,
};

/// The route tree is rooted here, so `module_router!` derives every
/// module-derived page's URL from this module.
///
/// The assets registered here are copied into an export at the same URLs the
/// running application serves them from.
pub fn router() -> Router {
    topcoat::router::module_router!()
        // `module_router!` derives the URLs of pages declared without one;
        // `discover_pages` adds the pages that declare an explicit path.
        .discover_pages()
        .assets(AssetBundle::load().unwrap())
        .build()
}

#[layout]
async fn root_layout(slot: Result) -> Result {
    view! {
        <!DOCTYPE html>
        <html>
            <head>
                <link rel="stylesheet" href=(asset!("./site.css"))>
                // Renders nothing in an exported page.
                topcoat::dev::script()
            </head>
            <body>
                <nav>
                    <a href="/">"home"</a>
                    <a href="/about">"about"</a>
                    <a href="/blog">"blog"</a>
                </nav>
                (slot?)
            </body>
        </html>
    }
}

// A page with a fixed path. Nothing to declare: an export renders it as it
// stands.
#[page]
async fn home() -> Result {
    view! {
        <h1>"A statically exported site"</h1>
        <p>"Run `topcoat export` to write this site into `dist/`."</p>
    }
}

mod about {
    use topcoat::{Result, router::page, view::view};

    #[page]
    async fn about() -> Result {
        view! {
            <h1>"About"</h1>
            <p>"src/app/about.rs -> /about"</p>
        }
    }
}
