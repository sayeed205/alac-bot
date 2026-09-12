//! Orchestrator domain types.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::{
    types::{ParsedTargetItem, TargetKind},
    wrapper::CodecPreference,
};

/// The non-terminal lifecycle phase of a job.  Terminality is represented by
/// `terminal_state` below so consumers can retain the last useful phase while
/// rendering a completed/cancelled job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobPhase {
    Resolving,
    CheckingCache,
    Queued,
    Processing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalJobState {
    Completed,
    Cancelled,
    Failed,
}

/// Descriptive alias for callers that prefer the `Job*` naming convention.
pub type JobTerminalState = TerminalJobState;

/// Live job bookkeeping. The orchestrator owns it, mutates it from several
/// tasks behind its mutex, and hands out read-only snapshots via events.
#[derive(Debug, Clone)]
pub struct ActiveRipJob {
    pub id: String,
    pub chat_id: i64,
    /// Chat the delivered copies target (group jobs retarget to the user's
    /// DM); the bridge uses it to send ZIP details to the right chat.
    pub delivery_chat_id: i64,
    pub user_id: i64,
    pub user_name: Option<String>,
    pub job_header: String,
    pub total_tracks: usize,
    pub status_msg_id: i64,
    /// Shared cancellation token — cloned into every pipeline stage.
    pub controller: CancellationToken,
    pub is_cancelled: bool,
    pub cancelled_by: Option<String>,
    pub cached_count: usize,
    pub ripped_count: usize,
    pub failed_count: usize,
    pub completed: bool,
    pub start_time_ms: u64,
    /// Last `activeActionText` written by a progress update.
    pub active_action_text: Option<String>,
    /// Queue position, maintained by the queue rather than the job flow.
    pub queue_position: Option<u64>,
    pub phase: JobPhase,
    pub terminal_state: Option<TerminalJobState>,
    pub skipped_count: usize,
    pub is_cache_only: bool,
    pub is_group: bool,
    pub reply_to_message_id: Option<i64>,
}

/// Everything one rip request carries.
#[derive(Debug, Clone)]
pub struct RipJobOptions {
    pub chat_id: i64,
    pub user_id: i64,
    pub user_name: Option<String>,
    /// Chat the file copy is delivered to (numbers only in practice; the
    /// bot resolves usernames/ids to i64 before enqueueing).
    pub delivery_chat_id: i64,
    pub is_group: bool,
    pub is_force: bool,
    pub is_cache_only: bool,
    /// Request album ZIP packaging. Cache-only `/dump` jobs may set this to
    /// automatically publish a complete ZIP, but never deliver it to users.
    pub zip: bool,
    /// The user explicitly asked for ZIP packaging (`/zip` command or the
    /// `-z`/`--zip` token). The engine uses this only to warn when the
    /// album turns out to have a single track; implicit auto-attempts
    /// (cache-only `/dump`) never warn.
    pub zip_explicit: bool,
    pub single_storefront: Option<String>,
    pub parsed_items: Vec<ParsedTargetItem>,
    pub reply_to_message_id: Option<i64>,
    pub status_msg_id: i64,
    pub is_admin: bool,
    /// Audio variant the ripper selects when several are offered
    /// (highest quality vs Dolby Atmos).
    pub codec_preference: CodecPreference,
}

/// A progress snapshot; every display field is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct RipJobProgress {
    pub job_id: String,
    pub total_tracks: usize,
    pub completed_tracks: usize,
    pub cached_count: usize,
    pub ripped_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub percent: u32,
    pub active_download_text: Option<String>,
    pub active_upload_text: Option<String>,
    pub activity_override: Option<String>,
}

/// One failed track, as reported in the job summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedTrack {
    pub id: String,
    pub error: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub storefront: Option<String>,
}

impl FailedTrack {
    pub fn new(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            error: error.into(),
            title: None,
            artist: None,
            storefront: None,
        }
    }

    pub fn with_meta(
        mut self,
        title: Option<String>,
        artist: Option<String>,
        storefront: Option<String>,
    ) -> Self {
        self.title = title;
        self.artist = artist;
        self.storefront = storefront;
        self
    }
}

/// The end-of-job report for the requesting chat.
#[derive(Debug, Clone, PartialEq)]
pub struct RipJobSummary {
    pub job_id: String,
    pub job_header: String,
    pub total_tracks: usize,
    pub cached_count: usize,
    pub ripped_count: usize,
    pub failed_count: usize,
    pub failed_tracks: Vec<FailedTrack>,
    pub skipped_uncached_tracks: Vec<String>,
    pub total_elapsed_sec: String,
    pub capped_count: usize,
    pub max_collection_limit: u32,
    pub is_cache_only: bool,
    pub is_group: bool,
    /// User-facing notes appended to the completion message (plain text;
    /// the bridge escapes them). Empty in the common case.
    pub warnings: Vec<String>,
    /// Metadata about a user-delivered album ZIP, rendered as the details
    /// message in the delivery chat. `None` for cache-only jobs and
    /// non-ZIP jobs.
    pub zip_delivery: Option<ZipDeliveryInfo>,
    /// Telegram message ID of the first delivered track or ZIP in the delivery chat.
    pub first_delivered_msg_id: Option<i32>,
}

/// Album details for a delivered ZIP, powering the post-ZIP info message.
#[derive(Debug, Clone, PartialEq)]
pub struct ZipDeliveryInfo {
    pub album: String,
    pub artist: String,
    /// First four characters of the album release date, may be empty.
    pub release_year: String,
    pub total_tracks: usize,
    /// Tracks actually present in the delivered archive.
    pub delivered_tracks: usize,
    pub total_parts: usize,
    /// Total delivered archive bytes.
    pub size_bytes: i64,
    pub is_partial: bool,
    pub album_id: String,
    pub storefront: String,
    pub artwork_url: Option<String>,
    pub genre: Option<String>,
    pub record_label: Option<String>,
    pub copyright: Option<String>,
    pub photo_delivered: bool,
    /// Highest-quality codec in the archive (`alac`, `mp4a.40.2`, `ec-3`).
    pub codec: Option<String>,
}

/// A target which could not be resolved.  The engine deliberately keeps this
/// structured; presentation (including HTML escaping) belongs to the bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionFailure {
    pub kind: TargetKind,
    pub id: String,
    pub error: String,
}

impl std::fmt::Display for ResolutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", kind_name(self.kind), self.id, self.error)
    }
}

fn kind_name(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Track => "track",
        TargetKind::Album => "album",
        TargetKind::Artist => "artist",
        TargetKind::Playlist => "playlist",
    }
}

/// Events emitted by the orchestrator.
#[derive(Debug, Clone)]
pub enum OrchestratorEvent<'a> {
    /// `job:created`
    Created(&'a ActiveRipJob),
    /// `job:started`
    Started(&'a ActiveRipJob),
    /// `job:progress`
    Progress(&'a ActiveRipJob, &'a RipJobProgress),
    /// `job:completed`
    Completed(&'a ActiveRipJob, &'a RipJobSummary),
    /// `job:cancelled`
    Cancelled(&'a ActiveRipJob, &'a Option<String>),
    /// `job:failed`
    Failed(&'a ActiveRipJob, &'a str),
}

/// Callback type subscribed to orchestrator events.
pub type EventCallback = Arc<dyn Fn(&OrchestratorEvent<'_>) + Send + Sync>;
