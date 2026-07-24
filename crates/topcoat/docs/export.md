Static export renders a Topcoat application's GET pages to HTML files that can
be hosted without a running Rust server. It uses the same router, layouts,
layers, app context, and asset declarations as the served application.

# Exporting a site

Run the command from the application workspace:

```text
topcoat export
```

The default output is `dist/`. The command builds the selected application,
bundles its assets, starts the built binary in export mode, and only replaces
the output directory after every page succeeds.

The build selection flags match the other commands that compile an app:

```text
topcoat export --release
topcoat export --package my-site
topcoat export --bin my-site
topcoat export --profile production
topcoat export --out public
```

By default, `/about` is written to `about/index.html`. Pass `--html-files` to
write it to `about.html` instead. `/` is always `index.html`, and `/404` is
always `404.html`. If the router has no `/404` page, Topcoat writes a minimal
fallback `404.html`.

# Fixed pages

Every fixed page that accepts GET is included automatically:

```rust
# use topcoat::{Result, router::page, view::view};
#[page("/about")]
async fn about() -> Result {
    view! {
        <!DOCTYPE html>
        <html><body><h1>"About"</h1></body></html>
    }
}
```

Pages that do not accept GET are not exported. API routes declared with
`#[route]` are also not exported.

# Dynamic module routes

A dynamic URL has no finite set of paths until the application supplies one.
For a module route, add `generate_static` to every dynamic segment.

A typed [`#[path_param]`](macro@crate::router::path_param) generator returns
values of the struct's inner type:

```rust
# use topcoat::{Result, context::Cx, router::path_param};
async fn generate_post_ids(_cx: &Cx) -> Result<Vec<u64>> {
    Ok(vec![1, 2, 3])
}

#[path_param(generate_static = generate_post_ids)]
struct PostId(u64);
```

If this declaration is in the module that contributes `{post_id}`, descendant
pages are generated for `1`, `2`, and `3`. A `str` path parameter generator
returns `Vec<String>`.

A manual parameter segment uses `Vec<String>`:

```rust
# use topcoat::{Result, context::Cx};
async fn generate_slugs(_cx: &Cx) -> Result<Vec<String>> {
    Ok(vec!["first-post".into(), "second-post".into()])
}

topcoat::router::segment!(
    kind = Param,
    rename = "slug",
    generate_static = generate_slugs,
);
```

A catch-all generator returns one non-empty `Vec<String>` per URL. Each inner
string is one path component:

```rust
# use topcoat::{Result, context::Cx};
async fn generate_docs(_cx: &Cx) -> Result<Vec<Vec<String>>> {
    Ok(vec![
        vec!["guide".into(), "start".into()],
        vec!["reference".into()],
    ])
}

topcoat::router::segment!(
    kind = CatchAll,
    rename = "path",
    generate_static = generate_docs,
);
```

This generates `/guide/start` and `/reference` for that catch-all segment.

Generators run from the root toward the page. A child generator is called once
for each parameter set produced by its parents. Its `&Cx` contains app context,
the concrete ancestor path, and ancestor path parameters, so it can query data
for the current parent. Results shared by several descendant pages are reused
during the export.

# Dynamic explicit paths

Segment declarations only affect module-derived paths. An explicit dynamic
page instead declares one generator on `#[page]`. It returns complete
[`StaticParams`](crate::router::StaticParams) sets:

```rust
# use topcoat::{
#     Result,
#     context::Cx,
#     router::{StaticParams, page},
#     view::view,
# };
async fn generate_articles(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(vec![
        StaticParams::new()
            .param("year", 2025)
            .catch_all("slug", ["rust", "async"]),
        StaticParams::new()
            .param("year", 2026)
            .catch_all("slug", ["topcoat"]),
    ])
}

#[page(
    "/articles/{year}/{*slug}",
    generate_static = generate_articles
)]
async fn article() -> Result {
    view! { <h1>"Article"</h1> }
}
```

Each set must provide every dynamic parameter exactly once and must not contain
unknown names. `param` supplies one component; `catch_all` supplies one or more
components. Topcoat percent-encodes generated components when it constructs the
URL.

An export fails if a GET page has a dynamic module segment without
`generate_static`, or an explicit dynamic path has no page-level
`generate_static`. Topcoat does not guess values and does not leave a dynamic
server fallback in the output.

# Rendering behavior

Each selected URL is dispatched as a real GET request through the router. This
means normal page rendering, layouts, layers, request context, and app context
all apply. Use [`is_static_export`](crate::router::is_static_export) when code
must distinguish a build-time render:

```rust
# use topcoat::{Result, context::Cx, router::is_static_export, view::view};
# async fn example(cx: &Cx) -> Result {
if is_static_export(cx) {
    // Avoid request-specific work that has no meaning in a static file.
}
# view! {}
# }
```

An exported response must be `200 OK`, have a `text/html` content type, and
must not set cookies or use content encoding. A violation stops the export and
leaves the previous output directory intact.

Files registered by Topcoat's asset bundle are copied to their public,
content-hashed URL paths. A page and a static file cannot write the same output
path.

# Calling the exporter directly

The same renderer is available as an in-process API:

```rust
# use topcoat::{
#     ExportConfig, ExportPathStyle,
#     router::Router,
# };
# async fn example(router: Router) -> Result<(), topcoat::ExportError> {
let report = topcoat::export(
    router,
    ExportConfig::new("dist").path_style(ExportPathStyle::Directory),
)
.await?;

println!("rendered {} pages", report.pages());
# Ok(())
# }
```

The direct API writes into the configured directory. The CLI adds staging and
atomic replacement around it.
