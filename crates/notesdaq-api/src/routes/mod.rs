pub mod blog;
pub mod meta;
pub mod post;
pub mod search;
pub mod ticker;

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(blog::router())
        .merge(post::router())
        .merge(ticker::router())
        .merge(search::router())
        .merge(meta::router())
}
