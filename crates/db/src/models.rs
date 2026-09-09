use chrono::{DateTime, Utc};
use diesel::prelude::*;
use engine::types::Provider;

use crate::schema::{requests, settings, tracks, users};

#[derive(Debug, Clone, Default, Queryable, Selectable)]
#[diesel(table_name = users)]
pub struct User {
    pub telegram_id: i64,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable, QueryableByName)]
#[diesel(table_name = tracks)]
pub struct Track {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub id: i32,
    #[diesel(sql_type = diesel::sql_types::VarChar)]
    pub provider: Provider,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub track_id: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub message_id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub file_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub file_unique_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub title: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub artist: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub album: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub duration: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub bit_depth: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub sample_rate: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub genre: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub release_date: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub track_number: i32,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub track_count: i32,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub created_at: DateTime<Utc>,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = requests)]
pub struct Request {
    pub id: i32,
    pub telegram_id: i64,
    pub chat_id: i64,
    pub provider: Provider,
    pub track_id: String,
    pub is_cache_hit: bool,
    pub duration_ms: Option<i32>,
    pub status: String,
    pub error_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = settings)]
pub struct SettingsRow {
    pub id: i16,
    pub ripping_mode: String,
    pub album_rip_enabled: bool,
    pub playlist_rip_enabled: bool,
    pub artist_rip_enabled: bool,
    pub txt_rip_enabled: bool,
    pub multi_link_rip_enabled: bool,
    pub max_collection_tracks: i32,
    pub auto_dump_enabled: bool,
    pub auto_dump_storefronts: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub telegram_id: i64,
    pub name: Option<&'a str>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = tracks)]
pub struct NewTrack<'a> {
    pub provider: Provider,
    pub track_id: &'a str,
    pub message_id: i32,
    pub file_id: &'a str,
    pub file_unique_id: &'a str,
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub duration: i32,
    pub bit_depth: i32,
    pub sample_rate: i32,
    pub genre: &'a str,
    pub release_date: &'a str,
    pub track_number: i32,
    pub track_count: i32,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = requests)]
pub struct NewRequest<'a> {
    pub telegram_id: i64,
    pub chat_id: i64,
    pub provider: Provider,
    pub track_id: &'a str,
    pub is_cache_hit: bool,
    pub duration_ms: Option<i32>,
    pub status: &'a str,
    pub error_reason: Option<&'a str>,
}
