use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/blog/{slug}", get(get_blog))
        .route("/blog/{slug}/positions", get(get_positions))
        .route("/blog/{slug}/chart", get(get_portfolio_chart))
        .route("/blog/{slug}/scatter", get(get_scatter))
}

// ── Types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct BlogProfile {
    uuid: String,
    slug: String,
    title: Option<String>,
    description: Option<String>,
    avatar_url: Option<String>,
    follower_count: Option<i64>,
    blog_url: Option<String>,
}

#[derive(Serialize)]
struct PortfolioSummary {
    total_positions: i64,
    original_posts: i64,
    avg_multiplier: Option<f64>,
    best_pick: Option<PickSummary>,
    total_invested: i64,
    total_current_value: i64,
    crawl_status: Option<String>,
}

#[derive(Serialize)]
struct PickSummary {
    post_id: String,
    multiplier: f64,
    buy_price: i64,
    current_price: i64,
    summary: Option<String>,
}

#[derive(Serialize)]
struct BlogResponse {
    blog: BlogProfile,
    portfolio: PortfolioSummary,
}

#[derive(Serialize)]
struct Position {
    post_id: String,
    buy_price: i64,
    current_price: i64,
    multiplier: f64,
    reblogged_at: chrono::DateTime<chrono::Utc>,
    is_op: bool,
    summary: Option<String>,
    post_url: Option<String>,
    op_slug: Option<String>,
    op_avatar: Option<String>,
    tags: Vec<String>,
    note_count: i64,
}

#[derive(Serialize)]
struct Pagination {
    page: i64,
    per_page: i64,
    total: i64,
}

#[derive(Serialize)]
struct PositionsResponse {
    positions: Vec<Position>,
    pagination: Pagination,
}

#[derive(Deserialize)]
pub struct PositionsQuery {
    sort: Option<String>,
    order: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
}

#[derive(Serialize, Clone)]
struct ChartPoint {
    t: chrono::DateTime<chrono::Utc>,
    value: i64,
}

#[derive(Serialize)]
struct ScatterPoint {
    post_id: String,
    buy_price: i64,
    current_price: i64,
    reblogged_at: chrono::DateTime<chrono::Utc>,
    summary: Option<String>,
}

// ── Handlers ──────────────────────────────────────────────────────

