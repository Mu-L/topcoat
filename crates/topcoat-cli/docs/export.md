Builds a deployable static site from a Topcoat application.

```sh
topcoat export
```

`topcoat export` compiles the application, bundles its assets, starts it, and
renders every page it can serve statically into a directory of files ready to
upload to any static host. The application itself decides what its static site
contains: the CLI asks it, fetches each URL over HTTP, and writes the responses
to disk.

```text
dist/
	index.html
	about/index.html
	blog/index.html
	blog/2026/static-export/index.html
	_topcoat/assets/site-4b2ef95524e01652.css
```

# Options

| Option | Default | Description |
|--------|---------|-------------|
| `--out <DIR>`, `-o` | `dist` | Directory the site is written to. |
| `--format <FORMAT>` | `directory` | How page URLs map to files; see [Output formats](#output-formats). |
| `--package <NAME>`, `-p` | | Build the named workspace package. |
| `--bin <NAME>` | | Build the named binary target. |
| `--release`, `-r` | | Build with the `release` profile. |
| `--profile <NAME>` | | Build with the named cargo profile. |

# What gets exported

A page with a **fixed path** is exported as it stands, with no opt-in:

```rust
# use topcoat::{Result, router::page, view::view};
// Exported to `about/index.html`.
#[page("/about")]
async fn about() -> Result {
    view! { <h1>"About"</h1> }
}
```

A page whose path has **dynamic segments** has no single URL to render, so it is
left out until it says which URLs to export it for. Everything else the
application serves is left out too:

- Routes declared with [`#[route]`](https://docs.rs/topcoat/latest/topcoat/router/attr.route.html) (JSON APIs and the like), which have no static representation.
- Pages that do not answer `GET`, such as `#[page(POST "/signup")]`.
- Assets hosted externally with [`AssetConfig::hosted_at`](https://docs.rs/topcoat/latest/topcoat/asset/struct.AssetConfig.html#method.hosted_at), which the application does not serve.

# Exporting a dynamic page

Give the page a generator with `generate_static`. It is an `async fn` taking the
request [`Cx`](https://docs.rs/topcoat/latest/topcoat/context/struct.Cx.html) and returning one
[`StaticParams`](https://docs.rs/topcoat/latest/topcoat/router/struct.StaticParams.html) per URL the page should be
exported at:

```rust
# use topcoat::{Result, context::Cx, router::{StaticParams, page}, view::view};
# struct Post { slug: &'static str }
# fn published_posts() -> Vec<Post> { vec![Post { slug: "hello" }] }
async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(published_posts()
        .iter()
        .map(|entry| StaticParams::from([("slug", entry.slug)]))
        .collect())
}

#[page("/blog/{slug}", generate_static = generate_static_params)]
async fn post() -> Result {
    view! { <h1>"Post"</h1> }
}
```

The generator runs inside a regular request, so it can read the app context,
query a database, or reach anything else a page handler can. Returning an empty
list is fine: the page is simply not exported. Returning an error fails the
export with a message naming the page.

`generate_static` belongs to [`#[page]`](https://docs.rs/topcoat/latest/topcoat/router/attr.page.html) alone. It is
not accepted by [`segment!`](https://docs.rs/topcoat/latest/topcoat/router/macro.segment.html) or
[`#[path_param]`](https://docs.rs/topcoat/latest/topcoat/router/attr.path_param.html), which describe a segment
shared by every item in a module rather than one page's URLs.

## Every set names every parameter

A parameter set describes a whole URL, so it must name **every** dynamic segment
in the page's path, including the segments its parent modules contribute. A page
in `src/app/blog/year/slug.rs` serves `/blog/{year}/{slug}` even though its own
module only contributes `{slug}`, and its generator has to supply both:

```rust
# use topcoat::{Result, context::Cx, router::StaticParams};
# struct Post { year: &'static str, slug: &'static str }
# fn published_posts() -> Vec<Post> { vec![Post { year: "2026", slug: "hello" }] }
async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(published_posts()
        .iter()
        .map(|entry| StaticParams::from([("year", entry.year), ("slug", entry.slug)]))
        .collect())
}
```

A set that leaves `year` out, or that names something the path does not declare,
fails the export rather than producing a URL nobody asked for.

## Explicit and module-derived paths

Both forms opt in the same way and land in the same export:

```rust
# use topcoat::{Result, context::Cx, router::{StaticParams, page}, view::view};
# async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> { Ok(vec![]) }
// A path derived from the module tree.
#[page(generate_static = generate_static_params)]
async fn post() -> Result {
    view! { <h1>"Post"</h1> }
}
```

```rust
# use topcoat::{Result, context::Cx, router::{StaticParams, page}, view::view};
# async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> { Ok(vec![]) }
// An explicit path, with the methods form (`#[page(GET "/tags/{tag}", ...)]`)
// available as usual.
#[page("/tags/{tag}", generate_static = generate_static_params)]
async fn tag() -> Result {
    view! { <h1>"Tag"</h1> }
}
```

A page with an explicit path is registered by
[`RouterBuilder::discover_pages`](https://docs.rs/topcoat/latest/topcoat/router/struct.RouterBuilder.html#method.discover_pages)
or by name, not by [`module_router!`](https://docs.rs/topcoat/latest/topcoat/router/macro.module_router.html), so a
module router needs both to see every page:

```rust
# use topcoat::router::Router;
# mod app {
# pub fn build() -> topcoat::router::Router {
topcoat::router::module_router!().discover_pages().build()
# }
# }
```

## Parameter values

A `{name}` parameter stands for exactly one URL segment, so its value must be
non-empty and must not contain a `/`. A `{*name}` catch-all stands for the rest
of the URL, so its value may contain `/`. Values are written into the URL as
given: the export escapes them when it requests the page, and names the output
file after the unescaped value.

# Assets

Every asset the application serves is copied into the site at the URL it is
served from, byte for byte, so the content-hashed links in the exported HTML
resolve:

```rust
# use topcoat::{asset::{AssetBundle, RouterBuilderAssetExt}, router::Router};
# fn router() -> Router {
Router::builder().assets(AssetBundle::load().unwrap()).build()
# }
```

Assets [hosted externally](https://docs.rs/topcoat/latest/topcoat/asset/struct.AssetConfig.html#method.hosted_at)
are not part of the export; they are already served from somewhere else.

To add files of your own to an export, declare their URLs with
[`RouterBuilder::static_files`](https://docs.rs/topcoat/latest/topcoat/router/struct.RouterBuilder.html#method.static_files).
The router must answer `GET` for each one.

# Output formats

`--format directory` (the default) writes clean directory URLs, which is what
GitHub Pages, Netlify, Cloudflare Pages, and S3 website hosting expect:

| URL | File |
|-----|------|
| `/` | `index.html` |
| `/about` | `about/index.html` |
| `/blog/hello` | `blog/hello/index.html` |

`--format file` writes HTML files instead, for hosts that resolve an
extensionless URL by appending `.html`:

| URL | File |
|-----|------|
| `/` | `index.html` |
| `/about` | `about.html` |
| `/blog/hello` | `blog/hello.html` |

Assets keep their exact URL under either format.

# Failing safely

The site is written to a staging directory beside the output, and only replaces
the output once every page and asset has been written successfully. A failed
export leaves the previous one exactly as it was, so a broken build never takes
a working site down.

An export fails, rather than writing part of a site, when:

- A generated parameter set is missing a parameter, names an unknown one, or names one twice.
- A page generates the same URL twice, or two pages generate the same URL.
- A page generates a URL that is also served as a static file.
- A page answers with anything other than `200 OK`.

Each failure names the page's route path and what is wrong with it.

# The route listing

`topcoat export` learns what to export from a development-only JSON endpoint the
application serves at `/_topcoat/routes/static`:

```json
{
	"pages": [{ "path": "/blog/hello", "route": "/blog/{slug}" }],
	"assets": ["/_topcoat/assets/site-4b2ef95524e01652.css"]
}
```

The endpoint is registered under exactly the same condition as Topcoat's other
development tooling: the application is running under the `topcoat` CLI, which
hands it the CLI's address in `TOPCOAT_DEV_URL`. A deployed application is
started without that variable and never registers the route, so the listing is
unreachable in production.

Requests the export makes are marked with an `x-topcoat-export` header, and the
[`dev::script`](https://docs.rs/topcoat/latest/topcoat/dev/attr.script.html) live-reload script renders nothing for
them: an exported page carries no development tooling.

# Limitations

- The application has to serve over TCP with [`topcoat::serve`](https://docs.rs/topcoat/latest/topcoat/fn.serve.html) or [`topcoat::start`](https://docs.rs/topcoat/latest/topcoat/fn.start.html), which is what reports its address to the CLI. An application serving on a Unix socket cannot be exported.
- Pages are rendered by the built application, so a page that depends on a database or another service needs that service reachable while the export runs.
- Only `GET` pages are exported. Form submissions, API routes, and anything else needing a server keep needing one; a static export is for the parts of a site that do not.
- Client-side behavior is untouched: runtime expressions, event handlers, and bind attributes keep working, but [`#[procedure]`](https://docs.rs/topcoat/latest/topcoat/runtime/attr.procedure.html) and [`#[shard]`](https://docs.rs/topcoat/latest/topcoat/runtime/attr.shard.html) call back into a running application and will not work on a static host.
