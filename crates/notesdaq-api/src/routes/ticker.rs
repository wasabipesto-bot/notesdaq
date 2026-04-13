use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use super::blog::ApiError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/ticker", get(get_ticker))
}

#[derive(Deserialize)]
pub struct TickerQuery {
    limit: Option<i64>,
    before_id: Option<i64>,
}

#[derive(Serialize)]
struct TickerEvent {
    id: i64,
    post_id: String,
    buy_price: i64,
    reblogged_at: chrono::DateTime<chrono::Utc>,
    investor_slug: String,
    investor_avatar: Option<String>,
    op_slug: Option<String>,
    summary: Option<String>,
}

async fn get_ticker(
    State(state): State<AppState>,
    Query(params): Query<TickerQuery>,
) -> Result<Json<Vec<TickerEvent>>, ApiError> {
    let pool = &state.pool;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);

    let rows = if let Some(before_id) = params.before_id {
        sqlx::query_as::<_, (i64, i64, i64, chrono::DateTime<chrono::Utc>, String, Option<String>, Option<String>, Option<String>)>(
            r#"
            SELECT
                re.id, re.post_id, re.reblog_count_at_time, re.reblogged_at,
                b.slug, b.avatar_url,
                op.slug, p.summary
            FROM reblog_events re
            JOIN blogs b ON b.uuid = re.blog_uuid
            LEFT JOIN posts p ON p.post_id = re.post_id
            LEFT JOIN blogs op ON op.uuid = p.blog_uuid
            WHERE NOT re.is_op_position AND re.id < $1
            ORDER BY re.id DESC
            LIMIT $2
            "#,
        )
        .bind(before_id)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, (i64, i64, i64, chrono::DateTime<chrono::Utc>, String, Option<String>, Option<String>, Option<String>)>(
            r#"
            SELECT
                re.id, re.post_id, re.reblog_count_at_time, re.reblogged_at,
                b.slug, b.avatar_url,
                op.slug, p.summary
            FROM reblog_events re
            JOIN blogs b ON b.uuid = re.blog_uuid
            LEFT JOIN posts p ON p.post_id = re.post_id
            LEFT JOIN blogs op ON op.uuid = p.blog_uuid
            WHERE NOT re.is_op_position
            ORDER BY re.id DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    let events: Vec<TickerEvent> = rows
        .into_iter()
        .map(|r| TickerEvent {
            id: r.0,
            post_id: r.1.to_string(),
            buy_price: r.2,
            reblogged_at: r.3,
            investor_slug: r.4,
            investor_avatar: r.5,
            op_slug: r.6,
            summary: r.7,
        })
        .collect();

    Ok(Json(events))
}
