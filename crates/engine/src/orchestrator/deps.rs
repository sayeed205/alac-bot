//! Orchestrator dependency seams (port of `OrchestratorDependencies`).
//!
//! TS injects `{tg, service, ripper, queue, settings?, uploadRetryBaseMs?}`.
//! The engine keeps the typed pieces (queue) on the orchestrator itself and
//! routes everything environment-owned through this trait: the settings
//! snapshot, the store, catalog/playlist resolution, the rip call, and the
//! Telegram sink. Errors reduce to strings (TS `new Error(msg)` parity).

use std::{collections::HashMap, future::Future, path::Path, pin::Pin};

use tokio_util::sync::CancellationToken;

use crate::{
    playlist::PlaylistData,
    ripper::{RipError, RipProgressCallback},
    settings::BotSettings,
    types::{AlbumTracks, ArtistTracks, TrackRipResult},
};

/// `(uploaded_bytes, total_bytes)` for upload progress callbacks.
pub type UploadProgressCallback = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// One cache row for a track (the TS `Track` db row subset the
/// orchestrator reads).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedTrack {
    pub apple_track_id: String,
    pub message_id: i64,
    pub file_id: String,
    pub file_unique_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// TS `SaveTrackInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveTrackInput {
    pub apple_track_id: String,
    pub message_id: i64,
    pub file_id: String,
    pub file_unique_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i64,
    pub bit_depth: u32,
    pub sample_rate: u32,
    pub genre: String,
    pub release_date: String,
    pub track_number: i64,
    pub track_count: i64,
}

impl SaveTrackInput {
    /// Mirrors the TS uploader: `saveTrack({...})` built from the rip
    /// result plus the dump message ids.
    pub fn from_rip_result(
        track_id: &str,
        rip: &TrackRipResult,
        message_id: i64,
        file_id: &str,
        file_unique_id: &str,
    ) -> Self {
        Self {
            apple_track_id: track_id.to_string(),
            message_id,
            file_id: file_id.to_string(),
            file_unique_id: file_unique_id.to_string(),
            title: rip.title.clone(),
            artist: rip.artist.clone(),
            album: rip.album.clone(),
            duration: rip.duration,
            bit_depth: rip.bit_depth,
            sample_rate: rip.sample_rate,
            genre: rip.genre.clone(),
            release_date: rip.release_date.clone(),
            track_number: rip.track_number,
            track_count: rip.track_count,
        }
    }
}

/// TS `NewRequest` (request log row).
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLog {
    pub telegram_id: i64,
    pub chat_id: i64,
    pub apple_track_id: String,
    pub is_cache_hit: bool,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub error_reason: Option<String>,
}

/// Result of the dump-channel upload (the ids the store needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpUpload {
    pub message_id: i64,
    pub file_id: String,
    pub file_unique_id: String,
}

/// Telegram-side failures (surface as messages like TS).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SinkError(pub String);

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Telegram-side operations the orchestrator performs (implemented by the
/// bot crate via ferogram in M5b). Boxed futures keep the trait
/// object-safe for the `&dyn` sink accessor.
pub trait TelegramSink: Send + Sync {
    /// TS `tg.sendMedia(DUMP_CHANNEL_ID, {type:'audio', file, title,
    /// performer, duration}, {caption, progressCallback})`.
    ///
    /// `Ok(None)` models the TS `dumpMsg?.media?.type !== 'audio'` case: the
    /// send returned a message whose media is not audio (or no media) — the
    /// caller records 'Upload failed: no audio media returned' without
    /// retrying and without a request log.
    fn send_audio_to_dump<'a>(
        &'a self,
        file_path: &'a str,
        title: &'a str,
        performer: &'a str,
        duration: i64,
        caption_html: &'a str,
        on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> BoxFuture<'a, Result<Option<DumpUpload>, SinkError>>;

    /// TS `sendDumpCopy` — copy a dump message to a chat with the caption
    /// stripped, optional replyTo, optional silent.
    fn send_dump_copy<'a>(
        &'a self,
        to_chat_id: i64,
        message_id: i64,
        reply_to: Option<i64>,
        silent: bool,
    ) -> BoxFuture<'a, Result<(), SinkError>>;

    /// TS `tg.deleteMessagesById(env.DUMP_CHANNEL_ID, ids)` — errors
    /// swallowed by the caller.
    fn delete_dump_messages<'a>(
        &'a self,
        message_ids: &'a [i64],
    ) -> BoxFuture<'a, Result<(), SinkError>>;
}

/// Everything environment-owned the orchestrator calls. Generic on the
/// ripper seam so `start_job` can move an `Arc<D>` into the queue task.
pub trait OrchestratorDeps: Send + Sync + 'static {
    // ── settings (ISettingsService subset — snapshot + logic in engine) ──
    fn get_settings(&self) -> impl Future<Output = BotSettings> + Send;

    // ── store (IAlacService subset; string errors = TS messages) ────────
    fn find_cached_tracks(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<HashMap<String, CachedTrack>, String>> + Send;
    fn save_track(&self, input: SaveTrackInput) -> impl Future<Output = Result<(), String>> + Send;
    fn delete_track(
        &self,
        apple_track_id: &str,
    ) -> impl Future<Output = Result<bool, String>> + Send;
    fn log_request(&self, log: RequestLog) -> impl Future<Output = Result<(), String>> + Send;

    // ── catalog resolution ───────────────────────────────────────────────
    fn fetch_album_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<AlbumTracks, String>> + Send;
    fn fetch_artist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<ArtistTracks, String>> + Send;

    // ── playlist resolution ──────────────────────────────────────────────
    fn fetch_playlist_tracks(
        &self,
        id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<PlaylistData, crate::playlist::PlaylistError>> + Send;

    // ── rip (ITrackRipper.rip) ───────────────────────────────────────────
    fn rip(
        &self,
        track_id: &str,
        on_progress: Option<&RipProgressCallback>,
        storefront: &str,
        signal: CancellationToken,
        output_dir: Option<&Path>,
    ) -> impl Future<Output = Result<TrackRipResult, RipError>> + Send;

    // ── telegram sink ────────────────────────────────────────────────────
    fn sink(&self) -> &dyn TelegramSink;

    /// TS `uploadRetryBaseMs` default 3000.
    fn upload_retry_base_ms(&self) -> u64 {
        3000
    }
}
