//! Async bridge between the engine's synchronous event callbacks and
//! Telegram/dashboard work.
//!
//! The orchestrator emits events from inside its pipeline; those callbacks
//! must not perform network I/O (they can run under the job mutex and would
//! stall the whole queue). The sync subscription only **clones** event data
//! into an unbounded mpsc channel; this module's spawned consumer does the
//! async rendering: per-request status messages + the global dashboard.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use engine::orchestrator::{
    deps::OrchestratorDeps,
    types::{ActiveRipJob, OrchestratorEvent, RipJobProgress, RipJobSummary},
};
use tokio::sync::mpsc;

use crate::{
    dashboard_manager,
    dashboard_map::{snapshot_from, JobContexts},
    handlers::rip::status::{
        cancelled_text, final_summary, render_progress, ProgressState, StatusEditor, StatusSink,
        SummaryInput, TelegramStatusSink,
    },
    BotState,
};

/// Owned copies of engine events, safe to move across an mpsc channel.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    Created {
        job: ActiveRipJob,
    },
    Progress {
        job: ActiveRipJob,
        progress: RipJobProgress,
    },
    Started {
        job: ActiveRipJob,
    },
    Completed {
        job: ActiveRipJob,
        summary: RipJobSummary,
    },
    Cancelled {
        job: ActiveRipJob,
        cancelled_by: Option<String>,
    },
    Failed {
        job: ActiveRipJob,
        error: String,
    },
}

impl BridgeEvent {
    /// Clone a borrowed engine event into an owned, sendable copy.
    pub fn from_engine(event: &OrchestratorEvent<'_>) -> Option<Self> {
        Some(match event {
            OrchestratorEvent::Created(job) => BridgeEvent::Created {
                job: (*job).clone(),
            },
            OrchestratorEvent::Started(job) => BridgeEvent::Started {
                job: (*job).clone(),
            },
            OrchestratorEvent::Progress(job, progress) => BridgeEvent::Progress {
                job: (*job).clone(),
                progress: (*progress).clone(),
            },
            OrchestratorEvent::Completed(job, summary) => BridgeEvent::Completed {
                job: (*job).clone(),
                summary: (*summary).clone(),
            },
            OrchestratorEvent::Cancelled(job, by) => BridgeEvent::Cancelled {
                job: (*job).clone(),
                cancelled_by: (*by).clone(),
            },
            OrchestratorEvent::Failed(job, error) => BridgeEvent::Failed {
                job: (*job).clone(),
                error: error.to_string(),
            },
        })
    }
}

/// Job registry shared by the bridge consumer and dashboard renders.
pub struct BridgeRegistry {
    /// Status editors keyed by job id (per-request status message lifecycle).
    editors: Mutex<HashMap<String, Arc<StatusEditor>>>,
    /// Rendering contexts keyed by job id (header/requester name).
    contexts: Mutex<JobContexts>,
}

