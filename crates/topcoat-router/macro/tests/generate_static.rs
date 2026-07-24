//! `#[page(generate_static = ...)]`: the pages a static export covers, for
//! both explicit and module-derived paths.

use topcoat::{
    Result,
    context::Cx,
    router::{Router, STATIC_ROUTES_PATH, StaticParams, page},
    view::view,
};

mod common;
use common::send;

/// The route tree, rooted here so `module_router!` derives paths from it.
mod app {
    use topcoat::router::{Router, StaticRoutes};

    use super::{Cx, Result, StaticParams, page, view};

    /// The application's router, with the static route listing registered
    /// explicitly: the router registers it itself only under the `topcoat`
    /// CLI, which a test is not running under.
    ///
    /// `discover_pages` adds the pages declared with an explicit path, so both
    /// forms end up in one router and one listing.
    pub fn router() -> Router {
        let builder = topcoat::router::module_router!().discover_pages();
        let listing = StaticRoutes::new(
            builder.static_pages(),
            vec!["/_topcoat/assets/logo-1a2b3c4d.png".to_owned()],
        );
        builder.route(listing).build()
    }

    // A fixed page. It is exported without opting in.
    #[page]
    pub async fn home() -> Result {
        view! { <h1>"home"</h1> }
    }

    pub mod blog {
        // `/blog/{year}/{slug}`: the page declares only its own module's
        // segment, but its generator has to name `year` as well.
        pub mod year {
            topcoat::router::segment!(kind = Param);

            pub mod slug {
                use super::super::super::{Cx, Result, StaticParams, page, view};

                topcoat::router::segment!(kind = Param);

                #[allow(
                    clippy::unused_async,
                    reason = "a generator is async so it can query for its parameters"
                )]
                async fn posts(_cx: &Cx) -> Result<Vec<StaticParams>> {
                    Ok(vec![
                        StaticParams::from([("year", "2025"), ("slug", "first")]),
                        StaticParams::from([("year", "2026"), ("slug", "second")]),
                    ])
                }

                #[page(generate_static = posts)]
                pub async fn post(cx: &Cx) -> Result {
                    let path = topcoat::router::uri(cx).path().to_owned();
                    view! {
                        <h1>"post"</h1>
                        <p>(path)</p>
                    }
                }
            }
        }
    }

    // A dynamic page with no generator: reachable at run time, left out of a
    // static export.
    pub mod users {
        pub mod id {
            use super::super::{Result, page, view};

            topcoat::router::segment!(kind = Param);

            #[page]
            pub async fn user() -> Result {
                view! { <h1>"user"</h1> }
            }
        }
    }
}

// An explicit path opts in the same way, and lands in the same listing.
#[allow(
    clippy::unused_async,
    reason = "a generator is async so it can query for its parameters"
)]
async fn guides(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(vec![StaticParams::from([("guide", "install")])])
}

#[page("/guides/{guide}", generate_static = guides)]
async fn guide(cx: &Cx) -> Result {
    let path = topcoat::router::uri(cx).path().to_owned();
    view! {
        <h1>"guide"</h1>
        <p>(path)</p>
    }
}

// A page that answers a method other than `GET` never reaches a static host.
#[page(POST "/subscribe")]
async fn subscribe() -> Result {
    view! { <p>"subscribed"</p> }
}

/// The listing served at [`STATIC_ROUTES_PATH`].
async fn listing(router: &Router) -> serde_json::Value {
    let (status, body) = send(router, STATIC_ROUTES_PATH).await;
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// The listing's page paths.
async fn listed_pages(router: &Router) -> Vec<String> {
    listing(router).await["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn fixed_pages_are_exported_without_opting_in() {
    let pages = listed_pages(&app::router()).await;
    assert!(pages.contains(&"/".to_owned()), "{pages:?}");
}

#[tokio::test]
async fn a_module_derived_page_is_exported_once_per_generated_parameter_set() {
    let pages = listed_pages(&app::router()).await;
    assert!(pages.contains(&"/blog/2025/first".to_owned()), "{pages:?}");
    assert!(pages.contains(&"/blog/2026/second".to_owned()), "{pages:?}");
}

#[tokio::test]
async fn an_explicit_path_is_exported_like_any_other() {
    let pages = listed_pages(&app::router()).await;
    assert!(pages.contains(&"/guides/install".to_owned()), "{pages:?}");
}

#[tokio::test]
async fn a_dynamic_page_without_a_generator_is_left_out() {
    let pages = listed_pages(&app::router()).await;
    assert!(
        !pages.iter().any(|path| path.starts_with("/users/")),
        "{pages:?}"
    );
}

#[tokio::test]
async fn a_non_get_page_is_left_out() {
    let pages = listed_pages(&app::router()).await;
    assert!(!pages.contains(&"/subscribe".to_owned()), "{pages:?}");
}

#[tokio::test]
async fn the_listing_names_the_route_each_page_came_from() {
    let listing = listing(&app::router()).await;
    let post = listing["pages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["path"] == "/blog/2025/first")
        .unwrap();
    assert_eq!(post["route"], "/blog/{year}/{slug}");
}

#[tokio::test]
async fn declared_static_files_are_listed() {
    let listing = listing(&app::router()).await;
    assert_eq!(
        listing["assets"].as_array().unwrap(),
        &vec![serde_json::json!("/_topcoat/assets/logo-1a2b3c4d.png")]
    );
}

#[tokio::test]
async fn every_listed_page_renders() {
    let router = app::router();
    for path in listed_pages(&router).await {
        let (status, body) = send(&router, &path).await;
        assert_eq!(status, 200, "{path}: {body}");
    }
}

#[tokio::test]
async fn a_generated_page_renders_at_its_generated_url() {
    let (status, body) = send(&app::router(), "/blog/2026/second").await;
    assert_eq!(status, 200);
    assert!(body.contains("/blog/2026/second"), "{body}");
}
