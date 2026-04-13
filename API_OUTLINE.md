# Notesdaq API Outline

> Reference for building `notesdaq-api` (Axum). All endpoints return JSON.
> Prefix: `/api/v1`

---

## 1. Blog / Portfolio

The killer feature: look up any blog and see their "investment portfolio."

### `GET /blog/{slug}`

Blog profile + portfolio summary.

```json
{
  "blog": {
    "uuid": "t:U7YuPu8OZt1EeYB9X2Asgg",
    "slug": "wasabipesto",
    "title": "...",
    "avatar_url": "...",
    "follower_count": 1234,
    "blog_url": "https://wasabipesto.tumblr.com"
  },
  "portfolio": {
    "total_positions": 847,
    "original_posts": 42,
    "avg_multiplier": 3.14,
    "best_pick": { "post_id": 766510142173954048, "multiplier": 6.56, "summary": "..." },
    "worst_pick": { "post_id": 123456789, "multiplier": 0.8, "summary": "..." },
    "total_invested": 55000,
    "total_current_value": 172700,
    "crawl_status": "completed"
  }
}
```

**Derived from:**
- `blogs` table for profile
- `reblog_events` WHERE blog_uuid = X, joined with latest snapshot per post
- `crawl_queue` for crawl status

### `GET /blog/{slug}/positions`

Paginated list of all positions (reblogs as investments).

**Query params:** `?sort=multiplier|buy_price|date|current_price&order=asc|desc&page=1&per_page=50`

```json
{
  "positions": [
    {
      "post_id": 766510142173954048,
      "buy_price": 222,
      "current_price": 1457,
      "multiplier": 6.56,
      "reblogged_at": "2024-12-12T13:23:07Z",
      "is_op": false,
      "post": {
        "summary": "...",
        "post_url": "https://...",
        "op_blog": { "slug": "weaver-z", "avatar_url": "..." },
        "tags": ["tag1", "tag2"],
        "note_count": 5230
      }
    }
  ],
  "pagination": { "page": 1, "per_page": 50, "total": 847 }
}
```

**Derived from:**
- `reblog_events` for buy_price, reblogged_at
- `reblog_count_snapshots` (latest) or `max(reblog_count_at_time)` for current_price
- `posts` for summary, URL, tags
- `blogs` for OP info

### `GET /blog/{slug}/chart`

Portfolio value over time (for sparkline or chart on profile page).

**Query params:** `?period=7d|30d|90d|all`

```json
{
  "points": [
    { "t": "2024-12-12T00:00:00Z", "value": 222 },
    { "t": "2024-12-13T00:00:00Z", "value": 245 }
  ]
}
```

**Derived from:** `reblog_count_snapshots` for each held post, summed at each snapshot time.
This is expensive — may want to materialize or cache.

---

## 2. Post / Stock

A single post viewed as a "stock" — its price history and investor list.

### `GET /post/{post_id}`

Post detail + stock summary.

```json
{
  "post": {
    "post_id": 766510142173954048,
    "summary": "...",
    "content_text": "...",
    "content_media": ["https://..."],
    "post_url": "https://...",
    "tags": [],
    "created_at": "2024-11-11T16:00:00Z",
    "note_count": 5230
  },
  "stock": {
    "current_price": 1457,
    "total_investors": 1457,
    "total_likes": 2100,
    "ipo_date": "2024-11-11T16:00:00Z",
    "op": { "slug": "weaver-z", "avatar_url": "..." },
    "fully_enumerated": true
  }
}
```

### `GET /post/{post_id}/investors`

Paginated investor list for a post.

**Query params:** `?sort=price|date&order=asc|desc&page=1&per_page=50`

```json
{
  "investors": [
    {
      "buy_price": 1,
      "reblogged_at": "2024-11-11T17:01:55Z",
      "is_op": false,
      "blog": { "slug": "pananananananananananananananana", "avatar_url": "..." },
      "multiplier": 1457.0
    }
  ],
  "pagination": { "page": 1, "per_page": 50, "total": 1458 }
}
```

### `GET /post/{post_id}/chart`

Price (reblog count) over time for the stock chart.

**Query params:** `?period=7d|30d|90d|all`

```json
{
  "points": [
    { "t": "2024-11-11T17:01:55Z", "price": 1 },
    { "t": "2024-11-12T00:00:00Z", "price": 9 }
  ]
}
```

**Derived from:** `reblog_count_snapshots` for this post. If we want finer resolution,
we can also derive points from `reblog_events` timestamps (each event IS a price point).

---

## 3. Leaderboards

### `GET /leaderboard/blogs`

Top blogs by portfolio performance.

**Query params:** `?sort=avg_multiplier|total_value|best_pick|positions&page=1&per_page=50`

