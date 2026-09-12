//! Async bridge between the engine's synchronous event callbacks and
//! Telegram/dashboard work.
//!
//! The orchestrator emits events from inside its pipeline; those callbacks
//! must not perform network I/O (they can run under the job mutex and would
//! stall the whole queue). The sync subscription only **clones** event data
//! into a bounded mpsc channel; this module's spawned consumer does the
//! async rendering: one shared status dashboard per chat.

use std::sync::{Arc, Mutex};

use engine::orchestrator::{
    deps::OrchestratorDeps,
    types::{ActiveRipJob, FailedTrack, OrchestratorEvent, RipJobProgress, RipJobSummary},
};
use tokio::sync::mpsc;

use crate::{
    dashboard_manager,
    dashboard_map::{snapshot_from, JobContexts},
    mirror_health::last_known_health,
    BotState,
};

/// Owned copies of engine events, safe to move across an mpsc channel.
/// `job` is boxed: `ActiveRipJob` is large enough that six inline copies
/// would bloat every `BridgeEvent` to the size of the biggest variant.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Created {
        job: Box<ActiveRipJob>,
    },
    Progress {
        job: Box<ActiveRipJob>,
        progress: Box<RipJobProgress>,
    },
    Started {
        job: Box<ActiveRipJob>,
    },
    Completed {
        job: Box<ActiveRipJob>,
        summary: Box<RipJobSummary>,
    },
    Cancelled {
        job: Box<ActiveRipJob>,
        cancelled_by: Option<String>,
    },
    Failed {
        job: Box<ActiveRipJob>,
        error: String,
    },
}

impl BridgeEvent {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Cancelled { .. } | Self::Failed { .. }
        )
    }

    /// Clone a borrowed engine event into an owned, sendable copy.
    pub fn from_engine(event: &OrchestratorEvent<'_>) -> Option<Self> {
        Some(match event {
            OrchestratorEvent::Created(job) => BridgeEvent::Created {
                job: Box::new((*job).clone()),
            },
            OrchestratorEvent::Started(job) => BridgeEvent::Started {
                job: Box::new((*job).clone()),
            },
            OrchestratorEvent::Progress(job, progress) => BridgeEvent::Progress {
                job: Box::new((*job).clone()),
                progress: Box::new((*progress).clone()),
            },
            OrchestratorEvent::Completed(job, summary) => BridgeEvent::Completed {
                job: Box::new((*job).clone()),
                summary: Box::new((*summary).clone()),
            },
            OrchestratorEvent::Cancelled(job, by) => BridgeEvent::Cancelled {
                job: Box::new((*job).clone()),
                cancelled_by: (*by).clone(),
            },
            OrchestratorEvent::Failed(job, error) => BridgeEvent::Failed {
                job: Box::new((*job).clone()),
                error: error.to_string(),
            },
        })
    }
}

/// Job registry shared by the bridge consumer and dashboard renders.
pub struct BridgeRegistry {
    /// Rendering contexts keyed by job id (header/requester name).
    contexts: Mutex<JobContexts>,
}

impl BridgeRegistry {
    fn new() -> Self {
        Self {
            contexts: Mutex::new(JobContexts::new()),
        }
    }

    /// Remember a job's rendering context (header/requester) from its latest
    /// engine snapshot. `user_name` is set once at creation and the header is
    /// finalized post-resolution, so later snapshots are authoritative.
    pub fn remember(&self, job: &ActiveRipJob) {
        self.contexts
            .lock()
            .expect("bridge contexts poisoned")
            .remember(job);
    }

    pub fn remember_progress(&self, progress: &RipJobProgress) {
        self.contexts
            .lock()
            .expect("bridge contexts poisoned")
            .remember_progress(progress);
    }

    /// Drop a finished job's context and editor so neither registry grows
    /// unbounded.
    pub fn forget(&self, job_id: &str) {
        self.contexts
            .lock()
            .expect("bridge contexts poisoned")
            .forget(job_id);
    }

    /// Copy of all contexts, for whole-dashboard snapshot builds.
    pub fn contexts_snapshot(&self) -> JobContexts {
        let guard = self.contexts.lock().expect("contexts poisoned");
        let mut copy = JobContexts::new();
        for (id, context) in guard.iter() {
            copy.insert(id.clone(), context.clone());
        }
        copy
    }
}

