use topcoat::{
    Result,
    context::Cx,
    router::{Router, StaticParams, is_static_export, page, raw_path_params},
    view::view,
};

mod app {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static PRODUCT_GENERATIONS: AtomicUsize = AtomicUsize::new(0);

    pub fn router() -> Router {
        topcoat::router::module_router!().build()
    }

    pub fn reset_generation_count() {
        PRODUCT_GENERATIONS.store(0, Ordering::Relaxed);
    }

    pub fn generation_count() -> usize {
        PRODUCT_GENERATIONS.load(Ordering::Relaxed)
    }

    #[page]
    async fn home(cx: &Cx) -> Result {
        view! { <p>"home " (is_static_export(cx))</p> }
    }

    mod products {
        pub mod product_id {
            use super::super::*;
            use topcoat::router::path_param;

            #[allow(clippy::unused_async)]
            async fn generate_product_ids(_cx: &Cx) -> Result<Vec<u32>> {
                PRODUCT_GENERATIONS.fetch_add(1, Ordering::Relaxed);
                Ok(vec![7, 42])
            }

            #[path_param(generate_static = generate_product_ids)]
            pub(super) struct ProductId(u32);

            #[page]
            async fn product(cx: &Cx) -> Result {
                let product_id = path_param::<ProductId>(cx).unwrap();
                view! { <p>"product " (product_id)</p> }
            }

            mod reviews {
                mod review_id {
                    use super::super::super::super::*;
                    use super::super::ProductId;
                    use topcoat::router::path_param;

                    #[allow(clippy::unused_async)]
                    async fn generate_review_ids(cx: &Cx) -> Result<Vec<u32>> {
                        let product_id = path_param::<ProductId>(cx).unwrap();
                        Ok(vec![*product_id * 10])
                    }

                    #[path_param(generate_static = generate_review_ids)]
                    struct ReviewId(u32);

                    #[page]
                    async fn review(cx: &Cx) -> Result {
                        let review_id = path_param::<ReviewId>(cx).unwrap();
                        view! { <p>"review " (review_id)</p> }
                    }
                }
            }
        }
    }

    mod docs {
        mod rest {
            use super::super::*;

            #[allow(clippy::unused_async)]
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

            #[page]
            async fn document(cx: &Cx) -> Result {
                let path = raw_path_params(cx)
                    .iter()
                    .find_map(|(name, value)| (name == "path").then_some(value))
                    .unwrap();
                view! { <p>(path)</p> }
            }
        }
    }
}

#[allow(clippy::unused_async)]
async fn generate_catalog(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(vec![
        StaticParams::new()
            .param("category", "books")
            .catch_all("slug", ["rust", "async"]),
        StaticParams::new()
            .param("category", "home and garden")
            .catch_all("slug", ["lighting"]),
    ])
}

#[page(
    "/catalog/{category}/{*slug}",
    generate_static = generate_catalog
)]
async fn catalog(cx: &Cx) -> Result {
    view! { <p>(is_static_export(cx))</p> }
}

#[page("/missing/{value}")]
async fn missing_generator() -> Result {
    view! { <p>"missing"</p> }
}

#[allow(clippy::unused_async)]
async fn generate_featured(_cx: &Cx) -> Result<Vec<StaticParams>> {
    Ok(vec![StaticParams::new().param("slug", "featured")])
}

#[page("/blog/{slug}", generate_static = generate_featured)]
async fn dynamic_blog_post() -> Result {
    view! { <p>"dynamic"</p> }
}

#[page("/blog/featured")]
async fn fixed_blog_post() -> Result {
    view! { <p>"fixed"</p> }
}

#[tokio::test]
async fn module_segments_generate_concrete_static_paths() {
    app::reset_generation_count();
    let paths = app::router().generate_static_paths().await.unwrap();
    let paths: Vec<_> = paths
        .iter()
        .map(topcoat::router::StaticPath::url_path)
        .collect();
    assert_eq!(
        paths,
        [
            "/",
            "/docs/guide/start",
            "/docs/reference",
            "/products/42",
            "/products/42/reviews/420",
            "/products/7",
            "/products/7/reviews/70",
        ]
    );
    assert_eq!(app::generation_count(), 1);
}

#[tokio::test]
async fn explicit_pages_generate_complete_parameter_sets() {
    let router = Router::builder().page(catalog).build();
    let paths = router.generate_static_paths().await.unwrap();
    let paths: Vec<_> = paths
        .iter()
        .map(topcoat::router::StaticPath::url_path)
        .collect();
    assert_eq!(
        paths,
        [
            "/catalog/books/rust/async",
            "/catalog/home%20and%20garden/lighting",
        ]
    );
}

#[tokio::test]
async fn explicit_dynamic_pages_require_a_generator() {
    let router = Router::builder().page(missing_generator).build();
    let error = router.generate_static_paths().await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires `generate_static` on `#[page]`")
    );
}

#[tokio::test]
async fn duplicate_concrete_urls_are_rejected() {
    let router = Router::builder()
        .page(dynamic_blog_post)
        .page(fixed_blog_post)
        .build();
    let error = router.generate_static_paths().await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("multiple pages generate the static URL `/blog/featured`")
    );
}
