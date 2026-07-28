`path_param!` declares a path parameter by the name it has in the URL and generates the type used to read it. It replaces the `#[path_param]` attribute.

```rust
// src/app/posts/post_id.rs
use topcoat::{Result, context::Cx, router::{page, path_param}, view::view};

path_param!(post_id: u64, error = not_found);

#[page]
async fn post(cx: &Cx) -> Result {
    let post_id = path_param::<PostId>(cx)?;
    view! { <h1>"Post " (post_id)</h1> }
}
```

One declaration serves both directions. The page reads a `PostId` out of the request, and a link elsewhere builds one to point back at the page with `href!`. That macro is specified in the [URL generation design](https://github.com/tokio-rs/topcoat/pull/225); this document assumes only that a link takes a constructed parameter value.

# Declaring a parameter

A declaration is a visibility, the parameter name, and the type the segment parses into.

```rust
path_param!(post_id: u64);                      // struct PostId(u64);
path_param!(post_id: u64, error = not_found);   // the same type; a bad parse answers 404
path_param!(pub slug);                          // pub struct Slug<T: AsRef<str> = String>(pub T);
```

The name is spelled the way it appears in the URL, and the type is that name in Pascal case, so `post_id` matches `{post_id}` and generates `PostId`. The type and its field take the declared visibility, and the generic appears only on the unparsed forms, which do not fix the type of the value they hold.

A descendant module reads a private type through ordinary Rust visibility, so the pages under `posts/post_id.rs` read `PostId` without `pub`. A link from a parent or a sibling module needs it.

The attribute went the other way, snake-casing the name of the struct you wrote into a URL name. Naming the parameter first puts the URL in the declaration and leaves the type to the macro, which is what the unparsed form needs: an attribute cannot add a generic without handing back a different struct than the one you wrote.

# Where the parameter appears in the URL

A declaration emits the segment override `segment!(kind = Param, rename = "post_id")` writes by hand. Under [`module_router!`](../crates/topcoat-router/docs/module_router.md) that turns the declaring module's segment into the parameter, so there is no placeholder to write and the file name does not matter.

```text
src/app/posts/post_id.rs      // path_param!(post_id: u64) serves /posts/{post_id}
```

A page with an explicit path writes the placeholder itself, and the names have to match.

```rust
path_param!(post_id: u64);

#[page("/posts/{post_id}")]
async fn post(cx: &Cx) -> Result { /* ... */ }
```

Reading a parameter that the matched path never captured panics with `path parameter "post_id" was not found in request path`, which is the failure mode of a misspelled placeholder.

The override is per module, so a module declares one parameter and does not also call `segment!`. Under `module_router!`, a second declaration in the same module panics during discovery with `duplicate segment specifier`, so a path that captures two parameters declares them in two modules.

```text
src/app/organizations/organization_id/users/user_id.rs   // /organizations/{organization_id}/users/{user_id}
```

A handler reads any ancestor module's parameter whose type is visible from it. See the [module router guide](../crates/topcoat-router/docs/module_router.md) for how modules stack.

# Reading the value

`path_param::<T>(cx)` is unchanged. The function and the macro share a name and live in different namespaces, so one import covers both.

```rust
use topcoat::router::path_param;
```

A parsed parameter returns a `Result` holding a reference to the parsed value, parsed at most once per request. The type has to implement `FromStr`, and the parse result has to be `Send + Sync + 'static` so it can be memoized.

```rust
// with `error = not_found` on the declaration
let post_id: &u64 = path_param::<PostId>(cx)?;
```

An unparsed parameter returns the percent-decoded segment as a `&str` borrowed from the request. It cannot fail, so there is nothing to unwrap, and reading it adds no allocation.

```rust
let slug: &str = path_param::<Slug>(cx);
```

`Slug` carries a type parameter for the string it holds, defaulting to `String` so that reading names the type on its own. The URL generation design writes the type without a default.

# Failing with an error response

`error = ...` maps a failed parse to a router error, so `?` in the handler answers the request instead of bubbling a parse error up.

```rust
path_param!(post_id: u64, error = bad_request("Post ID must be a number"));
```

The forms are the ones the attribute takes today: `not_found`, `unauthorized`, `forbidden`, `bad_request`, `redirect(...)`, and `redirect_permanent(...)`, each mirroring the [router error](../crates/topcoat-router/docs/error.md) constructor it names. `bad_request` is the one that carries a description, and a bare `error = bad_request` keeps its generated default, `invalid value for path parameter "post_id"`. Without an error form, the `Err` side is a reference to the `FromStr` error, and each call site picks its own response through `RouterErrorExt`.

An unparsed parameter has nothing to fail at, so `error` on one is a compile error.

# Building a value

The declaration produces a type you can construct, which is what a link needs.

```rust
// src/app/posts.rs links to the page under src/app/posts/post_id.rs,
// which declares `path_param!(pub post_id: u64)` for this import.
use crate::app::posts::post_id::{PostId, post};

href!(post, PostId(42))       // "/posts/42"
```

The unparsed type holds anything that borrows as a string, so a link passes a value it owns and a function can return one. A call site infers the type argument from the value; a signature has nothing to infer from and takes the `String` default.

```rust
// src/app/posts/slug.rs declares `path_param!(pub slug)` and the page `show`.
use crate::app::posts::slug::{Slug, show};

href!(show, Slug("my-first-post"))          // a Slug<&str>
href!(show, Slug(post.slug.clone()))        // a Slug<String>

fn slug_of(post: &Post) -> Slug {           // Slug<String>
    Slug(post.slug.clone())
}
```

`struct Slug(str)` reads a request fine, but its field is unsized, so no link can build one. That is what the unparsed form is for.

# Catch-all parameters

A `*` declares a parameter that matches the rest of the path. It holds a sequence of segments rather than one string. Each segment is percent-decoded on its own, so an encoded `/` inside a segment stays distinct from the separator between two.

```rust
path_param!(pub *doc_path);      // pub struct DocPath<T: IntoIterator<Item: AsRef<str>> = Vec<String>>(pub T);
path_param!(pub *ids: u32);      // pub struct Ids(pub Vec<u32>);
```

The type after `:` is what one segment parses into, the same as for a single-segment parameter. There is one per segment, so reading returns a sequence. `Ids` is not generic. A parsed catch-all knows its element type, so a link that holds a slice calls `.to_vec()`.

A declaration emits `segment!(kind = CatchAll, rename = "doc_path")`, which puts three rules on where it can appear:

- One per module, like any other parameter. A second declaration panics with `duplicate segment specifier`.
- Last served segment. A route with anything after the catch-all fails in `Router::build`, which panics with `failed to register route` carrying matchit's `InvalidCatchAll`.
- One segment at least, so `/docs` does not reach a page under `{*doc_path}`.

Reading yields the segments.

```rust
let doc_path: PathSegments<'_> = path_param::<DocPath>(cx);   // "guides", "getting-started"
let ids: &[u32] = path_param::<Ids>(cx)?;
```

`topcoat::router::PathSegments` is an iterator over the decoded segments the request captured, yielding `&str`. A page that serves files walks it once, rejecting `..` and pushing the rest onto a `PathBuf`. Where the segments are already trusted, `collect::<PathBuf>()` is the short form.

`path_param!(*doc_path: PathBuf)` is a different declaration rather than an error. The type after `:` is per segment, so it parses each segment into its own `PathBuf`.

A parsed catch-all parses every segment and memoizes the collection. The first segment that fails to parse is the error the parameter returns.

`error = ...` maps that failure the way it does elsewhere. For `/archive/1/x/3`, a bare `error = bad_request` describes it as `invalid value for path parameter "ids" at segment 1`, counting from zero. Without an error form, the `Err` side is the `FromStr` error alone and does not say which segment produced it.

A link passes the segments, and each one is encoded on the way in.

```rust
href!(document, DocPath(["guides", "getting-started"]))    // "/docs/guides/getting-started"
href!(archive, Ids(vec![1, 2, 3]))                         // "/archive/1/2/3"
```

`DocPath("guides/getting-started")` does not compile, since `&str` is not an `IntoIterator` of segments. A string with a `/` in it has already lost the distinction the encoder needs, and encoding it whole gives `/docs/guides%2Fgetting-started`.

Naming the collection instead of the segment type, `path_param!(*ids: Vec<u32>)`, was rejected on inference. The element type would come from `C: FromIterator<T>`, and `PathBuf` implements `FromIterator<P>` for every `P: AsRef<Path>`, so nothing determines `T`. Naming the segment type keeps one rule for both forms and leaves the container to `collect`.

This part of the design changes the router runtime, not only the macro. `RawPathParams` decodes each captured value as a unit, so a catch-all arrives with no segment boundaries. The typed form reads a tail stored as its decoded segments, and the untyped `segment!` capture then yields the raw tail, still encoded.

# Migrating

`#[path_param]` is removed rather than deprecated, and every declaration moves to the macro.

| Before | After |
|---|---|
| `#[path_param] struct PostId(u64);` | `path_param!(post_id: u64);` |
| `#[path_param(error = bad_request)] struct PostId(u64);` | `path_param!(post_id: u64, error = bad_request);` |
| `#[path_param] pub struct Slug(str);` | `path_param!(pub slug);` |
| `segment!(kind = CatchAll, rename = "path");` | `path_param!(*path);`, when the tail is read as a parameter |

Visibility moves from the struct to the front of the declaration, where only a parameter that appears in a link needs `pub`. The type name now follows the URL name, so a struct whose name did not round-trip through snake case, such as `PostID`, is renamed along with the handlers that read it. Reading a single segment is unchanged, so those handler bodies stay as they are.

The catch-all row is the one that costs work. A handler that read the tail as one string now iterates `PathSegments`, and the untyped `segment!(kind = CatchAll)` capture it used to read gives back the raw tail rather than a decoded string. That row also generates a type named `Path`, which collides with `std::path::Path` in a module that imports it. Renaming the parameter renames the placeholder with it, so an explicit path spelling `{*path}` changes too, though no URL a browser requests moves.

The declaration appears in prose and code that changes with it:

- [`router.md`](../crates/topcoat/docs/router.md), [`module_router.md`](../crates/topcoat-router/docs/module_router.md), [`context.md`](../crates/topcoat/docs/context.md), and [`error.md`](../crates/topcoat-router/docs/error.md)
- the [`path_param`](../crates/topcoat-router/macro/docs/path_param.md) and [`segment`](../crates/topcoat-router/macro/docs/segment.md) macro pages
- `AGENTS.md`
- the `path-query-params` and `toasty-todo` examples
- the [URL generation design](https://github.com/tokio-rs/topcoat/pull/225), whose catch-all example passes a string
