# Notesdaq — Design Document

> Maintained by the build agent. Updated as decisions are made and API behavior is verified.
> Last updated: 2026-04-10.

## What is Notesdaq?

A virtual stock market built on Tumblr reblog data. Every reblog is retroactively an
"investment" — the share price is the number of reblogs the root post had at the moment
of reblogging. Profit accumulates as the post spreads. No real money, no accounts, no
opt-in. Public Tumblr data only.

---

## Tumblr API — Verified Behavior

All findings below are from live API testing on 2026-04-10 against the `wasabipesto`
blog and the Tumblr staff blog.

### Authentication

- Full OAuth1 signing (consumer key + consumer secret + token + token secret) is required.
- API-key-only auth (consumer key as `api_key` query param) works for some public
  endpoints via curl, but OAuth is needed for the notes endpoint and follower counts.
- **TLS fingerprint blocking**: Tumblr's nginx rejects requests from `httpx` (Python)
  with HTTP 403. The `requests` library (Python) and `curl` work fine. In Rust, `reqwest`
  with native-tls or rustls should work (same TLS behavior as curl). Verified that
  `requests` + `requests-oauthlib` works for all endpoints.

### The reblog_count field does NOT exist

**This is the single most important API finding.** The spec assumed post objects would
have a `reblog_count` field separate from `note_count`. This is false. Post objects only
have `note_count`, which includes reblogs + likes + replies.

There is no cheap way to get a reblog-only count from the API. The share price must be
derived by enumerating individual notes via the `/notes` endpoint and counting
reblog-type events.

### Blog info (`/v2/blog/{blog}/info`)

Returns (verified fields):
- `uuid` — stable identifier, format `t:xxxxx`. **This is the primary key for blogs.**
- `name` — the URL slug (mutable — users can change it)
- `followers` — follower count (requires OAuth)
- `total_posts` — post count
- `description`, `title` — display text
- `avatar` — array of sizes (512, 128, 96, 64)
- `updated` — Unix timestamp of last activity
- `type` — "public" for public blogs
- `url` — full blog URL

### Post listing (`/v2/blog/{blog}/posts`)

With params `npf=true&reblog_info=true&notes_info=true`:

- `id` / `id_string` — post ID (use the integer)
- `blog_name`, `blog.uuid` — blog that owns this post/reblog
- `note_count` — total notes (reblogs + likes + replies)
- `timestamp`, `date` — creation time
- `tags` — TEXT array, the **reblogger's** tags (not the OP's)
- `summary` — short text preview
- `post_url`, `short_url` — links to the post
- `trail` — NPF structured content (array of hops with blog info and content blocks)
- `content` — the reblogger's own content additions (usually empty for reblogs)
- `interactability_reblog` — "everyone" or restricted
- `state` — "published", etc.

Reblog-specific fields (present on reblog posts in the listing, NOT on single post fetch):
- `reblogged_from_id`, `reblogged_from_name`, `reblogged_from_uuid`
- `reblogged_root_id`, `reblogged_root_name`, `reblogged_root_uuid`

Pagination: `limit` (max 20) + `offset`. `total_posts` in response for page count.

Inline notes: the `notes` field contains a preview (~50 recent notes) with full
structure (type, timestamp, blog_uuid). These are a convenience — the dedicated notes
endpoint is needed for complete enumeration.

### Notes endpoint (`/v2/blog/{blog}/notes?id={post_id}&mode=all`)

**Notes aggregate to the root post.** Querying any post in a reblog chain returns the
same note set. Verified: querying wasabipesto's reblog and the root blog
(michaelnordeman) returned identical `total_notes` (1,373) and identical note lists.

This means we can query notes via any blog+post_id pair in the chain — we don't need
to discover and query the root blog specifically.

Pagination: 50 notes per page, newest-to-oldest. Cursor is `_links.next.before_timestamp`.
Response includes `total_notes`.

Reblog note fields:
- `type` — "reblog"
- `timestamp` — Unix timestamp (integer in mode=all, float in mode=conversation)
- `blog_name`, `blog_uuid`, `blog_url` — the reblogger
- `post_id` — the reblogger's own post ID for this reblog
- `reblog_parent_blog_name` — who they reblogged from (name only, NOT UUID)
- `avatar_shape`, `avatar_url` — avatar data

