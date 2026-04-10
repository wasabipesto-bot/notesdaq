use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Blog {
    pub uuid: String,
    pub slug: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub follower_count: Option<i64>,
    pub avatar_url: Option<String>,
    pub blog_url: Option<String>,
    pub is_nsfw: bool,
    pub slug_updated_at: Option<DateTime<Utc>>,
    pub info_fetched_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Post {
    pub post_id: i64,
    pub blog_uuid: String,
    pub root_post_id: Option<i64>,
    pub parent_post_id: Option<i64>,
    pub parent_blog_uuid: Option<String>,
    pub root_blog_uuid: Option<String>,
    pub created_at: DateTime<Utc>,
    pub note_count: i64,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub content_text: Option<String>,
    pub content_media: Vec<String>,
    pub trail_npf: Option<serde_json::Value>,
    pub post_url: Option<String>,
    pub short_url: Option<String>,
    pub interactability: Option<String>,
    pub notes_enumerated_through: Option<DateTime<Utc>>,
    pub notes_fully_enumerated: bool,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_in_db_at: DateTime<Utc>,
    pub updated_in_db_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReblogEvent {
    pub id: i64,
    pub post_id: i64,
    pub blog_uuid: String,
    pub reblog_post_id: Option<i64>,
    pub reblog_count_at_time: i64,
    pub reblogged_at: DateTime<Utc>,
    pub parent_blog_name: Option<String>,
    pub parent_blog_uuid: Option<String>,
    pub is_op_position: bool,
    pub created_in_db_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Like {
    pub id: i64,
    pub post_id: i64,
    pub blog_uuid: String,
    pub liked_at: DateTime<Utc>,
    pub created_in_db_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CrawlQueueEntry {
    pub blog_uuid: String,
    pub priority: i32,
    pub status: String,
    pub posts_fetched: i32,
    pub total_posts: Option<i32>,
    pub last_error: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
