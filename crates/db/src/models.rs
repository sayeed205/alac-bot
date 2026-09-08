use chrono::{DateTime, Utc};
use welds::prelude::*;

/// A Telegram user or chat granted access to the bot.
#[derive(Debug, Clone, Default, WeldsModel)]
#[welds(schema = "public", table = "users")]
pub struct User {
    #[welds(primary_key)]
    pub telegram_id: i64,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, WeldsModel)]
#[welds(schema = "public", table = "tracks")]
pub struct Track {
    #[welds(primary_key)]
    pub id: i32,
    pub apple_track_id: String,
    pub message_id: i32,
    pub file_id: String,
    pub file_unique_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i32,
    pub bit_depth: i32,
    pub sample_rate: i32,
    pub genre: String,
    pub release_date: String,
    pub track_number: i32,
    pub track_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, WeldsModel)]
#[welds(schema = "public", table = "requests")]
pub struct Request {
    #[welds(primary_key)]
    pub id: i32,
    pub telegram_id: i64,
    pub chat_id: i64,
    pub apple_track_id: String,
    pub is_cache_hit: bool,
    pub duration_ms: Option<i32>,
    pub status: String,
    pub error_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, WeldsModel)]
#[welds(schema = "public", table = "settings")]
pub struct Setting {
    #[welds(primary_key)]
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}