async fn get_blog(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<BlogResponse>, ApiError> {
    let pool = &state.pool;

    let blog_uuid = resolve_slug(pool, &slug).await?;

    let blog = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<i64>, Option<String>)>(
        "SELECT uuid, slug, title, description, avatar_url, follower_count, blog_url FROM blogs WHERE uuid = $1"
    )
    .bind(&blog_uuid)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::NotFound("Blog not found".into()))?;

    let profile = BlogProfile {
        uuid: blog.0,
        slug: blog.1,
        title: blog.2,
        description: blog.3,
        avatar_url: blog.4,
        follower_count: blog.5,
        blog_url: blog.6,
    };

    // Portfolio summary: join reblog_events with latest snapshot per post
    let summary_row = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            count(*) FILTER (WHERE NOT is_op_position) as positions,
            count(*) FILTER (WHERE is_op_position) as originals
        FROM reblog_events
        WHERE blog_uuid = $1
        "#,
    )
    .bind(&blog_uuid)
    .fetch_one(pool)
    .await?;

    let total_positions = summary_row.0;
    let original_posts = summary_row.1;

    // Get aggregated portfolio value stats.
    // Use snapshots when available, fall back to max(reblog_count_at_time) per post.
    let value_stats = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<f64>)>(
        r#"
        WITH current_prices AS (
            SELECT DISTINCT ON (re.post_id)
                re.post_id,
                re.reblog_count_at_time as buy_price,
                COALESCE(
                    (SELECT reblog_count FROM reblog_count_snapshots
                     WHERE post_id = re.post_id ORDER BY snapshotted_at DESC LIMIT 1),
                    (SELECT max(reblog_count_at_time) FROM reblog_events WHERE post_id = re.post_id)
                ) as current_price
            FROM reblog_events re
            WHERE re.blog_uuid = $1 AND NOT re.is_op_position
        )
        SELECT
            sum(buy_price)::bigint,
            sum(current_price)::bigint,
            avg(CASE WHEN buy_price > 0
                THEN current_price::float8 / buy_price::float8
                END)
        FROM current_prices
        "#,
    )
    .bind(&blog_uuid)
    .fetch_one(pool)
    .await?;

    // Best pick
    let best = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
        r#"
        WITH picks AS (
            SELECT re.post_id, re.reblog_count_at_time as buy_price,
                COALESCE(
                    (SELECT reblog_count FROM reblog_count_snapshots
                     WHERE post_id = re.post_id ORDER BY snapshotted_at DESC LIMIT 1),
                    (SELECT max(reblog_count_at_time) FROM reblog_events WHERE post_id = re.post_id)
                ) as current_price,
                p.summary
            FROM reblog_events re
            LEFT JOIN posts p ON p.post_id = re.post_id
            WHERE re.blog_uuid = $1 AND NOT re.is_op_position AND re.reblog_count_at_time > 0
        )
        SELECT post_id, buy_price, current_price, summary
        FROM picks
        ORDER BY current_price::float8 / buy_price::float8 DESC
        LIMIT 1
        "#,
    )
    .bind(&blog_uuid)
    .fetch_optional(pool)
    .await?;

    let best_pick = best.map(|b| PickSummary {
        post_id: b.0.to_string(),
        multiplier: b.2 as f64 / b.1 as f64,
        buy_price: b.1,
        current_price: b.2,
        summary: b.3,
    });

    // Crawl status
    let crawl_status: Option<(String,)> =
        sqlx::query_as("SELECT status FROM crawl_queue WHERE blog_uuid = $1")
            .bind(&blog_uuid)
            .fetch_optional(pool)
            .await?;

    let portfolio = PortfolioSummary {
        total_positions,
        original_posts,
        avg_multiplier: value_stats.2,
        best_pick,
        total_invested: value_stats.0.unwrap_or(0),
        total_current_value: value_stats.1.unwrap_or(0),
        crawl_status: crawl_status.map(|r| r.0),
    };

    Ok(Json(BlogResponse {
        blog: profile,
        portfolio,
    }))
}

