use topcoat::{
    Result,
    context::Cx,
    router::{StaticParams, error::RouterErrorExt, page, path_param},
    view::view,
};

use super::Year;
use crate::posts::{self, POSTS};

/// Turns this module's segment into `{slug}`: the page serves
/// `/blog/{year}/{slug}`.
#[path_param]
struct Slug(str);

/// Every post the page is exported for.
///
/// A parameter set names *every* dynamic segment in the page's path, not only
/// the one this module contributes. `{year}` comes from the parent module, and
/// a set that leaves it out is rejected with an error naming the page.
///
/// The generator runs inside a request, so it can read the app context and
/// anything else a page handler can.
#[allow(
    clippy::unused_async,
    reason = "the generator is async so it can query a real data source"
)]
async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(POSTS
        .iter()
        .map(|entry| StaticParams::from([("year", entry.year), ("slug", entry.slug)]))
        .collect())
}

// A dynamic path, so the page opts into the export by naming its generator.
#[page(generate_static = generate_static_params)]
async fn post(cx: &Cx) -> Result {
    let year = path_param::<Year>(cx);
    let slug = path_param::<Slug>(cx);
    let post = posts::find(year, slug).ok_or_not_found()?;

    view! {
        <h1>(post.title)</h1>
        <p>(post.body)</p>
        <p><a href="/blog">"back to the blog"</a></p>
    }
}
