mod tumblr;

use anyhow::{Context, Result};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "notesdaq_fetch=info".parse().unwrap()),
        )
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or("postgres://notesdaq:notesdaq@localhost/notesdaq".into());

    let consumer_key = std::env::var("TUMBLR_CONSUMER_KEY").context("TUMBLR_CONSUMER_KEY not set")?;
    let consumer_secret =
        std::env::var("TUMBLR_CONSUMER_SECRET").context("TUMBLR_CONSUMER_SECRET not set")?;
    let token = std::env::var("TUMBLR_TOKEN").context("TUMBLR_TOKEN not set")?;
    let token_secret = std::env::var("TUMBLR_TOKEN_SECRET").context("TUMBLR_TOKEN_SECRET not set")?;

    let seed_blog = std::env::var("SEED_BLOG").unwrap_or("wasabipesto".into());
    let raw_dir = std::env::var("RAW_RESPONSE_DIR").unwrap_or("raw_responses".into());

    info!("Connecting to database: {}", database_url.split('@').last().unwrap_or(&database_url));
    let pool = notesdaq_db::connect(&database_url).await?;

    info!("Running migrations...");
    notesdaq_db::run_migrations(&pool).await?;
    info!("Migrations complete.");

    let client = tumblr::TumblrClient::new(
        consumer_key,
        consumer_secret,
        token,
        token_secret,
        raw_dir.into(),
    )?;

    // Seed the crawl queue
    info!("Seeding crawl with blog: {seed_blog}");
    let blog_info = client.get_blog_info(&seed_blog).await?;
    let blog_uuid = blog_info["uuid"].as_str().context("No UUID in blog info")?;

    notesdaq_db::queries::upsert_blog(
        &pool,
        blog_uuid,
        blog_info["name"].as_str().unwrap_or(&seed_blog),
        blog_info["title"].as_str(),
        blog_info["description"].as_str(),
        blog_info["followers"].as_i64(),
        blog_info["avatar"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v["url"].as_str()),
        blog_info["url"].as_str(),
        blog_info["is_nsfw"].as_bool().unwrap_or(false),
    )
    .await?;

    notesdaq_db::queries::enqueue_blog(&pool, blog_uuid, 100).await?;

    // Main crawl loop
    loop {
        let Some(target_uuid) = notesdaq_db::queries::next_crawl_target(&pool).await? else {
            info!("No more blogs in crawl queue. Done.");
            break;
        };

        // Look up the blog slug for API calls
        let blog_slug: Option<String> = sqlx::query_scalar("SELECT slug FROM blogs WHERE uuid = $1")
            .bind(&target_uuid)
            .fetch_optional(&pool)
            .await?;

        let Some(blog_slug) = blog_slug else {
            error!("Blog {target_uuid} in crawl queue but not in blogs table");
            notesdaq_db::queries::update_crawl_status(
                &pool, &target_uuid, "failed", 0, None, Some("Blog not in blogs table"),
            )
            .await?;
            continue;
        };

        info!("Crawling blog: {blog_slug} ({target_uuid})");
        notesdaq_db::queries::update_crawl_status(&pool, &target_uuid, "in_progress", 0, None, None)
            .await?;

        match crawl_blog(&pool, &client, &blog_slug, &target_uuid).await {
            Ok(posts_fetched) => {
                info!("Completed crawl of {blog_slug}: {posts_fetched} posts fetched");
                notesdaq_db::queries::update_crawl_status(
                    &pool,
                    &target_uuid,
                    "completed",
                    posts_fetched,
                    Some(posts_fetched),
                    None,
                )
                .await?;
            }
            Err(e) => {
                error!("Crawl of {blog_slug} failed: {e:#}");
                notesdaq_db::queries::update_crawl_status(
                    &pool,
                    &target_uuid,
                    "failed",
                    0,
                    None,
                    Some(&format!("{e:#}")),
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// Crawl a single blog: fetch all posts, then enumerate notes for each root post.
async fn crawl_blog(
    pool: &sqlx::PgPool,
    client: &tumblr::TumblrClient,
    blog_slug: &str,
    blog_uuid: &str,
) -> Result<i32> {
    // Phase 1: Fetch all posts for this blog (resume from last position)
    let resume_from = notesdaq_db::queries::get_crawl_progress(pool, blog_uuid).await?;
    info!("[{blog_slug}] Phase 1: Fetching posts (resuming from {resume_from})...");
    let mut offset = resume_from as u32;
    let mut posts_fetched: i32 = resume_from;
    let mut root_post_ids: Vec<i64> = Vec::new();

    // Collect root post IDs from already-fetched posts too
    let existing_roots: Vec<(Option<i64>, i64)> = sqlx::query_as(
        "SELECT root_post_id, post_id FROM posts WHERE blog_uuid = $1",
    )
    .bind(blog_uuid)
    .fetch_all(pool)
    .await?;
    for (root_id, post_id) in &existing_roots {
        let id = root_id.unwrap_or(*post_id);
        if !root_post_ids.contains(&id) {
            root_post_ids.push(id);
        }
    }

    loop {
        let data = client
            .get_blog_posts(blog_slug, offset, 20)
            .await
            .context("Failed to fetch posts")?;

        let posts = data["posts"]
            .as_array()
            .context("No posts array in response")?;

        if posts.is_empty() {
            break;
        }

        for post in posts {
            let post_id = post["id"].as_i64().context("No post ID")?;
            let post_blog_uuid = post["blog"]["uuid"]
                .as_str()
                .unwrap_or(blog_uuid);

            // Ensure the post's blog exists
            if post_blog_uuid != blog_uuid {
                ensure_blog_from_post(pool, post).await?;
            }

            // Extract root and parent info
            let root_post_id = post["reblogged_root_id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| post["reblogged_root_id"].as_i64());
            let parent_post_id = post["reblogged_from_id"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| post["reblogged_from_id"].as_i64());
            let parent_blog_uuid = post["reblogged_from_uuid"].as_str();
            let root_blog_uuid = post["reblogged_root_uuid"].as_str();

            // Extract content
            let (content_text, content_media) = extract_content(post);
            let trail_npf = post.get("trail").cloned();
            let tags: Vec<String> = post["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let timestamp = post["timestamp"].as_i64().unwrap_or(0);
            let created_at = chrono::DateTime::from_timestamp(timestamp, 0)
                .unwrap_or_else(chrono::Utc::now);

            // Ensure parent/root blog entries exist
            if let Some(uuid) = parent_blog_uuid {
                if let Some(name) = post["reblogged_from_name"].as_str() {
                    ensure_blog_minimal(pool, uuid, name).await?;
                }
            }
            if let Some(uuid) = root_blog_uuid {
                if let Some(name) = post["reblogged_root_name"].as_str() {
                    ensure_blog_minimal(pool, uuid, name).await?;
                }
            }

            notesdaq_db::queries::upsert_post(
                pool,
                post_id,
                post_blog_uuid,
                root_post_id,
                parent_post_id,
                parent_blog_uuid,
                root_blog_uuid,
                created_at,
                post["note_count"].as_i64().unwrap_or(0),
                &tags,
                post["summary"].as_str(),
                content_text.as_deref(),
                &content_media,
                trail_npf.as_ref(),
                post["post_url"].as_str(),
                post["short_url"].as_str(),
                post["interactability_reblog"].as_str(),
            )
            .await?;

            // Track root posts that need notes enumeration.
            // If this IS a root post (no root_post_id), it's a root itself.
            // If it's a reblog, the root is the one that needs enumeration.
            let root_id = root_post_id.unwrap_or(post_id);
            if !root_post_ids.contains(&root_id) {
                root_post_ids.push(root_id);
            }

            posts_fetched += 1;
        }

        info!("[{blog_slug}] Fetched {posts_fetched} posts (offset {offset})...");
        notesdaq_db::queries::update_crawl_status(
            pool,
            blog_uuid,
            "in_progress",
            posts_fetched,
            None,
            None,
        )
        .await?;

        offset += 20;
    }

    info!(
        "[{blog_slug}] Phase 1 complete: {posts_fetched} posts, {} unique root posts to enumerate",
        root_post_ids.len()
    );

    // Phase 2: Enumerate notes for each root post
    info!("[{blog_slug}] Phase 2: Enumerating notes for root posts...");
    let total_roots = root_post_ids.len();

    for (i, root_id) in root_post_ids.iter().enumerate() {
        // Check if already fully enumerated
        let already_done: Option<(bool,)> = sqlx::query_as(
            "SELECT notes_fully_enumerated FROM posts WHERE post_id = $1",
        )
        .bind(root_id)
        .fetch_optional(pool)
        .await?;

        if already_done.map(|r| r.0).unwrap_or(false) {
            info!("[{blog_slug}] Root post {root_id} already enumerated, skipping ({}/{total_roots})", i + 1);
            continue;
        }

        info!("[{blog_slug}] Enumerating notes for root post {root_id} ({}/{total_roots})...", i + 1);

        match enumerate_notes(pool, client, blog_slug, *root_id).await {
            Ok(count) => {
                info!("[{blog_slug}] Root post {root_id}: {count} reblog events recorded");
            }
            Err(e) => {
                warn!("[{blog_slug}] Failed to enumerate notes for {root_id}: {e:#}");
                // Continue with other posts — don't abort the whole crawl
            }
        }
    }

    Ok(posts_fetched)
}

/// Enumerate all notes for a root post, assign share prices, store reblog events and likes.
async fn enumerate_notes(
    pool: &sqlx::PgPool,
    client: &tumblr::TumblrClient,
    blog_slug: &str,
    root_post_id: i64,
) -> Result<i64> {
    // Check if we have a resume point
    let resume_through: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT notes_enumerated_through FROM posts WHERE post_id = $1",
    )
    .bind(root_post_id)
    .fetch_optional(pool)
    .await?;

    let mut before_timestamp: Option<i64> = resume_through
        .and_then(|r| r.0)
        .map(|dt| dt.timestamp());

    // Collect all reblog notes for this post
    let mut all_reblog_notes: Vec<serde_json::Value> = Vec::new();
    let mut pages = 0;

    loop {
        let data = client
            .get_notes(blog_slug, root_post_id, before_timestamp)
            .await;

        let data = match data {
            Ok(d) => d,
            Err(e) => {
                // If the post doesn't exist on this blog, it might need a different blog slug.
                // Try to find the root blog from our DB.
                if pages == 0 {
                    let root_slug = try_root_blog_slug(pool, root_post_id).await?;
                    if let Some(slug) = root_slug {
                        if slug != blog_slug {
                            info!("Retrying notes for {root_post_id} via root blog: {slug}");
                            return enumerate_notes_via(pool, client, &slug, root_post_id).await;
                        }
                    }
                }
                return Err(e.context("Failed to fetch notes"));
            }
        };

        let notes = data["notes"]
            .as_array()
            .context("No notes array")?;

        if notes.is_empty() {
            break;
        }

        // Process each note: store likes immediately, collect reblogs for batch processing
        let mut oldest_timestamp_in_page: Option<i64> = None;

        for note in notes {
            let note_type = note["type"].as_str().unwrap_or("unknown");
            let ts = note["timestamp"].as_i64().or_else(|| {
                note["timestamp"].as_f64().map(|f| f as i64)
            });

            if let Some(ts) = ts {
                oldest_timestamp_in_page = Some(
                    oldest_timestamp_in_page.map_or(ts, |old: i64| old.min(ts)),
                );
            }

            // Ensure blog exists for every note
            if let (Some(uuid), Some(name)) = (note["blog_uuid"].as_str(), note["blog_name"].as_str()) {
                ensure_blog_minimal(pool, uuid, name).await?;
            }

            match note_type {
                "reblog" => {
                    all_reblog_notes.push(note.clone());
                }
                "like" => {
                    if let (Some(blog_uuid), Some(ts)) = (note["blog_uuid"].as_str(), ts) {
                        let liked_at = chrono::DateTime::from_timestamp(ts, 0)
                            .unwrap_or_else(chrono::Utc::now);
                        // Ensure the root post exists (it might only exist via notes, not from a blog crawl)
                        ensure_root_post(pool, root_post_id).await?;
                        notesdaq_db::queries::insert_like(pool, root_post_id, blog_uuid, liked_at)
                            .await?;
                    }
                }
                _ => {} // reply, etc. — skip for now
            }
        }

        // Update enumeration progress
        if let Some(oldest_ts) = oldest_timestamp_in_page {
            if let Some(dt) = chrono::DateTime::from_timestamp(oldest_ts, 0) {
                notesdaq_db::queries::update_notes_progress(pool, root_post_id, dt, false)
                    .await?;
            }
        }

        pages += 1;

        // Check pagination
        let next_link = &data["_links"]["next"];
        if next_link.is_null() {
            break;
        }

        before_timestamp = next_link["query_params"]["before_timestamp"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| next_link["query_params"]["before_timestamp"].as_i64());

        if before_timestamp.is_none() {
            break;
        }
    }

    // Now assign share prices: sort reblogs chronologically and assign sequential counts
    all_reblog_notes.sort_by_key(|n| {
        n["timestamp"].as_i64().or_else(|| {
            n["timestamp"].as_f64().map(|f| f as i64)
        }).unwrap_or(0)
    });

    let mut reblog_count: i64 = 0;

    // Ensure the root post row exists before inserting events
    ensure_root_post(pool, root_post_id).await?;

    // Insert OP position (price 0) — find the root post's blog
    let op_uuid: Option<(String,)> = sqlx::query_as(
        "SELECT blog_uuid FROM posts WHERE post_id = $1",
    )
    .bind(root_post_id)
    .fetch_optional(pool)
    .await?;

    if let Some((op_uuid,)) = op_uuid {
        // Get the post's creation time for the OP position
        let op_time: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
            "SELECT created_at FROM posts WHERE post_id = $1",
        )
        .bind(root_post_id)
        .fetch_optional(pool)
        .await?;

        let op_time = op_time.map(|r| r.0).unwrap_or_else(chrono::Utc::now);

        // Check if OP position already exists
        let op_exists: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM reblog_events WHERE post_id = $1 AND is_op_position = TRUE LIMIT 1",
        )
        .bind(root_post_id)
        .fetch_optional(pool)
        .await?;

        if op_exists.is_none() {
            notesdaq_db::queries::insert_reblog_event(
                pool,
                root_post_id,
                &op_uuid,
                None,
                0, // price 0
                op_time,
                None,
                None,
                true, // is_op_position
            )
            .await?;
        }
    }

    // Insert reblog events with share prices
    for note in &all_reblog_notes {
        reblog_count += 1;

        let blog_uuid = match note["blog_uuid"].as_str() {
            Some(u) => u,
            None => continue,
        };
        let ts = note["timestamp"]
            .as_i64()
            .or_else(|| note["timestamp"].as_f64().map(|f| f as i64))
            .unwrap_or(0);
        let reblogged_at =
            chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(chrono::Utc::now);
        let reblog_post_id = note["post_id"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| note["post_id"].as_i64());
        let parent_blog_name = note["reblog_parent_blog_name"].as_str();

        // Resolve parent blog name to UUID if cached
        let parent_blog_uuid = if let Some(name) = parent_blog_name {
            notesdaq_db::queries::resolve_slug_to_uuid(pool, name).await?
        } else {
            None
        };

        notesdaq_db::queries::insert_reblog_event(
            pool,
            root_post_id,
            blog_uuid,
            reblog_post_id,
            reblog_count,
            reblogged_at,
            parent_blog_name,
            parent_blog_uuid.as_deref(),
            false,
        )
        .await?;

        // Add discovered blog to crawl queue with low priority
        notesdaq_db::queries::enqueue_blog(pool, blog_uuid, 0).await?;
    }

    // Mark post as fully enumerated
    let now = chrono::Utc::now();
    notesdaq_db::queries::update_notes_progress(pool, root_post_id, now, true).await?;

    // Record a snapshot
    notesdaq_db::queries::insert_snapshot(pool, root_post_id, reblog_count, now).await?;

    Ok(reblog_count)
}

/// Same as enumerate_notes but uses a different blog slug for the API call.
fn enumerate_notes_via<'a>(
    pool: &'a sqlx::PgPool,
    client: &'a tumblr::TumblrClient,
    blog_slug: &'a str,
    root_post_id: i64,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + 'a>> {
    Box::pin(enumerate_notes(pool, client, blog_slug, root_post_id))
}

/// Try to find the root blog's slug from our database.
async fn try_root_blog_slug(pool: &sqlx::PgPool, post_id: i64) -> Result<Option<String>> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        r#"
        SELECT b.slug FROM posts p
        JOIN blogs b ON b.uuid = p.blog_uuid
        WHERE p.post_id = $1
        "#,
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.0))
}

