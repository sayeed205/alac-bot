//! Versioned, typed database archive export/import.
//!
//! The wire format is JSON compressed with gzip.  JSON's explicit null and
//! string escaping rules make this safe for names, error messages, and other
//! values containing newlines or SQL punctuation; no SQL is generated or
//! parsed during restore.

use std::{io::Write, time::Instant};

use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use flate2::{write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{requests, settings, tracks, users},
    DbError, DbPool, Request, SettingsRow, Track, User,
};

const ARCHIVE_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpStats {
    pub users_count: i64,
    pub tracks_count: i64,
    pub requests_count: i64,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreStats {
    pub users_merged: u64,
    pub tracks_merged: u64,
    pub requests_merged: u64,
    pub duration_ms: u128,
}

#[derive(Debug, Serialize, Deserialize)]
struct Archive {
    format_version: u32,
    generated_at: String,
    users: Vec<UserArchive>,
    tracks: Vec<TrackArchive>,
    requests: Vec<RequestArchive>,
    settings: SettingsArchive,
}

#[derive(Debug, Serialize, Deserialize, Insertable)]
#[diesel(table_name = users)]
struct UserArchive {
    telegram_id: i64,
    name: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Insertable)]
#[diesel(table_name = tracks)]
struct TrackArchive {
    provider: engine::Provider,
    track_id: String,
    message_id: i32,
    file_id: String,
    file_unique_id: String,
    title: String,
    artist: String,
    album: String,
    duration: i32,
    bit_depth: i32,
    sample_rate: i32,
    genre: String,
    release_date: String,
    track_number: i32,
    track_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Insertable)]