async fn get_positions(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<PositionsQuery>,
) -> Result<Json<PositionsResponse>, ApiError> {
    let pool = &state.pool;
    let blog_uuid = resolve_slug(pool, &slug).await?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).clamp(1, 100);
    let offset = (page - 1) * per_page;

    let sort_col = match params.sort.as_deref() {
        Some("multiplier") => "multiplier",
        Some("buy_price") => "re.reblog_count_at_time",
        Some("current_price") => "current_price",
        Some("date") => "re.reblogged_at",
        _ => "re.reblogged_at",
    };
    let order = match params.order.as_deref() {
        Some("asc") => "ASC",
        _ => "DESC",
    };

    // Total count
    let (total,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM reblog_events WHERE blog_uuid = $1",
    )
    .bind(&blog_uuid)
    .fetch_one(pool)
    .await?;

    // Using a dynamic query since sort column can't be parameterized
    let query = format!(
        r#"
        SELECT
            re.post_id,
            re.reblog_count_at_time as buy_price,
            COALESCE(s.reblog_count, mx.max_price, 0) as current_price,
            CASE WHEN re.reblog_count_at_time > 0
                THEN COALESCE(s.reblog_count, mx.max_price, 0)::float8 / re.reblog_count_at_time::float8
                ELSE 'Infinity'::float8
            END as multiplier,
            re.reblogged_at,
            re.is_op_position,
            p.summary,
            p.post_url,
            op.slug as op_slug,
            op.avatar_url as op_avatar,
            p.tags,
            p.note_count
        FROM reblog_events re
        LEFT JOIN LATERAL (
            SELECT reblog_count FROM reblog_count_snapshots
            WHERE post_id = re.post_id
            ORDER BY snapshotted_at DESC LIMIT 1
        ) s ON true
        LEFT JOIN LATERAL (
            SELECT max(reblog_count_at_time) as max_price FROM reblog_events
            WHERE post_id = re.post_id
        ) mx ON true
        LEFT JOIN posts p ON p.post_id = re.post_id
        LEFT JOIN blogs op ON op.uuid = p.blog_uuid
        WHERE re.blog_uuid = $1
        ORDER BY {sort_col} {order}
        LIMIT $2 OFFSET $3
        "#,
    );

    let rows = sqlx::query_as::<_, (i64, i64, Option<i64>, Option<f64>, chrono::DateTime<chrono::Utc>, bool, Option<String>, Option<String>, Option<String>, Option<String>, Vec<String>, i64)>(
        &query,
    )
    .bind(&blog_uuid)
    .bind(per_page)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let positions: Vec<Position> = rows
        .into_iter()
        .map(|r| Position {
            post_id: r.0.to_string(),
            buy_price: r.1,
            current_price: r.2.unwrap_or(0),
            multiplier: r.3.unwrap_or(0.0),
            reblogged_at: r.4,
            is_op: r.5,
            summary: r.6,
            post_url: r.7,
            op_slug: r.8,
            op_avatar: r.9,
            tags: r.10,
            note_count: r.11,
        })
        .collect();

    Ok(Json(PositionsResponse {
        positions,
        pagination: Pagination {
            page,
            per_page,
            total,
        },
    }))
}

