use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

// ── Blogs ──────────────────────────────────────────────────────────

/// Upsert a blog, updating slug and metadata if it already exists.
pub async fn upsert_blog(
    pool: &PgPool,
    uuid: &str,
    slug: &str,
    title: Option<&str>,
    description: Option<&str>,
    follower_count: Option<i64>,
    avatar_url: Option<&str>,
    blog_url: Option<&str>,
    is_nsfw: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO blogs (uuid, slug, title, description, follower_count, avatar_url, blog_url, is_nsfw)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (uuid) DO UPDATE SET
            slug = EXCLUDED.slug,
            title = EXCLUDED.title,
            description = EXCLUDED.description,
            follower_count = COALESCE(EXCLUDED.follower_count, blogs.follower_count),
            avatar_url = COALESCE(EXCLUDED.avatar_url, blogs.avatar_url),
            blog_url = COALESCE(EXCLUDED.blog_url, blogs.blog_url),
            is_nsfw = EXCLUDED.is_nsfw,
            updated_at = now()
        "#,
    )
    .bind(uuid)
    .bind(slug)
    .bind(title)
    .bind(description)
    .bind(follower_count)
    .bind(avatar_url)
    .bind(blog_url)
    .bind(is_nsfw)
    .execute(pool)
    .await?;

    // Update name cache
    sqlx::query(
        r#"
        INSERT INTO blog_name_cache (slug, blog_uuid, cached_at)
        VALUES ($1, $2, now())
        ON CONFLICT (slug) DO UPDATE SET
            blog_uuid = EXCLUDED.blog_uuid,
            cached_at = now()
        "#,
    )
    .bind(slug)
    .bind(uuid)
    .execute(pool)
    .await?;

    Ok(())
}

/// Look up a blog UUID by slug from the cache.
pub async fn resolve_slug_to_uuid(pool: &PgPool, slug: &str) -> Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT blog_uuid FROM blog_name_cache WHERE slug = $1")
            .bind(slug)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| r.0))
}

// ── Posts ───────────────────────────────────────────────────────────

