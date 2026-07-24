mod year;

use topcoat::{Result, router::page, view::view};

use crate::posts::POSTS;

// `/blog`: a fixed path again, exported without opting in.
#[page]
async fn index() -> Result {
    view! {
        <h1>"Blog"</h1>
        <ul>
            for post in POSTS {
                <li>
                    <a href=(format!("/blog/{}/{}", post.year, post.slug))>
                        (post.title)
                    </a>
                </li>
            }
        </ul>
    }
}
