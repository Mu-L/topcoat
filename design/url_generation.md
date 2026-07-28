Topcoat generates URLs from the pages that serve them. `href!` builds a page's URL from its path parameters, so a link names the handler it points at instead of a string that goes stale when the route moves.

```rust
use topcoat::{Result, router::{href, page}, view::view};

#[page]
async fn home() -> Result {
    view! {
        <a href=(href!(about))>"About"</a>
    }
}
```

# Building a URL

`href!` takes the page's handler name, then one value per path parameter, in the order the parameters appear in the path.

```rust
href!(about)                                  // "/about"
href!(post, PostId(42))                       // "/posts/42"
href!(user, OrganizationId(9), UserId(41))    // "/organizations/9/users/41"
```

Path parameters are the [`#[path_param]`](../crates/topcoat-router/macro/docs/path_param.md) newtypes the page already reads from the request, so one type serves both directions.

```rust
// src/app/posts/post_id.rs
#[path_param(error = not_found)]
pub struct PostId(pub u64);

#[page]
async fn post(cx: &Cx) -> Result {
    view! { <h1>"Post " (path_param::<PostId>(cx)?)</h1> }
}
```

```rust
// src/app/posts.rs
use crate::app::posts::post_id::{PostId, post};

#[page]
async fn index() -> Result {
    view! {
        <ul>
            for id in [1_u64, 2, 3] {
                <li><a href=(href!(post, PostId(id)))>"Post " (id)</a></li>
            }
        </ul>
    }
}
```

`href!` rewrites to a method on the page, passing the parameters as a tuple.

```rust
href!(user, OrganizationId(9), UserId(41))
user.href((OrganizationId(9), UserId(41)))
```

The method takes exactly one argument because Rust has no variadic functions, and one argument holding any number of parameters means a tuple. Zero and one read worst.

```rust
about.href(())
post.href((PostId(42),))
```

Most pages have no path parameters, so `about.href(())` is the shape a reader meets most often. `href!` spreads the parameters into that tuple, keeping it out of every link.

The tuple stays the real API. It gives every page one calling convention, and leaves a method for helpers that build URLs generically. Everything below applies to both forms.

# What href! returns

An `Href`, not a `String`. Resolving one needs the route table and the base URL, both of which live on the request context, so an `Href` describes a URL and builds it later. That is also why it cannot implement `Display`.

A view resolves it for you.

```rust
<a href=(href!(post, PostId(42)))>"Post"</a>
```

Elsewhere it takes a context.

```rust
href!(post, PostId(42)).resolve(cx)      // "/posts/42"
```

`query` and `fragment` extend an `Href` before it resolves, and the redirect constructors take one as is. Comments below show what a URL resolves to.

# Parameter values

Each value is percent-encoded into its segment, so a title or an address containing `/`, `?`, or `#` stays inside the segment it belongs to.

```rust
href!(show, Slug::new("hello/world"))     // "/posts/hello%2Fworld"
```

A catch-all segment (`{*path}`) stands for several segments, so it is the one case where `/` is preserved.

```rust
href!(document, DocPath::new("guides/getting-started"))    // "/docs/guides/getting-started"
```

A parameter declared over `str` is unsized and cannot be built with the tuple-struct constructor, so `#[path_param]` gives those types a `new` constructor that borrows the string.

```rust
#[path_param]
pub struct Slug(str);

href!(show, Slug::new("my-first-post"))
```

Group segments never appear in a served URL and take no value, so a page under `app::_marketing::pricing` is reached with `href!(pricing)`.

Linking to a page from outside its own module needs its parameter type, so declare the type and its field `pub`.

# When mistakes are caught

Parameters are checked when the URL is built, never at compile time. A macro sees only its own item, so a page cannot name parameters its ancestors declare. Explicit paths could be checked earlier, but one rule for every page beats a partial one.

A URL carries the source location it was built at, so the failure names the link rather than the page rendering it.

```text
page `app::posts::post_id::post` serves "/posts/{post_id}" and needs `post_id`, but was given `user_id`
  link built at src/app/posts.rs:31:22
```

An unregistered page reads the same way. It usually means a page with an explicit path in an application that never called `.discover()`, or a module not reachable through a `mod` declaration from the module router's root.

```text
no route registered for page `app::posts::post_id::post`
  link built at src/app/posts.rs:31:22
```

These are programming errors with no recovery, so they panic rather than render a broken link, the same way a missing [asset](../crates/topcoat/docs/asset.md) does. A view renders to a complete string before its response is built, so the panic never truncates a partly written response.

# Query strings and fragments

`query` appends a query string from any `serde::Serialize` value. `#[query_params]` structs derive `Serialize`, so the struct a page reads is the struct a link writes.

