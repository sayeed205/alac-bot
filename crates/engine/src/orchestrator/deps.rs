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
    types::{AlbumTracks, ArtistTracks, Provider, TrackKey, TrackRipResult},
};

/// `(uploaded_bytes, total_bytes)` for upload progress callbacks.
pub type UploadProgressCallback = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// One cache row for a track (the TS `Track` db row subset the
/// orchestrator reads).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedTrack {
    pub track_key: TrackKey,
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
    pub track_key: TrackKey,
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
            track_key: TrackKey::new(Provider::Apple, track_id),
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
    pub track_key: TrackKey,
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

/// Metadata persisted for a completed album ZIP part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumUpload {
    pub provider: Provider,
    pub album_id: String,
    pub part_index: i32,
    pub total_parts: i32,
    pub message_id: i64,
    pub file_id: String,
    pub file_unique_id: String,
    pub file_size: i64,
    pub file_name: String,
    /// SHA-256 generation identity of the resolved track set. Empty string
    /// = unknown generation; never reused, forces one rebuild.
    pub generation_hash: String,
}

/// One cached album ZIP part read back for reuse decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAlbum {
    pub part_index: i32,
    pub total_parts: i32,
    pub message_id: i64,
    pub file_unique_id: String,
    pub generation_hash: String,
    /// Bytes of the stored archive part; powers the ZIP details message.
    pub file_size: i64,
}

/// Telegram-side failures (surface as messages like TS).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SinkError(pub String);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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

    /// Uploads a generated archive as a silent document in the dump channel.
    /// `thumb_path` attaches a Telegram document thumbnail when present
    /// (best-effort: implementations may drop it on failure).
    /// Implementations that do not support album archives can retain the
    /// default unsupported result (useful for narrow test adapters).
    fn send_document_to_dump<'a>(
        &'a self,
        _file_path: &'a str,
        _thumb_path: Option<&'a str>,
        _caption_html: &'a str,
        _on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> BoxFuture<'a, Result<Option<DumpUpload>, SinkError>> {
        Box::pin(async { Err(SinkError("ZIP document upload is unavailable".into())) })
    }

    /// Sends a generated archive directly to a chat without caching it.
    /// `thumb_path` attaches a Telegram document thumbnail when present.
    fn send_document_to_chat<'a>(
        &'a self,
        _chat_id: i64,
        _file_path: &'a str,
        _thumb_path: Option<&'a str>,
        _caption_html: &'a str,
        _on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> BoxFuture<'a, Result<i32, SinkError>> {
        Box::pin(async { Err(SinkError("direct ZIP upload is unavailable".into())) })
    }

    /// Sends a preview image (album cover) to a chat. Best-effort; the
    /// caller treats failures as non-fatal.
    fn send_photo_to_chat<'a>(
        &'a self,
        _chat_id: i64,
        _image_bytes: &'a [u8],
        _caption_html: &'a str,
    ) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async { Err(SinkError("photo send is unavailable".into())) })
    }

    /// Materializes a cached dump document for archive creation.
    fn download_dump_file<'a>(
        &'a self,
        _message_id: i64,
        _destination: &'a Path,
        _on_download_progress: Option<&'a UploadProgressCallback>,
    ) -> BoxFuture<'a, Result<(), SinkError>> {
        Box::pin(async { Err(SinkError("cached file download is unavailable".into())) })
    }

    /// TS `sendDumpCopy` — copy a dump message to a chat with the caption
    /// stripped, optional replyTo, optional silent. Returns the sent message ID.
    fn send_dump_copy<'a>(
        &'a self,
        to_chat_id: i64,
        message_id: i64,
        reply_to: Option<i64>,
        silent: bool,
    ) -> BoxFuture<'a, Result<i32, SinkError>>;

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
        keys: &[TrackKey],
    ) -> impl Future<Output = Result<HashMap<TrackKey, CachedTrack>, String>> + Send;
    fn save_track(&self, input: SaveTrackInput) -> impl Future<Output = Result<(), String>> + Send;
    fn delete_track(
        &self,
        track_key: &TrackKey,
    ) -> impl Future<Output = Result<bool, String>> + Send;
    fn log_request(&self, log: RequestLog) -> impl Future<Output = Result<(), String>> + Send;

    /// Persists a completed ZIP part. Partial archives intentionally never
    /// call this method.
    fn save_album<'a>(&'a self, _upload: AlbumUpload) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    /// Lists cached ZIP parts for an album, ordered by part_index ascending.
    fn find_albums<'a>(
        &'a self,
        _provider: Provider,
        _album_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<CachedAlbum>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Deletes all cached ZIP parts for an album before a rebuild republishes
    /// a possibly different part count.
    fn delete_albums<'a>(
        &'a self,
        _provider: Provider,
        _album_id: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    /// Fetches album artwork bytes for the ZIP cover (3000x3000 URL form).
    /// `None` = no artwork; failures degrade to a coverless archive.
    fn fetch_artwork<'a>(&'a self, _url: &'a str) -> BoxFuture<'a, Option<Vec<u8>>> {
        Box::pin(async { None })
    }

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

    /// Base delay for upload retries.  The live command defaults to 2000ms.
    fn upload_retry_base_ms(&self) -> u64 {
        2000
    }

    /// Number of retries after the initial upload attempt.  Thus the number
    /// of calls is `upload_max_retries() + 1`.
    fn upload_max_retries(&self) -> u32 {
        3
    }
}
