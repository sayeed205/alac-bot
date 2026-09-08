//! Engine job snapshot → dashboard view-model mapping.
//!
//! Pure functions only: no Telegram I/O, no orchestrator state. The event
//! bridge stores the last engine snapshot per job and every dashboard render
//! (open, refresh callback, `refresh_all`) goes through [`snapshot_from`].

use std::collections::HashMap;

use engine::orchestrator::types::{ActiveRipJob, JobPhase as EnginePhase, RipJobProgress};

use crate::dashboard::{DashboardJob, DashboardSnapshot, JobPhase};

/// Cached per-job rendering context. `ActiveRipJob` snapshots carry the live
/// counters, but `job_header`/`user_name` are finalized early; remembering
/// them keeps terminal rows (cancellation) renderable before removal.
#[derive(Debug, Default)]
pub struct JobContexts {
    jobs: HashMap<String, JobContext>,
}

#[derive(Debug, Clone)]
pub struct JobContext {
    /// Engine `job_header` post-resolution (rich HTML, already escaped).
    pub header: String,
    /// Requester display name (`user_name` from options).
    pub requester_name: String,
}

impl JobContexts {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    /// Remember a job's rendering context from its latest engine snapshot.
    pub fn remember(&mut self, job: &ActiveRipJob) {
        self.jobs.insert(
            job.id.clone(),
            JobContext {
                header: job.job_header.clone(),
                requester_name: job
                    .user_name
                    .clone()
                    .unwrap_or_else(|| format!("User {}", job.user_id)),
            },
        );
    }

    /// Drop contexts for finished jobs so the registry cannot grow unbounded.
    pub fn forget(&mut self, job_id: &str) {
        self.jobs.remove(job_id);
    }

    pub fn get(&self, job_id: &str) -> Option<&JobContext> {
        self.jobs.get(job_id)
    }

    /// Iterate remembered contexts (used by whole-snapshot builds).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &JobContext)> {
        self.jobs.iter()
    }

    /// Insert/replace one context (used by registry snapshots).
    pub fn insert(&mut self, job_id: String, context: JobContext) {
        self.jobs.insert(job_id, context);
    }
}

/// Map an engine phase to the dashboard's two-state view.
///
/// `Resolving`/`CheckingCache` render as `Processing` (an active-stage label):
/// the dashboard only distinguishes waiting-in-queue from active work.
pub fn phase_from(engine_phase: EnginePhase) -> JobPhase {
    match engine_phase {
        EnginePhase::Queued => JobPhase::Queued,
        _ => JobPhase::Processing,
    }
}

/// Percent for the dashboard row, clamped to 100.
pub fn percent_from(progress: &RipJobProgress) -> u8 {
    progress.percent.min(100) as u8
}

