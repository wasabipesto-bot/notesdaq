# notesdaq

A virtual stock market built on Tumblr reblog data. Every reblog is retroactively treated as a stock purchase — the "share price" is the number of reblogs the root post had at the moment of reblogging. As the post spreads, your investment grows.

No real money. No accounts. No opt-in. Just public Tumblr data and bragging rights.

## How it works

1. The **fetcher** crawls Tumblr blogs via the API, enumerating every reblog on every post
2. Reblogs are sorted chronologically and assigned sequential share prices (1st reblog = price 1, 2nd = price 2, etc.)
3. The original poster gets a synthetic position at price 0 (infinite return)
4. The **API** serves portfolio and stock data
5. The **frontend** lets you look up any blog and see their investment portfolio

## Architecture

```
notesdaq/
├── crates/
│   ├── notesdaq-db/       # Shared DB library (sqlx migrations, queries, models)
│   ├── notesdaq-fetch/    # Tumblr crawler binary (OAuth1, rate-limited)
│   └── notesdaq-api/      # Axum JSON API server
├── web/                   # Astro frontend
├── docker-compose.yml     # TimescaleDB
└── stats.sh               # Crawl progress dashboard
```

## Stack

- **Database**: PostgreSQL 17 + TimescaleDB (Docker)
- **Backend**: Rust (sqlx, axum, reqwest, OAuth1)
- **Frontend**: Astro 6, Chart.js
- **Crawl source**: Tumblr API v2 with full OAuth1 signing

## Running

### Prerequisites

- Rust toolchain
- Node.js 24+
- Docker (for TimescaleDB)
- Tumblr OAuth credentials in `.env`

### Database

```sh
docker compose up -d
```

### Fetcher

Crawls Tumblr and populates the database. Respects rate limits (1,000/hr, 5,000/day). Resumable.

```sh
cargo run --bin notesdaq-fetch
```

### API server

```sh
cargo run --bin notesdaq-api
# Listens on :3001
```

### Frontend

```sh
cd web
npm install
npx astro dev --host 0.0.0.0
# Listens on :4321, proxies /api/* to :3001
```

### Monitor crawl progress

```sh
./stats.sh
```

## .env

```
DATABASE_URL=postgres://notesdaq:notesdaq@localhost/notesdaq
TUMBLR_CONSUMER_KEY=...
TUMBLR_CONSUMER_SECRET=...
TUMBLR_TOKEN=...
TUMBLR_TOKEN_SECRET=...
SEED_BLOG=wasabipesto
RAW_RESPONSE_DIR=raw_responses
RUST_LOG=notesdaq_fetch=info
```

## Key design decisions

- **Blog primary key**: Tumblr UUID (`t:xxxxx`), not the mutable slug
- **Share price derivation**: Full notes enumeration (Tumblr has no `reblog_count` field)
- **Content storage**: Plain text + media URLs + raw NPF trail (for future search and rendering)
- **Rate limit strategy**: 1s delay between requests + pause when headers show limits approaching
- **Raw responses**: Saved as JSON files on disk for debugging, not in the database