#[diesel(table_name = requests)]
struct RequestArchive {
    telegram_id: i64,
    chat_id: i64,
    provider: engine::Provider,
    track_id: String,
    is_cache_hit: bool,
    duration_ms: Option<i32>,
    status: String,
    error_reason: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsArchive {
    id: i16,
    ripping_mode: String,
    album_rip_enabled: bool,
    playlist_rip_enabled: bool,
    artist_rip_enabled: bool,
    txt_rip_enabled: bool,
    multi_link_rip_enabled: bool,
    max_collection_tracks: i32,
    auto_dump_enabled: bool,
    auto_dump_storefronts: Vec<String>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct DbDumpService {
    pool: DbPool,
}

impl DbDumpService {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn export_dump(&self) -> Result<(Vec<u8>, DumpStats, String), DbError> {
        let started = Instant::now();
        let mut connection = self.pool.connection().await?;
        let users = users::table
            .select(User::as_select())
            .load::<User>(&mut *connection)
            .await?;
        let tracks = tracks::table
            .select(Track::as_select())
            .load::<Track>(&mut *connection)
            .await?;
        let requests = requests::table
            .select(Request::as_select())
            .load::<Request>(&mut *connection)
            .await?;
        let settings = settings::table
            .select(SettingsRow::as_select())
            .first::<SettingsRow>(&mut *connection)
            .await?;
        let archive = Archive {
            format_version: ARCHIVE_VERSION,
            generated_at: chrono::Utc::now().to_rfc3339(),
            users: users.into_iter().map(UserArchive::from).collect(),
            tracks: tracks.into_iter().map(TrackArchive::from).collect(),
            requests: requests.into_iter().map(RequestArchive::from).collect(),
            settings: SettingsArchive::from(settings),
        };
        let json = serde_json::to_vec(&archive)
            .map_err(|error| DbError::Row(format!("archive encode failed: {error}")))?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&json)
            .map_err(|error| DbError::Row(format!("gzip encode failed: {error}")))?;
        let compressed = encoder
            .finish()
            .map_err(|error| DbError::Row(format!("gzip finish failed: {error}")))?;
        let filename = format!(
            "alac_dump_{}.json.gz",
            chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S")
        );
        let stats = DumpStats {
            users_count: archive.users.len() as i64,
            tracks_count: archive.tracks.len() as i64,
            requests_count: archive.requests.len() as i64,
            bytes: compressed.len(),
        };
        tracing::info!(
            users = stats.users_count,
            tracks = stats.tracks_count,
            requests = stats.requests_count,
            elapsed_ms = started.elapsed().as_millis(),
            "database archive exported"
        );
        Ok((compressed, stats, filename))
    }

    /// Restore a typed archive inside one transaction. Every row is decoded
    /// before the transaction starts, so malformed input cannot partially
    /// modify the database.
    pub async fn import_dump(&self, gzip_bytes: &[u8]) -> Result<RestoreStats, DbError> {
        let started = Instant::now();
        let archive: Archive = serde_json::from_slice(&gunzip(gzip_bytes)?)
            .map_err(|error| DbError::Row(format!("invalid archive: {error}")))?;
        if archive.format_version != ARCHIVE_VERSION {
            return Err(DbError::Row(format!(
                "unsupported archive version {}",
                archive.format_version
            )));
        }
        let users_merged = archive.users.len() as u64;
        let tracks_merged = archive.tracks.len() as u64;
        let requests_merged = archive.requests.len() as u64;
        let mut connection = self.pool.connection().await?;
        connection
            .build_transaction()
            .run(|transaction| {
                Box::pin(async move {
                    for row in archive.users {
                        diesel::insert_into(users::table)
                            .values(row)
                            .on_conflict(users::telegram_id)
                            .do_update()
                            .set((
                                users::name.eq(diesel::upsert::excluded(users::name)),
                                users::created_at.eq(diesel::upsert::excluded(users::created_at)),
                            ))
                            .execute(transaction)
                            .await?;
                    }
                    for row in archive.tracks {
                        diesel::insert_into(tracks::table)
                            .values(row)
                            .on_conflict((tracks::provider, tracks::track_id))
                            .do_update()
                            .set((
                                tracks::message_id.eq(diesel::upsert::excluded(tracks::message_id)),
                                tracks::file_id.eq(diesel::upsert::excluded(tracks::file_id)),
                                tracks::file_unique_id
                                    .eq(diesel::upsert::excluded(tracks::file_unique_id)),
                                tracks::title.eq(diesel::upsert::excluded(tracks::title)),
                                tracks::artist.eq(diesel::upsert::excluded(tracks::artist)),
                                tracks::album.eq(diesel::upsert::excluded(tracks::album)),
                                tracks::duration.eq(diesel::upsert::excluded(tracks::duration)),
                                tracks::bit_depth.eq(diesel::upsert::excluded(tracks::bit_depth)),
                                tracks::sample_rate
                                    .eq(diesel::upsert::excluded(tracks::sample_rate)),
                                tracks::genre.eq(diesel::upsert::excluded(tracks::genre)),
                                tracks::release_date
                                    .eq(diesel::upsert::excluded(tracks::release_date)),
                                tracks::track_number
                                    .eq(diesel::upsert::excluded(tracks::track_number)),
                                tracks::track_count
                                    .eq(diesel::upsert::excluded(tracks::track_count)),
                                tracks::created_at.eq(diesel::upsert::excluded(tracks::created_at)),
                                tracks::updated_at.eq(diesel::upsert::excluded(tracks::updated_at)),
                            ))
                            .execute(transaction)
                            .await?;
                    }
                    for row in archive.requests {
                        diesel::insert_into(requests::table)
                            .values(row)
                            .execute(transaction)
                            .await?;
                    }
                    let row = archive.settings;
                    diesel::update(settings::table.filter(settings::id.eq(1_i16)))
                        .set((
                            settings::ripping_mode.eq(row.ripping_mode),
                            settings::album_rip_enabled.eq(row.album_rip_enabled),
                            settings::playlist_rip_enabled.eq(row.playlist_rip_enabled),
                            settings::artist_rip_enabled.eq(row.artist_rip_enabled),
                            settings::txt_rip_enabled.eq(row.txt_rip_enabled),
                            settings::multi_link_rip_enabled.eq(row.multi_link_rip_enabled),
                            settings::max_collection_tracks.eq(row.max_collection_tracks),
                            settings::auto_dump_enabled.eq(row.auto_dump_enabled),
                            settings::auto_dump_storefronts.eq(row.auto_dump_storefronts),
                            settings::updated_at.eq(row.updated_at),
                        ))
                        .execute(transaction)
                        .await?;
                    Ok::<(), diesel::result::Error>(())
                })
            })
            .await?;
        let stats = RestoreStats {
            users_merged,
            tracks_merged,
            requests_merged,
            duration_ms: started.elapsed().as_millis(),
        };
        tracing::info!(?stats, "database archive imported");
        Ok(stats)
    }
}

impl From<User> for UserArchive {
    fn from(row: User) -> Self {
        Self {
            telegram_id: row.telegram_id,
            name: row.name,
            created_at: row.created_at,
        }
    }
}
impl From<Track> for TrackArchive {
    fn from(row: Track) -> Self {
        Self {
            provider: row.provider,
            track_id: row.track_id,
            message_id: row.message_id,
            file_id: row.file_id,
            file_unique_id: row.file_unique_id,
            title: row.title,
            artist: row.artist,
            album: row.album,
            duration: row.duration,
            bit_depth: row.bit_depth,
            sample_rate: row.sample_rate,
            genre: row.genre,
            release_date: row.release_date,
            track_number: row.track_number,
            track_count: row.track_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
impl From<Request> for RequestArchive {
    fn from(row: Request) -> Self {
        Self {
            telegram_id: row.telegram_id,
            chat_id: row.chat_id,
            provider: row.provider,
            track_id: row.track_id,
            is_cache_hit: row.is_cache_hit,
            duration_ms: row.duration_ms,
            status: row.status,
            error_reason: row.error_reason,
            created_at: row.created_at,
        }
    }
}
impl From<SettingsRow> for SettingsArchive {
    fn from(row: SettingsRow) -> Self {
        Self {
            id: row.id,
            ripping_mode: row.ripping_mode,
            album_rip_enabled: row.album_rip_enabled,
            playlist_rip_enabled: row.playlist_rip_enabled,
            artist_rip_enabled: row.artist_rip_enabled,
            txt_rip_enabled: row.txt_rip_enabled,
            multi_link_rip_enabled: row.multi_link_rip_enabled,
            max_collection_tracks: row.max_collection_tracks,
            auto_dump_enabled: row.auto_dump_enabled,
            auto_dump_storefronts: row.auto_dump_storefronts,
            updated_at: row.updated_at,
        }
    }
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, DbError> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|error| DbError::Row(format!("gunzip failed: {error}")))?;
    Ok(out)
}
