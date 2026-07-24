use topcoat::{
    Result,
    context::Cx,
    router::{StaticParams, page, path_param},
    view::view,
};

use crate::posts::TAGS;

/// The `{tag}` placeholder written into the page's explicit path below.
#[path_param]
struct Tag(str);

/// The tags the page is exported for.
#[allow(
    clippy::unused_async,
    reason = "the generator is async so it can query a real data source"
)]
async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(TAGS
        .iter()
        .map(|name| StaticParams::from([("tag", *name)]))
        .collect())
}

// An explicit path opts in exactly the same way, and lands in the same export
// as the module-derived pages: the export covers both forms.
#[page("/tags/{tag}", generate_static = generate_static_params)]
async fn tag(cx: &Cx) -> Result {
    let tag = path_param::<Tag>(cx);
    view! {
        <h1>
            "Posts tagged "
            (tag)
        </h1>
        <p><a href="/blog">"back to the blog"</a></p>
    }
}