static REGISTRY: std::sync::OnceLock<Arc<BridgeRegistry>> = std::sync::OnceLock::new();

pub fn registry() -> Arc<BridgeRegistry> {
    REGISTRY
        .get_or_init(|| Arc::new(BridgeRegistry::new()))
        .clone()
}

/// The consumer loop: one event at a time, network edits allowed.
async fn consume(state: Arc<BotState>, mut rx: mpsc::Receiver<BridgeEvent>) {
    while let Some(event) = rx.recv().await {
        if let Err(error) = handle_event(Arc::clone(&state), event).await {
            tracing::warn!(%error, "status event handling failed");
        }
    }
}

/// Subscribe the bridge to orchestrator events and spawn its consumer.
pub fn start(state: Arc<BotState>) {
    const EVENT_BUFFER: usize = 256;
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);

    // Sync subscriber: clone event data only, never await.
    state.rip_orchestrator.subscribe(Arc::new(move |event| {
        if let Some(owned) = BridgeEvent::from_engine(event) {
            match tx.try_send(owned) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(event)) if event.is_terminal() => {
                    // Never lose a terminal state just because progress edits
                    // are filling the bounded queue.
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(event).await;
                    });
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::debug!("dropping stale progress event after queue saturation");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => (),
            }
        }
    }));

    tokio::spawn(consume(state, rx));
}

/// Build a global dashboard snapshot from the engine's current jobs.
pub async fn current_snapshot(state: &BotState) -> crate::dashboard::DashboardSnapshot {
    let active = state.rip_orchestrator.get_active_jobs();
    let settings = state.rip_deps.get_settings().await;
    let mode = settings.ripping_mode.as_str().to_owned();
    // Refresh mirror health opportunistically: the last-known value renders
    // immediately, and a fresh probe runs only when the cached one is stale.
    // The probe is bounded (4s timeout) and shares the ripper's policy
    // manager, so an unreachable mirror cannot flood the transport.
    let health = if last_known_health().is_fresh() {
        last_known_health().label().map(str::to_owned)
    } else {
        let report = state.rip_deps.probe_mirror_health().await;
        last_known_health().record(report);
        Some(report.health.label().to_owned())
    };
    // Per-viewer permissions are applied by the dashboard manager per entry;
    // the global snapshot stays viewer-neutral.
    snapshot_from(
        &active,
        &registry().contexts_snapshot(),
        0,
        false,
        &mode,
        health,
    )
}

/// Single consumer turn: refresh the shared dashboard for the event.
async fn handle_event(state: Arc<BotState>, event: BridgeEvent) -> Result<(), String> {
    let state_ref = state.as_ref();
    match event {
        BridgeEvent::Created { job } => {
            registry().remember(&job);
            refresh_dashboard_for_job(state_ref, &job).await;
        }
        BridgeEvent::Progress { job, progress } => {
            registry().remember(&job);
            registry().remember_progress(&progress);
            refresh_dashboard(state_ref, false).await;
        }
        BridgeEvent::Started { job } => {
            registry().remember(&job);
            refresh_dashboard(state_ref, false).await;
        }
        BridgeEvent::Completed { job, summary } => {
            registry().remember(&job);
            registry().forget(&job.id);
            notify_job_completed(state_ref, &job, &summary).await;
            refresh_dashboard(state_ref, true).await;
        }
        BridgeEvent::Cancelled { job, .. } => {
            registry().remember(&job);
            registry().forget(&job.id);
            refresh_dashboard(state_ref, true).await;
        }
        BridgeEvent::Failed { job, .. } => {
            registry().remember(&job);
            registry().forget(&job.id);
            refresh_dashboard(state_ref, true).await;
        }
    }
    Ok(())
}

/// Refresh every open dashboard; `force` bypasses the coalescing window for
/// terminal/queue-shape changes.
async fn refresh_dashboard(state: &BotState, force: bool) {
    let snapshot = current_snapshot(state).await;
    dashboard_manager().refresh(snapshot, force).await;
}