/// Upsert a post. Updates note_count and metadata on conflict.
pub async fn upsert_post(
    pool: &PgPool,
    post_id: i64,
    blog_uuid: &str,
    root_post_id: Option<i64>,
    parent_post_id: Option<i64>,
    parent_blog_uuid: Option<&str>,
    root_blog_uuid: Option<&str>,
    created_at: DateTime<Utc>,
    note_count: i64,
    tags: &[String],
    summary: Option<&str>,
    content_text: Option<&str>,
    content_media: &[String],
    trail_npf: Option<&serde_json::Value>,
    post_url: Option<&str>,
    short_url: Option<&str>,
    interactability: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO posts (
            post_id, blog_uuid, root_post_id, parent_post_id,
            parent_blog_uuid, root_blog_uuid, created_at, note_count,
            tags, summary, content_text, content_media, trail_npf,
            post_url, short_url, interactability
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        ON CONFLICT (post_id) DO UPDATE SET
            note_count = EXCLUDED.note_count,
            tags = EXCLUDED.tags,
            summary = EXCLUDED.summary,
            content_text = COALESCE(EXCLUDED.content_text, posts.content_text),
            content_media = CASE
                WHEN array_length(EXCLUDED.content_media, 1) IS NOT NULL
                THEN EXCLUDED.content_media
                ELSE posts.content_media
            END,
            trail_npf = COALESCE(EXCLUDED.trail_npf, posts.trail_npf),
            post_url = COALESCE(EXCLUDED.post_url, posts.post_url),
            short_url = COALESCE(EXCLUDED.short_url, posts.short_url),
            interactability = COALESCE(EXCLUDED.interactability, posts.interactability),
            updated_in_db_at = now()
        "#,
    )
    .bind(post_id)
    .bind(blog_uuid)
    .bind(root_post_id)
    .bind(parent_post_id)
    .bind(parent_blog_uuid)
    .bind(root_blog_uuid)
    .bind(created_at)
    .bind(note_count)
    .bind(tags)
    .bind(summary)
    .bind(content_text)
    .bind(content_media)
    .bind(trail_npf)
    .bind(post_url)
    .bind(short_url)
    .bind(interactability)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update notes enumeration progress on a post.
pub async fn update_notes_progress(
    pool: &PgPool,
    post_id: i64,
    enumerated_through: DateTime<Utc>,
    fully_enumerated: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE posts SET
            notes_enumerated_through = $2,
            notes_fully_enumerated = $3,
            last_polled_at = now(),
            updated_in_db_at = now()
        WHERE post_id = $1
        "#,
    )
    .bind(post_id)
    .bind(enumerated_through)
    .bind(fully_enumerated)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Reblog Events ──────────────────────────────────────────────────

/// Insert a reblog event (ignores duplicates via the unique index).
pub async fn insert_reblog_event(
    pool: &PgPool,
    post_id: i64,
    blog_uuid: &str,
    reblog_post_id: Option<i64>,
    reblog_count_at_time: i64,
    reblogged_at: DateTime<Utc>,
    parent_blog_name: Option<&str>,
    parent_blog_uuid: Option<&str>,
    is_op_position: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO reblog_events (
            post_id, blog_uuid, reblog_post_id, reblog_count_at_time,
            reblogged_at, parent_blog_name, parent_blog_uuid, is_op_position
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (post_id, blog_uuid, reblogged_at)
            WHERE is_op_position = FALSE
            DO NOTHING
        "#,
    )
    .bind(post_id)
    .bind(blog_uuid)
    .bind(reblog_post_id)
    .bind(reblog_count_at_time)
    .bind(reblogged_at)
    .bind(parent_blog_name)
    .bind(parent_blog_uuid)
    .bind(is_op_position)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Likes ──────────────────────────────────────────────────────────

/// Insert a like event (ignores duplicates).
pub async fn insert_like(
    pool: &PgPool,
    post_id: i64,
    blog_uuid: &str,
    liked_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO likes (post_id, blog_uuid, liked_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (post_id, blog_uuid, liked_at) DO NOTHING
        "#,
    )
    .bind(post_id)
    .bind(blog_uuid)
    .bind(liked_at)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Snapshots ──────────────────────────────────────────────────────

/// Record a reblog count snapshot for a post.
pub async fn insert_snapshot(
    pool: &PgPool,
    post_id: i64,
    reblog_count: i64,
    snapshotted_at: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO reblog_count_snapshots (post_id, reblog_count, snapshotted_at)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(post_id)
    .bind(reblog_count)
    .bind(snapshotted_at)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Crawl Queue ────────────────────────────────────────────────────

/// Add a blog to the crawl queue if it's not already there.
pub async fn enqueue_blog(pool: &PgPool, blog_uuid: &str, priority: i32) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO crawl_queue (blog_uuid, priority)
        VALUES ($1, $2)
        ON CONFLICT (blog_uuid) DO NOTHING
        "#,
    )
    .bind(blog_uuid)
    .bind(priority)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get the next blog to crawl (in_progress first for resume, then pending).
pub async fn next_crawl_target(pool: &PgPool) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT blog_uuid FROM crawl_queue
        WHERE status IN ('pending', 'in_progress')
        ORDER BY
            CASE WHEN status = 'in_progress' THEN 0 ELSE 1 END,
            priority DESC,
            created_at ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

/// Get the posts_fetched count for resume.
pub async fn get_crawl_progress(pool: &PgPool, blog_uuid: &str) -> Result<i32> {
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT posts_fetched FROM crawl_queue WHERE blog_uuid = $1",
    )
    .bind(blog_uuid)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

/// Update crawl queue status for a blog.
pub async fn update_crawl_status(
    pool: &PgPool,
    blog_uuid: &str,
    status: &str,
    posts_fetched: i32,
    total_posts: Option<i32>,
    last_error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE crawl_queue SET
            status = $2,
            posts_fetched = $3,
            total_posts = $4,
            last_error = $5,
            started_at = CASE WHEN $2 = 'in_progress' AND started_at IS NULL THEN now() ELSE started_at END,
            completed_at = CASE WHEN $2 IN ('completed', 'failed') THEN now() ELSE completed_at END,
            updated_at = now()
        WHERE blog_uuid = $1
        "#,
    )
    .bind(blog_uuid)
    .bind(status)
    .bind(posts_fetched)
    .bind(total_posts)
    .bind(last_error)
    .execute(pool)
    .await?;
    Ok(())
}
