//! Orchestrator domain types (port of `orchestrator/types.ts`).

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::types::ParsedTargetItem;

/// TS `ActiveRipJob` — live job bookkeeping. TS mutates this object by
/// reference from several tasks; here the orchestrator owns it and hands
/// out read-only snapshots via progress events.
#[derive(Debug, Clone)]
pub struct ActiveRipJob {
    pub id: String,
    pub chat_id: i64,
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
    /// TS `queuePosition?` — maintained by the /alac command handler (M5b),
    /// not by `startJob` itself.
    pub queue_position: Option<u64>,
}

/// TS `RipJobOptions`.
#[derive(Debug, Clone)]
pub struct RipJobOptions {
    pub chat_id: i64,
    pub user_id: i64,
    pub user_name: Option<String>,
    /// Chat the file copy is delivered to (TS allows number | string; the
    /// bot resolves usernames/ids to i64 before enqueueing).
    pub delivery_chat_id: i64,
    pub is_group: bool,
    pub is_force: bool,
    pub is_cache_only: bool,
    pub single_storefront: Option<String>,
    pub parsed_items: Vec<ParsedTargetItem>,
    pub reply_to_message_id: Option<i64>,
    pub status_msg_id: i64,
    pub is_admin: bool,
}

/// TS `RipJobProgress` — every field optional except the counters.
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

/// TS `RipJobSummary.failedTracks` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedTrack {
    pub id: String,
    pub error: String,
}

/// TS `RipJobSummary`.
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
}

/// Events emitted by the orchestrator (TS EventEmitter: job:created,
/// job:started, job:progress, job:completed, job:cancelled, job:failed).
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