/// Ensure a dashboard exists for a job even when the command-side preflight
/// could not open one (for example, a transient Telegram send failure). The
/// Created event is emitted after the job is admitted, so this is the first
/// reliable point at which we can recover without losing the status surface.
async fn refresh_dashboard_for_job(state: &BotState, job: &ActiveRipJob) {
    let snapshot = current_snapshot(state).await;
    let manager = dashboard_manager();
    if manager.contains(job.chat_id).await {
        if let Err(error) = manager.replace_entry_from(job.chat_id, snapshot).await {
            tracing::warn!(
                chat_id = job.chat_id,
                job_id = %job.id,
                error = ?error,
                "failed to replace status dashboard for new job"
            );
        }
        return;
    }

    let sink =
        crate::handlers::dashboard_sink(state.client.clone(), ferogram::PeerRef::from(job.chat_id));
    if let Err(error) = manager
        .open(
            job.chat_id,
            job.user_id,
            state.auth.is_admin(job.user_id),
            sink,
            snapshot,
        )
        .await
    {
        tracing::warn!(
            chat_id = job.chat_id,
            job_id = %job.id,
            error = ?error,
            "failed to open status dashboard for new job"
        );
    }
}

fn failed_track_reason(error: &str) -> Option<&'static str> {
    let lower = error.to_ascii_lowercase();
    if lower.contains("404")
        || lower.contains("failed to get m3u8")
        || lower.contains("unavailable")
        || lower.contains("unstreamable")
        || lower.contains("not streamable")
        || lower.contains("not available")
        || lower.contains("not found")
    {
        Some("Unavailable on Apple Music")
    } else if lower.contains("cancelled") {
        Some("Cancelled")
    } else if lower.contains("timed out") || lower.contains("timeout") {
        Some("Timed out")
    } else if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        Some("Authentication error")
    } else {
        None
    }
}

fn format_failed_track(failed: &FailedTrack) -> String {
    let label = match (&failed.title, &failed.artist) {
        (Some(title), Some(artist)) if !title.trim().is_empty() && !artist.trim().is_empty() => {
            format!(
                "{} - {}",
                crate::html::escape(title),
                crate::html::escape(artist)
            )
        }
        (Some(title), _) if !title.trim().is_empty() => crate::html::escape(title).to_string(),
        _ if !failed.id.is_empty() && failed.id.chars().all(|c| c.is_ascii_digit()) => {
            format!("Track {}", failed.id)
        }
        _ => crate::html::escape(&failed.id).to_string(),
    };

    let link = if !failed.id.is_empty() && failed.id.chars().all(|c| c.is_ascii_digit()) {
        let url = if let Some(sf) = failed
            .storefront
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            format!("https://music.apple.com/{sf}/song/{}", failed.id)
        } else {
            format!("https://music.apple.com/song/{}", failed.id)
        };
        format!(r#"<a href="{url}">{label}</a>"#)
    } else {
        label
    };

    if let Some(reason) = failed_track_reason(&failed.error) {
        format!("{link} <i>({reason})</i>")
    } else {
        link
    }
}

