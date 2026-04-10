-- Notesdaq initial schema
-- Requires: PostgreSQL 17 + TimescaleDB extension

-- Blogs: keyed by Tumblr UUID (stable), slug is display-only and mutable
CREATE TABLE IF NOT EXISTS blogs (
    uuid            TEXT PRIMARY KEY,          -- Tumblr UUID, e.g. "t:U7YuPu8OZt1EeYB9X2Asgg"
    slug            TEXT NOT NULL,             -- Current URL slug (mutable)
    title           TEXT,
    description     TEXT,
    follower_count  BIGINT,
    avatar_url      TEXT,                      -- URL to largest available avatar
    blog_url        TEXT,                      -- Full blog URL
    is_nsfw         BOOLEAN NOT NULL DEFAULT FALSE,
    slug_updated_at TIMESTAMPTZ,
    info_fetched_at TIMESTAMPTZ,              -- Last time we called /blog/{blog}/info
    deleted_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_blogs_slug ON blogs (slug);

-- Posts: both original posts and reblogs, keyed by Tumblr post ID
CREATE TABLE IF NOT EXISTS posts (
    post_id             BIGINT PRIMARY KEY,
    blog_uuid           TEXT NOT NULL REFERENCES blogs(uuid),
    root_post_id        BIGINT,               -- NULL for original posts; root post ID for reblogs
    parent_post_id      BIGINT,               -- NULL for original posts; immediate parent for reblogs
    parent_blog_uuid    TEXT REFERENCES blogs(uuid),
    root_blog_uuid      TEXT REFERENCES blogs(uuid),
    created_at          TIMESTAMPTZ NOT NULL,
    note_count          BIGINT NOT NULL DEFAULT 0,
    tags                TEXT[] NOT NULL DEFAULT '{}',
    summary             TEXT,
    content_text        TEXT,                  -- Plain text extracted from trail
    content_media       TEXT[] NOT NULL DEFAULT '{}', -- URLs of embedded media
    trail_npf           JSONB,                -- Raw NPF trail for rendering
    post_url            TEXT,
    short_url           TEXT,
    interactability     TEXT,                  -- "everyone" or restricted
    notes_enumerated_through TIMESTAMPTZ,     -- Oldest note timestamp we've reached (NULL = not started)
    notes_fully_enumerated   BOOLEAN NOT NULL DEFAULT FALSE,
    last_polled_at      TIMESTAMPTZ,
    deleted_at          TIMESTAMPTZ,
    created_in_db_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_in_db_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_posts_blog_uuid ON posts (blog_uuid);
CREATE INDEX IF NOT EXISTS idx_posts_root_post_id ON posts (root_post_id);
CREATE INDEX IF NOT EXISTS idx_posts_not_enumerated ON posts (note_count DESC)
    WHERE notes_fully_enumerated = FALSE AND root_post_id IS NULL;

-- Reblog events: one row per reblog, the core "position" in the market
-- Multiple reblogs of the same post by the same user each get their own row
CREATE TABLE IF NOT EXISTS reblog_events (
    id                      BIGSERIAL PRIMARY KEY,
    post_id                 BIGINT NOT NULL REFERENCES posts(post_id), -- The root post (the "stock")
    blog_uuid               TEXT NOT NULL REFERENCES blogs(uuid),      -- Who reblogged
    reblog_post_id          BIGINT,           -- The reblogger's own post ID (from notes endpoint)
    reblog_count_at_time    BIGINT NOT NULL,  -- Share price: # of reblogs before this one
    reblogged_at            TIMESTAMPTZ NOT NULL,
    parent_blog_name        TEXT,             -- Who they reblogged from (name, not UUID)
    parent_blog_uuid        TEXT REFERENCES blogs(uuid), -- Resolved from cache when available
    is_op_position          BOOLEAN NOT NULL DEFAULT FALSE, -- Synthetic OP position at price 0
    created_in_db_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_reblog_events_post_id ON reblog_events (post_id);
CREATE INDEX IF NOT EXISTS idx_reblog_events_blog_uuid ON reblog_events (blog_uuid);
CREATE INDEX IF NOT EXISTS idx_reblog_events_reblogged_at ON reblog_events (reblogged_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_reblog_events_unique_reblog
    ON reblog_events (post_id, blog_uuid, reblogged_at)
    WHERE is_op_position = FALSE;

-- Reblog count snapshots: TimescaleDB hypertable for share price history
CREATE TABLE IF NOT EXISTS reblog_count_snapshots (
    post_id         BIGINT NOT NULL REFERENCES posts(post_id),
    reblog_count    BIGINT NOT NULL,
    snapshotted_at  TIMESTAMPTZ NOT NULL
);

SELECT create_hypertable('reblog_count_snapshots', 'snapshotted_at', if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_snapshots_post_id ON reblog_count_snapshots (post_id, snapshotted_at DESC);

-- Likes: one row per like event, not part of scoring but stored for analytics
CREATE TABLE IF NOT EXISTS likes (
    id              BIGSERIAL PRIMARY KEY,
    post_id         BIGINT NOT NULL REFERENCES posts(post_id),
    blog_uuid       TEXT NOT NULL REFERENCES blogs(uuid),
    liked_at        TIMESTAMPTZ NOT NULL,
    created_in_db_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_likes_post_id ON likes (post_id);
CREATE INDEX IF NOT EXISTS idx_likes_blog_uuid ON likes (blog_uuid);
CREATE UNIQUE INDEX IF NOT EXISTS idx_likes_unique ON likes (post_id, blog_uuid, liked_at);

-- Crawl queue: tracks which blogs need to be crawled and progress
CREATE TABLE IF NOT EXISTS crawl_queue (
    blog_uuid           TEXT PRIMARY KEY REFERENCES blogs(uuid),
    priority            INTEGER NOT NULL DEFAULT 0,  -- Higher = crawl sooner
    status              TEXT NOT NULL DEFAULT 'pending', -- pending, in_progress, completed, failed
    posts_fetched       INTEGER NOT NULL DEFAULT 0,
    total_posts         INTEGER,
    last_error          TEXT,
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_crawl_queue_status ON crawl_queue (status, priority DESC);

-- Blog name -> UUID mapping cache
-- Populated from every API response; refreshed via explicit info calls
CREATE TABLE IF NOT EXISTS blog_name_cache (
    slug        TEXT PRIMARY KEY,
    blog_uuid   TEXT NOT NULL REFERENCES blogs(uuid),
    cached_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