async fn get_portfolio_chart(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<ChartPoint>>, ApiError> {
    let pool = &state.pool;
    let blog_uuid = resolve_slug(pool, &slug).await?;

    // Portfolio P&L over time using LOCF (last observation carried forward).
    // For each held post, P&L = current_price - buy_price. We track current_price
    // via reblog events and snapshots, and subtract the buy price (this blog's entry).
    //
    // Step 1: get this blog's buy prices (their reblog event per post)
    let buy_prices = sqlx::query_as::<_, (i64, i64, chrono::DateTime<chrono::Utc>)>(
        r#"
        SELECT post_id, reblog_count_at_time, reblogged_at
        FROM reblog_events
        WHERE blog_uuid = $1
        "#,
    )
    .bind(&blog_uuid)
    .fetch_all(pool)
    .await?;

    use std::collections::HashMap;
    let buy_price_map: HashMap<i64, i64> = buy_prices.iter().map(|r| (r.0, r.1)).collect();
    let buy_time_map: HashMap<i64, chrono::DateTime<chrono::Utc>> = buy_prices.iter().map(|r| (r.0, r.2)).collect();

    // Step 2: get the full price timeline for all held posts
    let events = sqlx::query_as::<_, (i64, chrono::DateTime<chrono::Utc>, i64)>(
        r#"
        WITH held_posts AS (
            SELECT DISTINCT post_id FROM reblog_events WHERE blog_uuid = $1
        )
        SELECT post_id, ts, price FROM (
            SELECT post_id, reblogged_at AS ts, reblog_count_at_time AS price
            FROM reblog_events
            WHERE post_id IN (SELECT post_id FROM held_posts)
            UNION ALL
            SELECT post_id, snapshotted_at AS ts, reblog_count AS price
            FROM reblog_count_snapshots
            WHERE post_id IN (SELECT post_id FROM held_posts)
        ) combined
        ORDER BY ts
        "#,
    )
    .bind(&blog_uuid)
    .fetch_all(pool)
    .await?;

    // Step 3: LOCF — track latest price per post, but only count a post's P&L
    // after the blog actually acquired it (reblogged_at).
    let mut latest_price: HashMap<i64, i64> = HashMap::new();
    let mut chart: Vec<ChartPoint> = Vec::new();
    let mut last_ts: Option<chrono::DateTime<chrono::Utc>> = None;

    for (post_id, ts, price) in &events {
        latest_price.insert(*post_id, *price);

        if last_ts.map_or(true, |prev| prev != *ts) {
            // Sum P&L for all positions acquired on or before this timestamp
            let total_pnl: i64 = latest_price.iter()
                .filter(|(pid, _)| buy_time_map.get(pid).map_or(false, |bt| bt <= ts))
                .map(|(pid, cur)| cur - buy_price_map.get(pid).unwrap_or(&0))
                .sum();
            chart.push(ChartPoint { t: *ts, value: total_pnl });
            last_ts = Some(*ts);
        } else if let Some(last) = chart.last_mut() {
            let total_pnl: i64 = latest_price.iter()
                .filter(|(pid, _)| buy_time_map.get(pid).map_or(false, |bt| bt <= ts))
                .map(|(pid, cur)| cur - buy_price_map.get(pid).unwrap_or(&0))
                .sum();
            last.value = total_pnl;
        }
    }

    // Downsample if too many points (keep ~500 for chart readability)
    if chart.len() > 500 {
        let step = chart.len() / 500;
        let mut sampled: Vec<ChartPoint> = chart.iter().step_by(step).cloned().collect();
        if let Some(last) = chart.last() {
            if sampled.last().map_or(true, |s| s.t != last.t) {
                sampled.push(last.clone());
            }
        }
        chart = sampled;
    }

    Ok(Json(chart))
}

async fn get_scatter(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Vec<ScatterPoint>>, ApiError> {
    let pool = &state.pool;
    let blog_uuid = resolve_slug(pool, &slug).await?;

    let rows = sqlx::query_as::<_, (i64, i64, Option<i64>, chrono::DateTime<chrono::Utc>, Option<String>)>(
        r#"
        SELECT
            re.post_id,
            re.reblog_count_at_time,
            COALESCE(
                (SELECT reblog_count FROM reblog_count_snapshots
                 WHERE post_id = re.post_id ORDER BY snapshotted_at DESC LIMIT 1),
                (SELECT max(reblog_count_at_time) FROM reblog_events WHERE post_id = re.post_id)
            ),
            re.reblogged_at,
            p.summary
        FROM reblog_events re
        LEFT JOIN posts p ON p.post_id = re.post_id
        WHERE re.blog_uuid = $1 AND NOT re.is_op_position
        "#,
    )
    .bind(&blog_uuid)
    .fetch_all(pool)
    .await?;

    let points: Vec<ScatterPoint> = rows
        .into_iter()
        .map(|r| ScatterPoint {
            post_id: r.0.to_string(),
            buy_price: r.1,
            current_price: r.2.unwrap_or(0),
            reblogged_at: r.3,
            summary: r.4,
        })
        .collect();

    Ok(Json(points))
}

// ── Helpers ───────────────────────────────────────────────────────

async fn resolve_slug(pool: &PgPool, slug: &str) -> Result<String, ApiError> {
    // Try name cache first
    let cached: Option<(String,)> =
        sqlx::query_as("SELECT blog_uuid FROM blog_name_cache WHERE slug = $1")
            .bind(slug)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    if let Some((uuid,)) = cached {
        return Ok(uuid);
    }

    // Fall back to blogs table
    let row: Option<(String,)> =
        sqlx::query_as("SELECT uuid FROM blogs WHERE slug = $1")
            .bind(slug)
            .fetch_optional(pool)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

    row.map(|r| r.0)
        .ok_or_else(|| ApiError::NotFound(format!("Blog '{slug}' not found")))
}

// ── Error type ────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    Internal(String),
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ApiError::NotFound(m) => (axum::http::StatusCode::NOT_FOUND, m),
            ApiError::Internal(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}
