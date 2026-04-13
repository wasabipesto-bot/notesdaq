use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::blog::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/search", get(search))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    page: Option<i64>,
    per_page: Option<i64>,
}

#[derive(Serialize)]
struct SearchResult {
    slug: String,
    avatar_url: Option<String>,
    follower_count: Option<i64>,
    title: Option<String>,
    has_positions: bool,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
    total: i64,
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let pool = &state.pool;
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (params.page.unwrap_or(1).max(1) - 1) * per_page;
    let pattern = format!("%{}%", params.q.replace('%', "\\%"));

    let (total,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM blogs WHERE slug ILIKE $1 OR title ILIKE $1",
    )
    .bind(&pattern)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query_as::<_, (String, Option<String>, Option<i64>, Option<String>, String)>(
        r#"
        SELECT b.slug, b.avatar_url, b.follower_count, b.title, b.uuid
        FROM blogs b
        WHERE b.slug ILIKE $1 OR b.title ILIKE $1
        ORDER BY
            CASE WHEN b.slug = $2 THEN 0
                 WHEN b.slug ILIKE $2 || '%' THEN 1
                 ELSE 2 END,
            b.follower_count DESC NULLS LAST
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(&pattern)
    .bind(&params.q)
    .bind(per_page)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    for r in rows {
        let has_positions: (i64,) = sqlx::query_as(
            "SELECT count(*) FROM reblog_events WHERE blog_uuid = $1 LIMIT 1",
        )
        .bind(&r.4)
        .fetch_one(pool)
        .await?;

        results.push(SearchResult {
            slug: r.0,
            avatar_url: r.1,
            follower_count: r.2,
            title: r.3,
            has_positions: has_positions.0 > 0,
        });
    }

    Ok(Json(SearchResponse { results, total }))
}
