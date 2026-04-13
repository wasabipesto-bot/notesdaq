use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use super::blog::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/stats", get(get_stats))
        .route("/health", get(health))
}

#[derive(Serialize)]
struct Stats {
    total_blogs: i64,
    blogs_crawled: i64,
    total_posts: i64,
    total_reblog_events: i64,
    total_likes: i64,
    posts_enumerated: i64,
    crawl_queue_pending: i64,
}

async fn get_stats(
    State(state): State<AppState>,
) -> Result<Json<Stats>, ApiError> {
    let pool = &state.pool;

    let row = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT count(*) FROM blogs),
            (SELECT count(*) FROM crawl_queue WHERE status = 'completed'),
            (SELECT count(*) FROM posts),
            (SELECT count(*) FROM reblog_events),
            (SELECT count(*) FROM likes),
            (SELECT count(*) FROM posts WHERE notes_fully_enumerated),
            (SELECT count(*) FROM crawl_queue WHERE status = 'pending')
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(Json(Stats {
        total_blogs: row.0,
        blogs_crawled: row.1,
        total_posts: row.2,
        total_reblog_events: row.3,
        total_likes: row.4,
        posts_enumerated: row.5,
        crawl_queue_pending: row.6,
    }))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}