/// Map an engine job snapshot into a dashboard row for a specific viewer.
///
/// Cancel permission is `viewer == requester || viewer_is_admin` (oracle
/// `commands-rip.ts:323-432` authorization).
pub fn job_to_dashboard(
    job: &ActiveRipJob,
    context: &JobContext,
    viewer_id: i64,
    viewer_is_admin: bool,
) -> DashboardJob {
    let total = job.total_tracks as u64;
    let completed = job.cached_count + job.ripped_count + job.failed_count + job.skipped_count;
    let percent = if total > 0 {
        ((completed as f64 / total as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    } else {
        0
    };
    DashboardJob {
        id: job.id.clone(),
        requester_id: job.user_id,
        requester_name: context.requester_name.clone(),
        header: context.header.clone(),
        phase: phase_from(job.phase),
        queue_position: job.queue_position,
        cached: job.cached_count as u64,
        ripped: job.ripped_count as u64,
        failed: job.failed_count as u64,
        total,
        percent,
        is_cancel_allowed_for_viewer: viewer_is_admin || viewer_id == job.user_id,
    }
}

/// Build a whole dashboard snapshot from the engine's active jobs.
///
/// Jobs missing a remembered context (e.g. an engine restart missed by the
/// bridge) still render with a fallback header rather than disappearing.
pub fn snapshot_from(
    active: &[ActiveRipJob],
    contexts: &JobContexts,
    viewer_id: i64,
    viewer_is_admin: bool,
    ripping_mode: &str,
    mirror_health: Option<String>,
) -> DashboardSnapshot {
    let jobs = active
        .iter()
        .map(|job| {
            let context = contexts
                .get(&job.id)
                .cloned()
                .unwrap_or_else(|| JobContext {
                    header: job.job_header.clone(),
                    requester_name: job
                        .user_name
                        .clone()
                        .unwrap_or_else(|| format!("User {}", job.user_id)),
                });
            job_to_dashboard(job, &context, viewer_id, viewer_is_admin)
        })
        .collect();
    DashboardSnapshot {
        ripping_mode: ripping_mode.to_owned(),
        mirror_health,
        jobs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_job(
        phase: EnginePhase,
        queue_position: Option<u64>,
        user: i64,
        user_name: Option<&str>,
    ) -> ActiveRipJob {
        use tokio_util::sync::CancellationToken;
        ActiveRipJob {
            id: "job_1".into(),
            chat_id: 100,
            user_id: user,
            user_name: user_name.map(str::to_owned),
            job_header: "Album: <b>X</b> by <b>Y</b>".into(),
            total_tracks: 10,
            status_msg_id: 55,
            controller: CancellationToken::new(),
            is_cancelled: false,
            cancelled_by: None,
            cached_count: 2,
            ripped_count: 3,
            failed_count: 1,
            completed: false,
            start_time_ms: 0,
            active_action_text: None,
            queue_position,
            phase,
            terminal_state: None,
            skipped_count: 0,
            is_cache_only: false,
            is_group: false,
            reply_to_message_id: None,
        }
    }

    #[test]
    fn queued_jobs_map_to_waiting_rows_with_position() {
        let job = engine_job(EnginePhase::Queued, Some(3), 7, Some("Alice"));
        let ctx = JobContext {
            header: job.job_header.clone(),
            requester_name: "Alice".into(),
        };
        let row = job_to_dashboard(&job, &ctx, 7, false);
        assert_eq!(row.phase, JobPhase::Queued);
        assert_eq!(row.queue_position, Some(3));
        assert_eq!(row.requester_name, "Alice");
        assert_eq!(row.cached, 2);
        assert_eq!(row.ripped, 3);
        assert_eq!(row.failed, 1);
        assert_eq!(row.total, 10);
        assert_eq!(row.percent, 60); // (2+3+1+0)/10
    }

    #[test]
    fn resolving_and_processing_map_to_processing() {
        for phase in [
            EnginePhase::Resolving,
            EnginePhase::CheckingCache,
            EnginePhase::Processing,
        ] {
            let job = engine_job(phase, Some(0), 1, None);
            assert_eq!(phase_from(job.phase), JobPhase::Processing);
        }
        assert_eq!(phase_from(EnginePhase::Queued), JobPhase::Queued);
    }

    #[test]
    fn cancel_permission_is_requester_or_admin() {
        let ctx = JobContext {
            header: "h".into(),
            requester_name: "u".into(),
        };
        let job = engine_job(EnginePhase::Processing, None, 42, Some("Bob"));
        // Requester
        assert!(job_to_dashboard(&job, &ctx, 42, false).is_cancel_allowed_for_viewer);
        // Admin stranger
        assert!(job_to_dashboard(&job, &ctx, 99, true).is_cancel_allowed_for_viewer);
        // Unauthorized stranger
        assert!(!job_to_dashboard(&job, &ctx, 99, false).is_cancel_allowed_for_viewer);
    }

    #[test]
    fn snapshot_includes_all_active_jobs_with_mode_and_health() {
        let mut contexts = JobContexts::new();
        contexts.remember(&engine_job(EnginePhase::Processing, None, 1, Some("A")));
        let active = vec![engine_job(EnginePhase::Processing, None, 1, Some("A"))];
        let snapshot = snapshot_from(&active, &contexts, 1, false, "live", Some("healthy".into()));
        assert_eq!(snapshot.jobs.len(), 1);
        assert_eq!(snapshot.ripping_mode, "live");
        assert_eq!(snapshot.mirror_health.as_deref(), Some("healthy"));
        assert_eq!(snapshot.jobs[0].requester_name, "A");
    }

    #[test]
    fn percent_is_clamped_and_zero_total_renders_zero() {
        let progress = RipJobProgress {
            job_id: "j".into(),
            total_tracks: 0,
            completed_tracks: 0,
            cached_count: 0,
            ripped_count: 0,
            failed_count: 0,
            skipped_count: 0,
            percent: 0,
            active_download_text: None,
            active_upload_text: None,
            activity_override: None,
        };
        assert_eq!(percent_from(&progress), 0);
    }
}