/// Ensure a blog row exists with minimal info (uuid + slug only).
async fn ensure_blog_minimal(pool: &sqlx::PgPool, uuid: &str, slug: &str) -> Result<()> {
    notesdaq_db::queries::upsert_blog(pool, uuid, slug, None, None, None, None, None, false).await
}

/// Ensure a blog exists from a post's embedded blog object.
async fn ensure_blog_from_post(pool: &sqlx::PgPool, post: &serde_json::Value) -> Result<()> {
    let blog = &post["blog"];
    if let (Some(uuid), Some(name)) = (blog["uuid"].as_str(), blog["name"].as_str()) {
        notesdaq_db::queries::upsert_blog(
            pool,
            uuid,
            name,
            blog["title"].as_str(),
            blog["description"].as_str(),
            None, // followers not in embedded blog objects
            blog["avatar"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v["url"].as_str()),
            blog["url"].as_str(),
            false,
        )
        .await?;
    }
    Ok(())
}

/// Ensure a root post row exists (it might only be known from notes, not from a blog crawl).
/// If we don't know the blog, we create a placeholder blog entry.
async fn ensure_root_post(pool: &sqlx::PgPool, post_id: i64) -> Result<()> {
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT post_id FROM posts WHERE post_id = $1")
            .bind(post_id)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        // Ensure the placeholder blog exists
        ensure_blog_minimal(pool, "unknown", "unknown").await?;
        // Insert a minimal post row — we don't have full metadata yet
        sqlx::query(
            r#"
            INSERT INTO posts (post_id, blog_uuid, created_at, note_count)
            VALUES ($1, 'unknown', now(), 0)
            ON CONFLICT (post_id) DO NOTHING
            "#,
        )
        .bind(post_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Extract plain text and media URLs from a post's trail/content.
fn extract_content(post: &serde_json::Value) -> (Option<String>, Vec<String>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut media_urls: Vec<String> = Vec::new();

    // Extract from trail (the full post content chain)
    if let Some(trail) = post["trail"].as_array() {
        for item in trail {
            if let Some(content) = item["content"].as_array() {
                extract_from_blocks(content, &mut text_parts, &mut media_urls);
            }
        }
    }

    // Extract from content (the reblogger's own additions, usually empty)
    if let Some(content) = post["content"].as_array() {
        extract_from_blocks(content, &mut text_parts, &mut media_urls);
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    };

    (text, media_urls)
}

fn extract_from_blocks(
    blocks: &[serde_json::Value],
    text_parts: &mut Vec<String>,
    media_urls: &mut Vec<String>,
) {
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(text) = block["text"].as_str() {
                    if !text.is_empty() {
                        text_parts.push(text.to_string());
                    }
                }
            }
            Some("image") => {
                // Media array has multiple sizes; take the first (largest)
                if let Some(media) = block["media"].as_array() {
                    if let Some(url) = media.first().and_then(|m| m["url"].as_str()) {
                        media_urls.push(url.to_string());
                    }
                }
            }
            Some("video") => {
                if let Some(url) = block["url"].as_str() {
                    media_urls.push(url.to_string());
                } else if let Some(media) = block["media"].as_object() {
                    if let Some(url) = media.get("url").and_then(|u| u.as_str()) {
                        media_urls.push(url.to_string());
                    }
                }
            }
            Some("audio") => {
                if let Some(url) = block["url"].as_str() {
                    media_urls.push(url.to_string());
                }
            }
            Some("link") => {
                if let Some(url) = block["url"].as_str() {
                    media_urls.push(url.to_string());
                }
                if let Some(title) = block["title"].as_str() {
                    text_parts.push(title.to_string());
                }
                if let Some(description) = block["description"].as_str() {
                    text_parts.push(description.to_string());
                }
            }
            _ => {}
        }
    }
}