impl BridgeRegistry {
    fn new() -> Self {
        Self {
            editors: Mutex::new(HashMap::new()),
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

    /// Drop a finished job's context and editor so neither registry grows
    /// unbounded.
    pub fn forget(&self, job_id: &str) {
        self.contexts
            .lock()
            .expect("bridge contexts poisoned")
            .forget(job_id);
        self.editors
            .lock()
            .expect("bridge editors poisoned")
            .remove(job_id);
    }

    fn editor(&self, job_id: &str) -> Option<Arc<StatusEditor>> {
        self.editors
            .lock()
            .expect("bridge editors poisoned")
            .get(job_id)
            .cloned()
    }

    fn insert_editor(&self, job_id: &str, editor: Arc<StatusEditor>) {
        self.editors
            .lock()
            .expect("bridge editors poisoned")
            .insert(job_id.to_owned(), editor);
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
async fn consume(state: Arc<BotState>, mut rx: mpsc::UnboundedReceiver<BridgeEvent>) {
    while let Some(event) = rx.recv().await {
        if let Err(error) = handle_event(Arc::clone(&state), event).await {
            tracing::warn!(%error, "status event handling failed");
        }
    }
}

/// Subscribe the bridge to orchestrator events and spawn its consumer.
pub fn start(state: Arc<BotState>) {
    let (tx, rx) = mpsc::unbounded_channel();

    // Sync subscriber: clone event data only, never await.
    state.rip_orchestrator.subscribe(Arc::new(move |event| {
        if let Some(owned) = BridgeEvent::from_engine(event) {
            let _ = tx.send(owned);
        }
    }));

    tokio::spawn(consume(state, rx));
}

/// Build a global dashboard snapshot from the engine's current jobs.
pub async fn current_snapshot(state: &BotState) -> crate::dashboard::DashboardSnapshot {
    let active = state.rip_orchestrator.get_active_jobs();
    let mode = state
        .rip_deps
        .get_settings()
        .await
        .ripping_mode
        .as_str()
        .to_owned();
    // Per-viewer permissions are applied by the dashboard manager per entry;
    // the global snapshot stays viewer-neutral.
    snapshot_from(
        &active,
        &registry().contexts_snapshot(),
        0,
        false,
        &mode,
        None,
    )
}

/// Single consumer turn: update per-request status message, refresh dashboard.
async fn handle_event(state: Arc<BotState>, event: BridgeEvent) -> Result<(), String> {
    let state_ref = state.as_ref();
    match event {
        BridgeEvent::Created { job } => {
            // Register the status editor around the command handler's
            // initial "Resolving..." message immediately: every later edit
            // (progress or terminal) goes through this single editor.
            registry().remember(&job);
            let sink: Arc<dyn StatusSink> = Arc::new(TelegramStatusSink {
                client: state_ref.client.clone(),
                peer: ferogram::PeerRef::from(job.chat_id),
            });
            registry().insert_editor(
                &job.id,
                Arc::new(StatusEditor::new(
                    sink,
                    job.status_msg_id as i32,
                    job.id.clone(),
                )),
            );
        }
        BridgeEvent::Progress { job, progress } => {
            registry().remember(&job);
            update_status_message(state_ref, &job, &progress).await?;
            refresh_dashboard(state_ref, false).await;
        }
        BridgeEvent::Started { job } => {
            registry().remember(&job);
            refresh_dashboard(state_ref, false).await;
        }
        BridgeEvent::Completed { job, summary } => {
            registry().remember(&job);
            let text = final_summary(&SummaryInput {
                target: summary.job_header.clone(),
                total: summary.total_tracks,
                cached: summary.cached_count,
                ripped: summary.ripped_count,
                skipped: summary.skipped_uncached_tracks.len(),
                failed: summary
                    .failed_tracks
                    .iter()
                    .map(|t| (t.id.clone(), t.error.clone()))
                    .collect(),
                elapsed: summary.total_elapsed_sec.clone(),
                cache_only: summary.is_cache_only,
                group: summary.is_group,
                capped: summary.capped_count,
                cap_limit: summary.max_collection_limit,
            });
            finish_status(&job, text).await?;
            registry().forget(&job.id);
            refresh_dashboard(state_ref, true).await;
        }
        BridgeEvent::Cancelled { job, cancelled_by } => {
            registry().remember(&job);
            let text = cancelled_text(
                &job.job_header,
                cancelled_by.as_deref().unwrap_or("User"),
                job.cached_count + job.ripped_count,
                job.total_tracks,
            );
            finish_status(&job, text).await?;
            registry().forget(&job.id);
            refresh_dashboard(state_ref, true).await;
        }
        BridgeEvent::Failed { job, error } => {
            registry().remember(&job);
            // Oracle commands-rip.ts:662-687 renders resolution failures with
            // their details in a code block; other failures are generic.
            let text = if error.starts_with("Failed to resolve any tracks") {
                format!(
                    "⚠️ <b>Failed to resolve any tracks:</b><br/><code>{}</code>",
                    crate::html::escape(&error)
                )
            } else {
                format!(
                    "⚠️ <b>Download Failed:</b><br/><br/><code>{}</code>",
                    crate::html::escape(&error)
                )
            };
            finish_status(&job, text).await?;
            registry().forget(&job.id);
            refresh_dashboard(state_ref, true).await;
        }
    }
    Ok(())
}

/// Write a terminal status edit; the editor falls back to sending a fresh
/// message when the original cannot be edited.
async fn finish_status(job: &ActiveRipJob, text: String) -> Result<(), String> {
    let Some(editor) = registry().editor(&job.id) else {
        return Ok(());
    };
    editor.final_text(text).await;
    Ok(())
}

/// Lazily create the status editor around the initial resolving message and
/// edit it with the latest progress render.
async fn update_status_message(
    state: &BotState,
    job: &ActiveRipJob,
    progress: &RipJobProgress,
) -> Result<(), String> {
    let registry = registry();
    if registry.editor(&job.id).is_none() {
        let sink: Arc<dyn StatusSink> = Arc::new(TelegramStatusSink {
            client: state.client.clone(),
            peer: ferogram::PeerRef::from(job.chat_id),
        });
        registry.insert_editor(
            &job.id,
            Arc::new(StatusEditor::new(
                sink,
                job.status_msg_id as i32,
                job.id.clone(),
            )),
        );
    }
    let Some(editor) = registry.editor(&job.id) else {
        return Ok(());
    };
    let text = render_progress(
        &ProgressState {
            header: job.job_header.clone(),
            total: progress.total_tracks,
            cached: progress.cached_count,
            ripped: progress.ripped_count,
            failed: progress.failed_count,
            skipped: progress.skipped_count,
            cache_only: job.is_cache_only,
            group: job.is_group,
            active_download: progress.active_download_text.clone(),
            active_upload: progress.active_upload_text.clone(),
        },
        progress.activity_override.as_deref(),
    );
    editor.update(text, false, false).await;
    Ok(())
}

/// Refresh every open dashboard; `force` bypasses the coalescing window for
/// terminal/queue-shape changes.
async fn refresh_dashboard(state: &BotState, force: bool) {
    let snapshot = current_snapshot(state).await;
    dashboard_manager().refresh(snapshot, force).await;
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
}