```rust
#[query_params]
pub struct PostsQuery {
    page: Option<u32>,
    q: Option<String>,
}

// "/posts?page=2&q=rust"
href!(index).query(&PostsQuery {
    page: Some(2),
    q: Some("rust".into()),
})
```

Fields holding `None` are left out. A slice of pairs covers one-off links that do not deserve a type.

```rust
href!(index).query(&[("page", 2)])
```

`fragment` appends a `#` fragment.

```rust
href!(post, PostId(42)).fragment("comments")      // "/posts/42#comments"
```

# Absolute URLs

`href!` produces a root-relative URL, which is what a link inside the site needs. Content that leaves the site, such as mail, feeds, and sitemaps, needs the absolute form. `url!` takes the same arguments and resolves against the [base URL](../crates/topcoat/docs/context.md) registered on the router.

```rust
let router = Router::builder().base_url("https://example.com").build();

url!(post, PostId(42))        // "https://example.com/posts/42"
```

An application mounted under a path prefix registers it as part of that base URL, as in `https://example.com/app`. The proxy strips the prefix before the router matches, so the router only ever sees `/posts/42` while the browser needs `/app/posts/42`. Relative URLs carry the prefix for that reason, so `href!` reads the base URL too.

# Redirecting

A Post/Redirect/Get handler names its destination the way a link does.

```rust
use topcoat::router::error::see_other;

#[page(POST)]
async fn create(Form(input): Form<NewPost>) -> Result {
    let id = insert(input).await?;
    Err(see_other(href!(post, PostId(id))).into())
}
```

# Resolving outside a view

A handler already holds the context `resolve` needs.

```rust
#[route(GET "/feed.xml")]
async fn feed(cx: &Cx) -> Result<String> {
    Ok(url!(post, PostId(42)).resolve(cx))
}
```

Work that runs outside a request, such as a background job or a sitemap task, takes a context from the router itself.

```rust
let cx = router.cx();
let url = url!(post, PostId(42)).resolve(&cx);
```

Tests use the same method. A module-derived URL resolves through the route table, so a context built by hand covers only pages with an explicit path.

```rust
let cx = app::router().cx();
assert_eq!(href!(post, PostId(42)).resolve(&cx), "/posts/42");
```

# API routes

`#[route]` handlers work the same way, which covers form actions and fetch targets.

```rust
#[route(POST "/api/posts/{post_id}/publish")]
async fn publish(cx: &Cx) -> Result<&'static str> {
    Ok("published")
}

href!(publish, PostId(42))        // "/api/posts/42/publish"
```

A `#[layout]` has no URL of its own, so it gets neither.

# Other ideas

These are worth considering but not settled.

**Give string parameters a sized type.** A `str` parameter is unsized, so it can be named but never held: not constructed, returned, stored, or collected. `Slug::new` above exists only to work around that.

```rust
#[path_param]
pub struct Slug(pub String);

path_param::<Slug>(cx)                  // still &str, still no allocation
href!(show, Slug(post.slug.clone()))
```

Reading would special-case `String` the way it special-cases `str` today, so serving a request still allocates nothing. The allocation moves to URL building, where the value usually came out of a `String` anyway. `Cow<'static, str>` avoids it for literals at the cost of a noisier type.

`String` then reads identically to `str` and does more, so the question is whether `str` stays at all.

**Render a page marker directly.** A page with no parameters could render as its own URL.

```rust
<a href=(about)>"About"</a>
```

One trait impl on the marker. The risk is two ways to write a link, with the shorter one breaking the moment the page gains a parameter.

**Choose the attribute from the element.** A `page` attribute could expand to `href` on an anchor and `action` on a form.

```rust
<a page=(post, PostId(42))>"Post"</a>
<form page=(create) method="post">
```

Overloading `href` instead is not available. `AttributeValueViewParts` is implemented for tuples, so a parenthesized list in attribute position already means "render these in order", and `href=(post, PostId(42))` has a meaning today. A new attribute name sidesteps that, at the cost of inventing one that is not HTML and hiding which attribute it produces.

**Check parameters at compile time.** Giving `module_router!` the route tree changes what the macro knows. It can pull in each module body itself, thread a module's parameters down to its children, and generate a typed `href` per page.

```rust
module_router! {
    mod about;
    mod posts { mod post_id; }
}
```

Every page then takes its parameters directly.

```rust
post.href(PostId(42))       // one argument per path parameter
post.href(UserId(42))       // does not compile
```

That drops the tuple and most of the reason `href!` exists, and it is the only approach that catches a bad link without running the code that builds it. The cost is that route modules compile as one unit and editor tooling follows them less well.
