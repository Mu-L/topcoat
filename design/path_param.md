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
path_param!(post_id: u64);                      // struct PostId(pub u64);
path_param!(post_id: u64, error = not_found);   // the same type; a bad parse answers 404
path_param!(pub slug);                          // pub struct Slug<T: AsRef<str> = String>(pub T);
```

The name is spelled the way it appears in the URL, and the type is that name in Pascal case, so `post_id` matches `{post_id}` and generates `PostId`. The type and its field take the declared visibility.

Most declarations need none. A parameter is read by the module that declares it and by its descendants, which reach a private type through ordinary Rust visibility, so the pages under `posts/post_id.rs` read `PostId` without it being `pub`. Linking is what needs `pub`, since the module holding the link is usually a parent or a sibling.

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
let post_id: &u64 = path_param::<PostId>(cx)?;
```

An unparsed parameter returns the percent-decoded segment as a `&str` borrowed from the request. It cannot fail, so there is nothing to unwrap, and reading it adds no allocation.

```rust
let slug: &str = path_param::<Slug>(cx);
```

`Slug` carries a type parameter for the string it holds, defaulting to `String`. The default is what lets `path_param::<Slug>(cx)` name the type on its own, and it covers the other position that has no value to infer from: a signature. The URL generation design writes the type without a default.

# Failing with an error response

`error = ...` maps a failed parse to a router error, so `?` in the handler answers the request instead of bubbling a parse error up.

```rust
path_param!(pub post_id: u64, error = bad_request("Post ID must be a number"));
```

The forms are the ones the attribute takes today: `not_found`, `unauthorized`, `forbidden`, `bad_request`, `redirect(...)`, and `redirect_permanent(...)`, each mirroring the [router error](../crates/topcoat-router/docs/error.md) constructor it names, and each keeping its default message. Without one, the `Err` side is a reference to the `FromStr` error, and each call site picks its own response through `RouterErrorExt`.

An unparsed parameter has nothing to fail at, so `error` on one is a compile error.

# Building a value

The declaration produces a type you can construct, which is what a link needs.

```rust
// src/app/posts.rs links to the page under src/app/posts/post_id.rs.
use crate::app::posts::post_id::{PostId, post};

href!(post, PostId(42))       // "/posts/42"
```

A link imports the parameter type alongside the handler, so a parameter that appears in one is declared `pub` where a parameter only read by its own subtree is not.

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

A `*` declares a parameter that matches the rest of the path. Its value is the remaining segments with the separators between them, so it is the one parameter that spans more than one segment.

```rust
path_param!(pub *doc_path);             // matches {*doc_path}, read as &str
path_param!(pub *doc_path: PathBuf);    // parses the tail

href!(document, DocPath("guides/getting-started"))    // "/docs/guides/getting-started"
```

A catch-all is written today as `segment!(kind = CatchAll, rename = "path")`, which captures the tail but generates no type to read it with, leaving `RawPathParams`. `segment!` keeps that job for an untyped capture, and `path_param!` covers the typed one, which the URL generation design needs in order to link to a catch-all page.

# Migrating

`#[path_param]` is removed rather than deprecated, and every declaration moves to the macro.

| Before | After |
|---|---|
| `#[path_param] struct PostId(u64);` | `path_param!(post_id: u64);` |
| `#[path_param(error = bad_request)] struct PostId(u64);` | `path_param!(post_id: u64, error = bad_request);` |
| `#[path_param] pub struct Slug(str);` | `path_param!(pub slug);` |
| `segment!(kind = CatchAll, rename = "path");` | `path_param!(*path);`, when the tail is read as a parameter |

Visibility moves from the struct to the front of the declaration. The type name now follows the URL name, so a struct whose name did not round-trip through snake case, such as `PostID`, is renamed along with the handlers that read it. The last row generates a type named `Path`, so a module that also uses `std::path` renames the parameter. Reading is unchanged, so handler bodies stay as they are.

The declaration appears in prose and code that changes with it:

- [`router.md`](../crates/topcoat/docs/router.md), [`module_router.md`](../crates/topcoat-router/docs/module_router.md), [`context.md`](../crates/topcoat/docs/context.md), and [`error.md`](../crates/topcoat-router/docs/error.md)
- the [`path_param`](../crates/topcoat-router/macro/docs/path_param.md) and [`segment`](../crates/topcoat-router/macro/docs/segment.md) macro pages
- `AGENTS.md`
- the `path-query-params` and `toasty-todo` examples