Like note fields:
- `type` — "like"
- `timestamp`, `blog_name`, `blog_uuid`, `blog_url`, `avatar_*`
- No `post_id` (likes don't create posts)

Conversation mode (`mode=conversation`) returns additional fields on reblog notes:
- `added_text` — text the reblogger added
- `reblog_parent_post_id`, `reblog_parent_blog_url`
- `is_original_poster`
These are useful but not needed for the core flow.

### Single post fetch (`/v2/blog/{blog}/posts/{post_id}`)

Returns the post object but with FEWER reblog fields than the listing endpoint.
`reblogged_from_id`, `reblogged_from_uuid`, `reblogged_root_id` are **missing** from
the single post endpoint. Use the listing endpoint for reblog chain data.

### Deleted / nonexistent posts

- Returns HTTP 404 with JSON body: `{"meta":{"status":404,"msg":"Not Found"},...}`
- Clean and detectable. Both nonexistent posts and nonexistent blogs return 404.
- Deleted posts can still be reblogged from existing reblogs in the chain.
- "Unrebloggable" posts cannot be reblogged but still have historical data.
- Neither should be excluded from analysis.
- Private posts (blogs made private) won't appear in API results.
- No special-case handling — log errors as they come.

### Rate limits

Confirmed from response headers:
- `X-Ratelimit-Perhour-Limit: 1000`
- `X-Ratelimit-Perhour-Remaining: <n>`
- `X-Ratelimit-Perhour-Reset: <seconds>`
- `X-Ratelimit-Perday-Limit: 5000`
- `X-Ratelimit-Perday-Remaining: <n>`
- `X-Ratelimit-Perday-Reset: <seconds>`

Rate limit increase: email api_ratelimit@tumblr.com. Not yet requested — will do after
MVP is working.

### Post ID resolution

Post IDs from Tumblr URLs may be the root post's ID, not the reblogger's ID. When
fetching a reblog by the root post ID via the reblogger's blog, the API returns 404.
Always use the blog-specific post ID from the posts listing or notes endpoint.

---

## Data Model

### Core entities

**Blogs** — keyed by Tumblr UUID (`t:xxxxx`). Slug is display-only and mutable.
Blog name → UUID mappings are cached and refreshed ~daily via `/blog/{blog}/info`.

**Posts** — keyed by Tumblr post ID. Both original posts and reblogs are stored.
A reblog has a `root_post_id` pointing to the original. Content fields (text, media,
NPF trail, tags) are extracted during ingestion for future search.

**Reblog events** — one row per reblog. The core "position" in the stock market.
`reblog_count_at_time` is the share price, derived from sequential count of
reblog-type notes on the root post. Multiple reblogs of the same post by the same
user each get their own row.

**Reblog count snapshots** — TimescaleDB hypertable. Records the running reblog count
for a root post at a point in time. Used for share price history charts.

**Likes** — one row per like event. Not part of scoring, stored for future analysis.

### Share price derivation

Since `reblog_count` doesn't exist as an API field:

1. Enumerate all notes for a root post via `/notes?mode=all`, paginating
   newest-to-oldest.
2. Collect all reblog-type notes.
3. Sort chronologically (oldest first).
4. Assign sequential reblog count: first reblog = price 1, second = price 2, etc.
5. The OP gets a synthetic position at price 0.
6. Store each as a `reblog_event` row with `reblog_count_at_time`.

Share prices can only be assigned after **full** enumeration of a post's notes.
Progress is tracked via `notes_enumerated_through` on the post.

For ongoing updates after full enumeration: fetch new notes since last check. New
reblogs get prices = previous total + position in new batch.

### Scoring

- Raw multiplier: `current_reblog_count / buy_price`
- Buy price of 0 (OP) yields infinity — stored as-is, displayed as +Inf
- Aggregation: average multiplier over reblogs (not including OP position)
- Formula is in application code, not materialized in the DB

### What we store for future features

- **Tags** on reblogs — for "investing at X notes" meme detection, search
- **Follower count** per blog — influence correlation
- **content_text** — plain text from trail, for full-text search
- **content_media** — TEXT[] of embedded image/video URLs
- **trail_npf** — JSONB, raw NPF trail for frontend rendering
- **Like events** — for analytics beyond share price
- **Blog descriptions, titles, avatars** — for display

Tags and reblog additions are only captured when we crawl a blog's posts directly.
The notes endpoint does not include tags. Coverage grows as more blogs are crawled.

---

## Architecture

### Fetch binary (`notesdaq-fetch`)

Single Rust binary. Synchronous blocking requests with ~1 second delay between calls.
Reads rate limit headers and pauses if approaching limits.

Crawl loop:
1. Seed with a blog (initially: wasabipesto).
2. Paginate all posts via `/blog/{blog}/posts` (20/page).
3. For each unique root post, enumerate all notes via `/blog/{blog}/notes`.
4. After full enumeration, assign share prices and insert reblog_events.
5. Discovered blogs go into an uncrawled pool. After finishing the seed blog,
   pick one at random and repeat.

All progress is persisted to the database. The fetcher can be stopped and resumed.

Raw API responses are saved as JSON files on disk (not in the DB), organized by
endpoint and timestamp. Each file includes request details and response headers.

### Blog name → UUID resolution

A lookup cache populated from every API response that includes blog info. Refreshed
explicitly via `/blog/{blog}/info` calls roughly once per day. Stored in the blogs
table.

### HTTP client

Uses `reqwest` in Rust (Tumblr blocks `hyper`-based clients with certain TLS
fingerprints). OAuth1 signing via the `oauth1-request` crate or manual HMAC-SHA1.

### Database

PostgreSQL + TimescaleDB extension. Single database. TimescaleDB hypertable for
`reblog_count_snapshots` only; everything else is standard PostgreSQL.

Local dev: Docker Compose with `timescale/timescaledb:latest-pg17` on this VPS.
Production target: Shalash (AMD Ryzen 7 7840U, 96GB DDR5, 4TB NVMe) via Docker
Compose in the `valinor` repo.

### API server (`notesdaq-api`) — not yet built

Axum HTTP server. Reads from PostgreSQL. Serves JSON API for the Astro frontend.
Will handle OG image generation (resvg + image crate).

### Frontend — not yet built

Astro with client:load islands. Separate from the API binary. Deployed to static host.

---

## Crawl Strategy

### Initial seed

Start with wasabipesto's blog (~18,400 posts). Estimated ~920 API calls for post
enumeration, plus ~30,000+ calls for notes enumeration on root posts. At 1,000
calls/hour with 1s delays, this takes roughly 30+ hours of continuous crawling.

### Expansion

After the seed blog is complete, pick a random undiscovered blog from the crawl pool
(populated from notes enumeration) and repeat. The owner will review results and choose
new seed blogs manually.

### Budget

- 1,000 requests/hour, 5,000/day (current limits)
- ~1 request/second target rate
- `note_count` on post objects serves as a cheap activity indicator for polling priority
- Full notes enumeration is the expensive operation

---

## Decisions Log

| Decision | Choice | Date | Rationale |
|---|---|---|---|
| Share price source | Notes enumeration (count reblogs) | 2026-04-10 | `reblog_count` field doesn't exist in API |
| HTTP client (Python tests) | `requests` + `requests-oauthlib` | 2026-04-10 | Tumblr TLS-fingerprint blocks `httpx` |
| HTTP client (Rust) | `reqwest` | 2026-04-10 | Same TLS reasoning; widely used |
| Rate limit strategy | 1s delay + respect headers | 2026-04-10 | Simple, debuggable, sufficient at current limits |
| Raw response storage | JSON files on disk | 2026-04-10 | Avoids DB bloat; includes headers and request metadata |
| Post content storage | content_text + content_media + trail_npf | 2026-04-10 | Supports future search without premature HTML rendering |
| Blog PK | Tumblr UUID | 2026-04-10 | Slugs are mutable |
| OP position | Synthetic reblog_event at price 0 | 2026-04-10 | Simplifies portfolio queries |
| Buy price 0 multiplier | Infinity | 2026-04-10 | Stored as-is; frontend displays +Inf |
| Deleted/private posts | No special handling | 2026-04-10 | Log errors, don't exclude from analysis |
| Likes | Store in separate table | 2026-04-10 | Free data from notes endpoint; future analytics |
| Notes aggregation | Query any post in chain | 2026-04-10 | Verified: notes aggregate to root post across all copies |
| Blog name→UUID | Cache in DB, refresh daily | 2026-04-10 | Notes only give parent blog name, not UUID |
