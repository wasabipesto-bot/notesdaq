use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Serialize;

use super::blog::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/post/{post_id}", get(get_post))
        .route("/post/{post_id}/chart", get(get_price_chart))
}

#[derive(Serialize)]
struct PostDetail {
    post_id: String,
    summary: Option<String>,
    content_text: Option<String>,
    content_media: Vec<String>,
    trail_npf: Option<serde_json::Value>,
    post_url: Option<String>,
    reblog_url: Option<String>,
    tags: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    note_count: i64,
}

#[derive(Serialize)]
struct StockInfo {
    current_price: i64,
    total_investors: i64,
    total_likes: i64,
    created_at: chrono::DateTime<chrono::Utc>,
    op: Option<BlogRef>,
    fully_enumerated: bool,
}

#[derive(Serialize)]
struct BlogRef {
    slug: String,
    avatar_url: Option<String>,
    uuid: String,
}

#[derive(Serialize)]
struct PostResponse {
    post: PostDetail,
    stock: StockInfo,
}

#[derive(Serialize)]
struct PricePoint {
    t: chrono::DateTime<chrono::Utc>,
    price: i64,
    investor: Option<String>,
}

async fn get_post(
    State(state): State<AppState>,
    Path(post_id): Path<i64>,
) -> Result<Json<PostResponse>, ApiError> {
    let pool = &state.pool;

    type PostRow = (
        i64, Option<String>, Option<String>, Vec<String>, Option<serde_json::Value>,
        Option<String>, Vec<String>, chrono::DateTime<chrono::Utc>, i64,
        String, bool,
    );

    let row: PostRow = sqlx::query_as(
        r#"
        SELECT
            p.post_id, p.summary, p.content_text, p.content_media, p.trail_npf,
            p.post_url, p.tags, p.created_at, p.note_count,
            p.blog_uuid, p.notes_fully_enumerated
        FROM posts p
        WHERE p.post_id = $1
        "#,
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound("Post not found".into()))?;

    let blog_uuid = row.9.clone();
    let fully_enumerated = row.10;

    // If this is a placeholder root post (no content), try to find a reblog with content
    let (summary, content_text, content_media, trail_npf, post_url, tags) =
        if row.2.is_none() && row.5.is_none() && row.1.is_none() {
            // Look for a reblog of this post that has content
            let fallback: Option<(Option<String>, Option<String>, Vec<String>, Option<serde_json::Value>, Option<String>, Vec<String>)> =
                sqlx::query_as(
                    r#"
                    SELECT p.summary, p.content_text, p.content_media, p.trail_npf, p.post_url, p.tags
                    FROM posts p
                    WHERE p.root_post_id = $1 AND p.content_text IS NOT NULL
                    LIMIT 1
                    "#,
                )
                .bind(post_id)
                .fetch_optional(pool)
                .await?;
            fallback.unwrap_or((row.1, row.2, row.3.clone(), row.4.clone(), row.5.clone(), row.6.clone()))
        } else {
            (row.1, row.2, row.3.clone(), row.4.clone(), row.5.clone(), row.6.clone())
        };

    // If this post has no URL and OP is unknown, find a reblog URL as fallback
    let reblog_url = if post_url.is_none() {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT post_url FROM posts WHERE root_post_id = $1 AND post_url IS NOT NULL LIMIT 1",
        )
        .bind(post_id)
        .fetch_optional(pool)
        .await?
        .flatten()
    } else {
        None
    };

    let post = PostDetail {
        post_id: row.0.to_string(),
        summary,
        content_text,
        content_media,
        trail_npf,
        post_url,
        reblog_url,
        tags,
        created_at: row.7,
        note_count: row.8,
    };

    // OP blog info
    let op = sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT slug, avatar_url, uuid FROM blogs WHERE uuid = $1",
    )
    .bind(&blog_uuid)
    .fetch_optional(pool)
    .await?
    .map(|b| BlogRef {
        slug: b.0,
        avatar_url: b.1,
        uuid: b.2,
    });

    // For root posts, use this post_id directly. For reblogs, find the root.
    let root_id: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT root_post_id FROM posts WHERE post_id = $1",
    )
    .bind(post_id)
    .fetch_one(pool)
    .await?
    .unwrap_or(post_id);

    // Current price: snapshot if available, else max reblog event price
    let current_price: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(
            (SELECT reblog_count FROM reblog_count_snapshots WHERE post_id = $1 ORDER BY snapshotted_at DESC LIMIT 1),
            (SELECT max(reblog_count_at_time) FROM reblog_events WHERE post_id = $1),
            0
        )
        "#,
    )
    .bind(root_id)
    .fetch_one(pool)
    .await?;

    // Total investors (non-OP reblog events)
    let (total_investors,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM reblog_events WHERE post_id = $1 AND NOT is_op_position",
    )
    .bind(root_id)
    .fetch_one(pool)
    .await?;

    // Total likes
    let (total_likes,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM likes WHERE post_id = $1",
    )
    .bind(root_id)
    .fetch_one(pool)
    .await?;

    let stock = StockInfo {
        current_price,
        total_investors,
        total_likes,
        created_at: post.created_at,
        op,
        fully_enumerated,
    };

    Ok(Json(PostResponse { post, stock }))
}

async fn get_price_chart(
    State(state): State<AppState>,
    Path(post_id): Path<i64>,
) -> Result<Json<Vec<PricePoint>>, ApiError> {
    let pool = &state.pool;

    // Find root post
    let root_id: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT root_post_id FROM posts WHERE post_id = $1",
    )
    .bind(post_id)
    .fetch_one(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(post_id);

    // Use reblog_events as price points — each reblog IS a price change.
    // This gives exact resolution without needing periodic snapshots.
    let points = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, i64, Option<String>)>(
        r#"
        SELECT re.reblogged_at, re.reblog_count_at_time, b.slug
        FROM reblog_events re
        LEFT JOIN blogs b ON b.uuid = re.blog_uuid
        WHERE re.post_id = $1 AND NOT re.is_op_position
        ORDER BY re.reblog_count_at_time ASC
        "#,
    )
    .bind(root_id)
    .fetch_all(pool)
    .await?;

    let chart: Vec<PricePoint> = points
        .into_iter()
        .map(|r| PricePoint { t: r.0, price: r.1, investor: r.2 })
        .collect();

    Ok(Json(chart))
}