/// Send one terminal completion notice to the originating chat. Reply to the
/// command when it still exists; otherwise mention the requester explicitly
/// so completion remains visible even after message cleanup.
async fn notify_job_completed(state: &BotState, job: &ActiveRipJob, summary: &RipJobSummary) {
    let peer = ferogram::PeerRef::from(job.chat_id);
    let reply_id = job
        .reply_to_message_id
        .and_then(|raw| i32::try_from(raw).ok());
    let reply_id = match reply_id {
        Some(id) => state
            .client
            .get_messages(peer.clone(), &[id])
            .await
            .ok()
            .and_then(|messages| messages.first().map(|_| id)),
        None => None,
    };

    // Format user mention for origin chat
    let name = job
        .user_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("User");
    let mention = if name.starts_with('@') {
        let handle = name.trim_start_matches('@');
        format!(r#"<a href="https://t.me/{handle}">@{handle}</a>"#)
    } else if job.user_id > 0 {
        format!(
            r#"<a href="tg://user?id={}">{}</a>"#,
            job.user_id,
            crate::html::escape(name)
        )
    } else {
        crate::html::escape(name)
    };

    let elapsed = summary
        .total_elapsed_sec
        .parse::<f64>()
        .map(|seconds| crate::presentation::readable_time_compact(seconds as u64))
        .unwrap_or_else(|_| summary.total_elapsed_sec.clone());
    let mut lines = vec![
        summary.job_header.clone(),
        "┃".to_owned(),
        format!(
            "┣ Tracks: {} ({} cached · {} ripped · {} failed)",
            summary.total_tracks, summary.cached_count, summary.ripped_count, summary.failed_count
        ),
        format!("┣ Elapsed: {elapsed}"),
    ];
    if !summary.failed_tracks.is_empty() {
        if summary.failed_tracks.len() == 1 {
            let item = format_failed_track(&summary.failed_tracks[0]);
            lines.push(format!("┣ ❌ Failed: {item}"));
        } else {
            lines.push("┣ ❌ Failed tracks:".to_owned());
            for track in summary.failed_tracks.iter().take(15) {
                let item = format_failed_track(track);
                lines.push(format!("┣ • {item}"));
            }
            if summary.failed_tracks.len() > 15 {
                let rem = summary.failed_tracks.len() - 15;
                lines.push(format!("┣ • ...and {rem} more"));
            }
        }
    }
    // Engine-authored plain-text notes (e.g. single-track ZIP skip). Escaped
    // because they can embed album names.
    for warning in &summary.warnings {
        lines.push(format!("┣ ⚠️ {}", crate::html::escape(warning)));
    }
    lines.push(format!("┗ By: {mention}"));
    let details = lines.join("<br/>");

    let keyboard = if job.delivery_chat_id != job.chat_id {
        summary.first_delivered_msg_id.map(|msg_id| {
            let url = if state.bot_id > 0 {
                format!(
                    "tg://openmessage?user_id={}&message_id={msg_id}",
                    state.bot_id
                )
            } else {
                format!(
                    "https://t.me/{}",
                    state.bot_username.as_deref().unwrap_or("alac_bot")
                )
            };
            ferogram::keyboard::InlineKeyboard::new()
                .row([ferogram::keyboard::Button::url("View", url)])
                .into_markup()
        })
    } else {
        None
    };

    // ZIP jobs: the album preview photo + rich details caption was delivered
    // alongside the ZIP files. If the photo could not be delivered, fall back
    // to sending the rich album details as a text message (no webpage preview).
    if let Some(zip) = summary.zip_delivery.as_ref() {
        if !summary.is_cache_only && zip.total_parts > 0 && !zip.photo_delivered {
            let caption_meta = engine::orchestrator::caption::AlbumDetailsCaptionMetadata {
                album: &zip.album,
                artist: &zip.artist,
                album_id: &zip.album_id,
                storefront: &zip.storefront,
                total_tracks: zip.total_tracks,
                delivered_tracks: zip.delivered_tracks,
                size_bytes: zip.size_bytes,
                total_parts: zip.total_parts,
                release_year: &zip.release_year,
                genre: zip.genre.as_deref(),
                record_label: zip.record_label.as_deref(),
                is_partial: zip.is_partial,
                user_name: job.user_name.as_deref(),
                user_id: job.user_id,
                codec: zip.codec.as_deref(),
            };
            let details_html =
                engine::orchestrator::caption::format_album_details_caption(&caption_meta);
            let details_msg = ferogram::InputMessage::html(details_html).no_webpage(true);
            if let Err(error) = state
                .client
                .send_message(ferogram::PeerRef::from(job.delivery_chat_id), details_msg)
                .await
            {
                tracing::warn!(job_id = %job.id, error = %error, "ZIP details message failed");
            }
        }
    }

    let mut input = ferogram::InputMessage::html(details.clone())
        .no_webpage(true)
        .reply_to(reply_id);
    if let Some(k) = keyboard.clone() {
        input = input.reply_markup(k);
    }
    if let Err(error) = state.client.send_message(peer.clone(), input).await {
        tracing::warn!(job_id = %job.id, error = %error, "completion notification failed");
        // The command may have been deleted between the existence check and
        // the reply. Fall back to sending without reply_to so the user still gets
        // a completion notification.
        if reply_id.is_some() {
            let mut fb_msg = ferogram::InputMessage::html(details).no_webpage(true);
            if let Some(k) = keyboard {
                fb_msg = fb_msg.reply_markup(k);
            }
            let _ = state.client.send_message(peer, fb_msg).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use engine::orchestrator::types::JobPhase as EnginePhase;
    use tokio_util::sync::CancellationToken;

    use super::*;

    fn job(id: &str) -> ActiveRipJob {
        ActiveRipJob {
            id: id.into(),
            chat_id: 100,
            delivery_chat_id: 100,
            user_id: 7,
            user_name: Some("Alice".into()),
            job_header: "Album: <b>X</b>".into(),
            total_tracks: 2,
            status_msg_id: 55,
            controller: CancellationToken::new(),
            is_cancelled: false,
            cancelled_by: None,
            cached_count: 0,
            ripped_count: 0,
            failed_count: 0,
            completed: false,
            start_time_ms: 0,
            active_action_text: None,
            queue_position: None,
            phase: EnginePhase::Processing,
            terminal_state: None,
            skipped_count: 0,
            is_cache_only: false,
            is_group: false,
            reply_to_message_id: None,
        }
    }

    fn progress(job_id: &str) -> RipJobProgress {
        RipJobProgress {
            job_id: job_id.into(),
            total_tracks: 2,
            completed_tracks: 1,
            cached_count: 1,
            ripped_count: 0,
            failed_count: 0,
            skipped_count: 0,
            percent: 50,
            active_download_text: None,
            active_upload_text: None,
            activity_override: None,
        }
    }

    #[test]
    fn from_engine_clones_every_variant() {
        let job = job("job_1");
        assert!(matches!(
            BridgeEvent::from_engine(&OrchestratorEvent::Created(&job)),
            Some(BridgeEvent::Created { .. })
        ));
        assert!(matches!(
            BridgeEvent::from_engine(&OrchestratorEvent::Started(&job)),
            Some(BridgeEvent::Started { .. })
        ));
        let progress = progress("job_1");
        assert!(matches!(
            BridgeEvent::from_engine(&OrchestratorEvent::Progress(&job, &progress)),
            Some(BridgeEvent::Progress { .. })
        ));
        assert!(matches!(
            BridgeEvent::from_engine(&OrchestratorEvent::Failed(&job, "err")),
            Some(BridgeEvent::Failed { .. })
        ));
    }
    #[test]
    fn format_failed_track_rendering() {
        // 1. With title, artist, storefront
        let ft1 = FailedTrack {
            id: "6804576275".into(),
            error: "failed to get m3u8".into(),
            title: Some("Bhaber deshe thako konya".into()),
            artist: Some("Fakira".into()),
            storefront: Some("in".into()),
        };
        assert_eq!(
            format_failed_track(&ft1),
            r#"<a href="https://music.apple.com/in/song/6804576275">Bhaber deshe thako konya - Fakira</a> <i>(Unavailable on Apple Music)</i>"#
        );

        // 2. Without storefront
        let ft2 = FailedTrack {
            id: "6804576275".into(),
            error: "failed to get m3u8".into(),
            title: Some("Bhaber deshe thako konya".into()),
            artist: Some("Fakira".into()),
            storefront: None,
        };
        assert_eq!(
            format_failed_track(&ft2),
            r#"<a href="https://music.apple.com/song/6804576275">Bhaber deshe thako konya - Fakira</a> <i>(Unavailable on Apple Music)</i>"#
        );

        // 3. Only numeric ID
        let ft3 = FailedTrack {
            id: "12345".into(),
            error: "404".into(),
            title: None,
            artist: None,
            storefront: None,
        };
        assert_eq!(
            format_failed_track(&ft3),
            r#"<a href="https://music.apple.com/song/12345">Track 12345</a> <i>(Unavailable on Apple Music)</i>"#
        );

        // 4. Non-numeric ID (circuit breaker)
        let ft4 = FailedTrack {
            id: "Remaining tracks".into(),
            error: "offline".into(),
            title: None,
            artist: None,
            storefront: None,
        };
        assert_eq!(format_failed_track(&ft4), "Remaining tracks");

        // 5. Special chars escaped
        let ft5 = FailedTrack {
            id: "999".into(),
            error: "err".into(),
            title: Some("Tom & Jerry <Special>".into()),
            artist: Some("AC/DC & Friends".into()),
            storefront: None,
        };
        assert_eq!(
            format_failed_track(&ft5),
            r#"<a href="https://music.apple.com/song/999">Tom &amp; Jerry &lt;Special&gt; - AC/DC &amp; Friends</a>"#
        );
    }
}