```json
{
  "entries": [
    {
      "rank": 1,
      "blog": { "slug": "wasabipesto", "avatar_url": "..." },
      "avg_multiplier": 3.14,
      "total_positions": 847,
      "best_multiplier": 6.56,
      "total_current_value": 172700
    }
  ],
  "pagination": { ... }
}
```

**Note:** Only includes blogs we've fully crawled. Partial data is excluded or flagged.

### `GET /leaderboard/posts`

Top posts (stocks) by various metrics.

**Query params:** `?sort=price|growth|investors|recent_growth&period=7d|30d|all&page=1&per_page=50`

```json
{
  "entries": [
    {
      "rank": 1,
      "post_id": 766510142173954048,
      "current_price": 1457,
      "total_investors": 1457,
      "summary": "...",
      "op": { "slug": "weaver-z", "avatar_url": "..." },
      "growth_pct": 145.0
    }
  ],
  "pagination": { ... }
}
```

---

## 4. Search

### `GET /search`

Unified search across blogs and posts.

**Query params:** `?q=wasabipesto&type=blog|post|all&page=1&per_page=20`

```json
{
  "blogs": [
    { "slug": "wasabipesto", "avatar_url": "...", "total_positions": 847 }
  ],
  "posts": [
    { "post_id": 123, "summary": "matching text...", "current_price": 45 }
  ]
}
```

**Derived from:**
- Blog search: `blogs` table, `ILIKE` on slug/title/description
- Post search: `posts` table, full-text search on `content_text` and `tags`
- Consider `pg_trgm` extension for fuzzy blog name matching

---

## 5. Ticker / Activity Feed

### `GET /ticker`

Recent reblog events across all tracked posts — the "stock ticker."

**Query params:** `?limit=50&before_id=12345`

```json
{
  "events": [
    {
      "id": 98765,
      "post_id": 766510142173954048,
      "buy_price": 1400,
      "investor": { "slug": "someone", "avatar_url": "..." },
      "op": { "slug": "weaver-z" },
      "reblogged_at": "2026-02-26T23:01:29Z",
      "summary": "..."
    }
  ]
}
```

**Derived from:** `reblog_events` ordered by `reblogged_at DESC`, joined with blogs and posts.
Cursor-based pagination via `before_id`.

---

## 6. System / Meta

### `GET /stats`

Overall system statistics for a dashboard footer or about page.

```json
{
  "total_blogs": 7234,
  "blogs_crawled": 1,
  "total_posts": 18429,
  "total_reblog_events": 145000,
  "total_likes": 280000,
  "posts_enumerated": 4500,
  "crawl_in_progress": "wasabipesto"
}
```

### `GET /health`

Liveness check.

```json
{ "status": "ok", "db": true }
```

---

## 7. OG Images (future)

### `GET /og/blog/{slug}.png`

Generated image for social sharing — portfolio card with blog name, avatar, key stats.
Rendered server-side with `resvg` + an SVG template.

### `GET /og/post/{post_id}.png`

Stock card — post summary, price chart sparkline, OP info.

---

## Implementation Notes

### Axum structure

```
notesdaq-api/
├── src/
│   ├── main.rs          — server setup, router, state
│   ├── routes/
│   │   ├── blog.rs      — /blog/{slug}, /blog/{slug}/positions, /blog/{slug}/chart
│   │   ├── post.rs      — /post/{post_id}, /post/{post_id}/investors, /post/{post_id}/chart
│   │   ├── leaderboard.rs
│   │   ├── search.rs
│   │   ├── ticker.rs
│   │   └── meta.rs      — /stats, /health
│   ├── models.rs        — API response structs (separate from DB models)
│   └── queries.rs       — read-only query functions (or reuse notesdaq-db)
```

### Key queries that need indexes

Already have:
- `idx_reblog_events_blog_uuid` — portfolio lookup
- `idx_reblog_events_post_id` — stock/investor lookup
- `idx_snapshots_post_id` — price history

May need to add:
- `idx_reblog_events_reblogged_at` — ticker feed (already exists)
- Full-text index on `posts.content_text` — search
- `pg_trgm` index on `blogs.slug` — fuzzy blog search
- Materialized view or cache for leaderboard aggregations (expensive)

### Multiplier calculation

Done in application code, not SQL:
```rust
fn multiplier(buy_price: i64, current_price: i64) -> f64 {
    if buy_price == 0 { f64::INFINITY }
    else { current_price as f64 / buy_price as f64 }
}
```

### CORS

Allow all origins (public data, no auth). Set via `tower-http` CorsLayer.

### Caching

- Blog profiles and portfolio summaries: short TTL (5 min) since data changes as crawl progresses
- Post stock data: same
- Leaderboards: longer TTL (15-30 min), expensive to compute
- OG images: long TTL (1 hour+), cache on disk or CDN

### What the frontend needs that the API doesn't compute

- Formatting: multiplier as "6.56x", prices with commas, relative timestamps
- Sorting/filtering UI state
- Chart rendering (API provides data points, frontend draws)
- Inf display for OP positions
