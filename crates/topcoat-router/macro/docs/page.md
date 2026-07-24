Declares a page handler.

The page's URL is the path string given to the attribute (`#[page("/about")]`). When no path is given, the URL is derived from the function's enclosing module path, kebab-cased, provided the function is reachable from a [`module_router!`](macro.module_router.html). Both forms register into the same router, so explicit and module-derived paths can be mixed freely in one app.

A page serves `GET` by default. To serve other methods, name them before the path, using the same forms as [`#[route]`](attr.route.html): a single method (`#[page(POST "/signup")]`), a bracketed list (`[GET, POST]`), or `*` for every method.

Path strings are Topcoat [`Path`](struct.Path.html)s: literal segments (`users`), `{name}` for dynamic parameters, `{*name}` for wildcard tails, and `(name)` for groups (which participate in layout and layer matching but are stripped from the served URL).

A page registers like any other handler: pass the function name to [`RouterBuilder::page`](struct.RouterBuilder.html#method.page), or let [`discover`](trait.RouterBuilderDiscoverExt.html) or [`module_router!`](macro.module_router.html) collect it automatically.

After the methods and path, the attribute takes one option: `generate_static = ...`, which opts a dynamic page into a static export. See [Static export](#static-export).

# Handler signature

The function is `async` and returns [`Result`](../type.Result.html). It may take [`cx: &Cx`](../context/struct.Cx.html), one request body parameter implementing [`FromRequest`](trait.FromRequest.html), both, or neither. The body parameter may use a destructuring pattern such as `Json(input): Json<T>`. The parameters may appear in either order, but there can be at most one body parameter, because the body is a stream that can only be consumed once.

# Examples

Explicit path:

```rust
# use topcoat::{Result, router::page, view::view};
#[page("/users/{id}")]
async fn user_profile() -> Result {
    view! { <h1>"User profile"</h1> }
}
```

Module-derived path (in `src/app/about.rs` under `module_router!()`, this serves `/about`):

```rust
# use topcoat::{Result, router::page, view::view};
#[page]
async fn about() -> Result {
    view! { <h1>"About"</h1> }
}
```

Declaring a method (a form submission answered with a rendered view):

```rust
# use topcoat::{Result, router::{Form, page}, view::view};
# use serde::Deserialize;
# #[derive(Deserialize)]
# struct Signup { email: String }
#[page(POST "/signup")]
async fn signup(Form(input): Form<Signup>) -> Result {
    view! { <h1>"Welcome, " (input.email)</h1> }
}
```

Reading a request body:

```rust
# use topcoat::{Result, router::{Form, page}, view::view};
# use serde::Deserialize;
# #[derive(Deserialize)]
# struct Search { q: String }
#[page("/contact")]
async fn contact(Form(input): Form<Search>) -> Result {
    view! { <main>"searching for " (input.q)</main> }
}
```

# Pages as components

A page doubles as a [component](../view/attr.component.html): calling it inside [`view!`](../view/macro.view.html) renders it inline. A page that reads a request body takes the already-parsed value as a `body` prop instead of parsing the request.

```rust
# use topcoat::{Result, router::{Form, page}, view::view};
# use serde::Deserialize;
# #[derive(Deserialize)]
# struct Search { q: String }
# #[page("/contact")]
# async fn contact(Form(input): Form<Search>) -> Result {
#     view! { <main>"searching for " (input.q)</main> }
# }
#[page("/preview")]
async fn preview() -> Result {
    let query = Search {
        q: String::from("topcoat"),
    };
    view! {
        contact(body: Form(query))
    }
}
```

# Static export

[`topcoat export`](https://docs.rs/topcoat-cli/latest/topcoat_cli/export/index.html) renders an application into a directory of files for a static host. A page with a fixed path is exported as it stands. A page whose path has dynamic segments has no single URL to render, so it opts in by naming a generator with `generate_static`:

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

The generator is an `async fn` taking [`cx: &Cx`](../context/struct.Cx.html) and returning one [`StaticParams`](struct.StaticParams.html) per URL the page is exported at. It runs inside a regular request, so it can read the app context and anything else a page handler can.

Each set must name **every** dynamic segment in the page's path, including the segments its parent modules contribute. A page in `src/app/blog/year/slug.rs` serves `/blog/{year}/{slug}` even though its own module only contributes `{slug}`, so its generator supplies both `year` and `slug`. A set that leaves one out, names something the path does not declare, or names one twice fails the export with a message naming the page.

The option works the same for a module-derived path and an explicit one, and follows the path when both are given:

```rust
# use topcoat::{Result, context::Cx, router::{StaticParams, page}, view::view};
# async fn generate_static_params(_cx: &Cx) -> Result<Vec<StaticParams>> { Ok(vec![]) }
#[page("/tags/{tag}", generate_static = generate_static_params)]
async fn tag() -> Result {
    view! { <h1>"Tag"</h1> }
}
```

`generate_static` is accepted only here, not by [`segment!`](macro.segment.html) or [`#[path_param]`](attr.path_param.html): those describe a segment shared by every item in a module, while the URLs to export belong to one page. A page that does not answer `GET` cannot be exported, so declaring a generator on one is an error.
