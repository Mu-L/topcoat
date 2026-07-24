//! The site's content. A real application would read this from a database or
//! the filesystem; the export runs the generators inside a request, so they
//! can reach anything a page handler can.

/// A published post.
pub struct Post {
    pub year: &'static str,
    pub slug: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

/// Every published post.
pub const POSTS: &[Post] = &[
    Post {
        year: "2025",
        slug: "hello-world",
        title: "Hello, world",
        body: "The first post.",
    },
    Post {
        year: "2026",
        slug: "static-export",
        title: "Exporting a static site",
        body: "Pages with a fixed path are exported on their own; dynamic ones opt in.",
    },
];

/// Looks up a post by the year and slug in the URL.
pub fn find(year: &str, slug: &str) -> Option<&'static Post> {
    POSTS
        .iter()
        .find(|post| post.year == year && post.slug == slug)
}

/// Every tag a feed is published for.
pub const TAGS: &[&str] = &["releases", "guides"];
