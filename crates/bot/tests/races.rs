//! M5c hardening tests: the bot-level cancel-vs-complete race and mirror
//! health mapping.

use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use engine::{
    orchestrator::{
        deps::{CachedTrack, OrchestratorDeps, RequestLog, SaveTrackInput},
        types::{JobPhase, OrchestratorEvent, RipJobOptions},
        RipOrchestrator,
    },
    playlist::{PlaylistData, PlaylistError},
    settings::BotSettings,
    types::{AlbumTracks, ArtistTracks, ParsedTargetItem, TargetKind},
};

/// Fake deps whose rip can be held mid-flight from the test.
#[derive(Clone)]
struct RaceDeps {
    settings: BotSettings,
    rip_hold: Arc<tokio::sync::Mutex<bool>>,
    rip_calls: Arc<AtomicUsize>,
}

impl OrchestratorDeps for RaceDeps {
    fn get_settings(&self) -> impl Future<Output = BotSettings> + Send {
        let settings = self.settings.clone();
        async move { settings }
    }
    async fn find_cached_tracks(
        &self,
        _ids: &[String],
    ) -> Result<HashMap<String, CachedTrack>, String> {
        Ok(HashMap::new())
    }
    async fn save_track(&self, _input: SaveTrackInput) -> Result<(), String> {
        Ok(())
    }
    async fn delete_track(&self, _id: &str) -> Result<bool, String> {
        Ok(true)
    }
    async fn log_request(&self, _log: RequestLog) -> Result<(), String> {
        Ok(())
    }
    async fn fetch_album_tracks(
        &self,
        _id: &str,
        _storefront: &str,
    ) -> Result<AlbumTracks, String> {
        Err("no albums".to_owned())
    }
    async fn fetch_artist_tracks(
        &self,
        _id: &str,
        _storefront: &str,
    ) -> Result<ArtistTracks, String> {
        Err("no artists".to_owned())
    }
    async fn fetch_playlist_tracks(
        &self,
        _id: &str,
        _storefront: &str,
    ) -> Result<PlaylistData, PlaylistError> {
        Err(PlaylistError::TimedOut {
            elapsed_ms: 1,
            message: "no playlists".to_owned(),
        })
    }
    async fn rip(
        &self,
        _id: &str,
        _progress: Option<&engine::ripper::RipProgressCallback>,
        _storefront: &str,
        _signal: tokio_util::sync::CancellationToken,
        _temp_dir: Option<&std::path::Path>,
    ) -> Result<engine::types::TrackRipResult, engine::ripper::RipError> {
        self.rip_calls.fetch_add(1, Ordering::SeqCst);
        // Hold until the test flips the gate or the safety deadline passes.
        // The signal is deliberately NOT observed: the race under test is
        // what the engine does when the pipeline settles AFTER a cancel.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while *self.rip_hold.lock().await && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Err(engine::ripper::RipError::Message(
            "Mirror service is currently offline".to_owned(),
        ))
    }
    fn sink(&self) -> &dyn engine::orchestrator::deps::TelegramSink {
        unreachable!("sink not reached in race tests")
    }
}

fn race_options() -> RipJobOptions {
    RipJobOptions {
        chat_id: 100,
        user_id: 42,
        user_name: Some("tester".to_owned()),
        delivery_chat_id: 100,
        is_group: false,
        is_force: false,
        is_cache_only: false,
        single_storefront: None,
        parsed_items: vec![ParsedTargetItem {
            id: "track1".to_owned(),
            kind: TargetKind::Track,
            storefront: None,
        }],
        reply_to_message_id: Some(555),
        status_msg_id: 999,
        is_admin: true,
    }
}

/// Collect terminal events across the whole test.
fn terminal_recorder(orch: &RipOrchestrator) -> Arc<Mutex<Vec<(&'static str, String)>>> {
    let events: Arc<Mutex<Vec<(&'static str, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    orch.subscribe(Arc::new(move |event| {
        let entry = match event {
            OrchestratorEvent::Completed(job, _) => ("completed", job.id.clone()),
            OrchestratorEvent::Cancelled(job, _) => ("cancelled", job.id.clone()),
            OrchestratorEvent::Failed(job, _) => ("failed", job.id.clone()),
            _ => return,
        };
        sink.lock().unwrap().push(entry);
    }));
    events
}

/// Wait until the job reaches the given phase.
async fn wait_for_phase(
    orch: &RipOrchestrator,
    phase: JobPhase,
) -> engine::orchestrator::types::ActiveRipJob {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(job) = orch
            .get_active_jobs()
            .into_iter()
            .find(|job| job.phase == phase)
        {
            return job;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "job never reached phase {phase:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_active_rip_emits_exactly_one_cancelled_terminal() {
    let orch = Arc::new(RipOrchestrator::new());
    let terminals = terminal_recorder(&orch);
    let deps = Arc::new(RaceDeps {
        settings: engine::settings::default_settings(),
        rip_hold: Arc::new(tokio::sync::Mutex::new(true)),
        rip_calls: Arc::new(AtomicUsize::new(0)),
    });

    let run_orch = Arc::clone(&orch);
    let run_deps = Arc::clone(&deps);
    let options = race_options();
    let task = tokio::spawn(async move { run_orch.start_job(run_deps, &options).await });

    let job = wait_for_phase(&orch, JobPhase::Processing).await;
    assert!(orch.cancel_job(&job.id, Some("tester")));
    // Let the held rip settle AFTER the cancellation — the race condition.
    // The queued task's result is intentionally not asserted: the terminal
    // event stream is the contract under test.
    *deps.rip_hold.lock().await = false;
    let _ = task.await.unwrap();

    // Exactly one terminal event, and it is the cancellation: the late
    // pipeline error cannot manufacture a second terminal.
    let terminals = terminals.lock().unwrap();
    assert_eq!(terminals.len(), 1, "exactly one terminal event");
    assert_eq!(terminals[0].0, "cancelled");
    assert!(orch.get_active_jobs().is_empty(), "job removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settled_failed_job_is_no_longer_cancellable() {
    let orch = Arc::new(RipOrchestrator::new());
    let terminals = terminal_recorder(&orch);
    let deps = Arc::new(RaceDeps {
        settings: engine::settings::default_settings(),
        rip_hold: Arc::new(tokio::sync::Mutex::new(false)),
        rip_calls: Arc::new(AtomicUsize::new(0)),
    });

    let run_orch = Arc::clone(&orch);
    let run_deps = Arc::clone(&deps);
    let options = race_options();
    let task = tokio::spawn(async move { run_orch.start_job(run_deps, &options).await });
    let result = task.await.unwrap();

    // The fake rip fails with a mirror-offline error, but the Rust deviation
    // records it as a failed track and completes the job with a summary —
    // exactly one terminal event, and it is Completed (not Failed).
    let terminals = terminals.lock().unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].0, "completed");
    assert!(result.is_ok(), "recorded failure + continue settles Ok");
    // The failed track + the breaker's synthetic "Remaining tracks" row.
    assert_eq!(result.as_ref().unwrap().failed_count, 2);
    // Late cancel is a no-op on a settled job.
    assert!(!orch.cancel_job(&terminals[0].1, Some("late")));
}

/// The event bridge skips progress edits while total_tracks == 0 (the
/// oracle keeps the "Resolving..." message untouched until the tracklist is
/// known). This pins the guard's field shape.
#[test]
fn unresolved_progress_has_zero_total_tracks() {
    let progress = engine::orchestrator::types::RipJobProgress {
        job_id: "j".to_owned(),
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
    assert_eq!(progress.total_tracks, 0);
}

/// Mirror health cache matches the oracle's last-known-only contract.
#[test]
fn mirror_health_labels_match_oracle() {
    use bot::mirror_health::{HealthReport, LastKnownHealth, MirrorHealth};

    let health = LastKnownHealth::new();
    assert_eq!(health.label(), None, "unknown before first probe");
    health.record(HealthReport {
        health: MirrorHealth::Online,
        latency_ms: 42,
    });
    assert_eq!(health.label(), Some("Online"));
    health.record(HealthReport {
        health: MirrorHealth::Unreachable,
        latency_ms: 4000,
    });
    assert_eq!(health.label(), Some("Unreachable"));
    health.record(HealthReport {
        health: MirrorHealth::Unavailable,
        latency_ms: 4000,
    });
    assert_eq!(health.label(), Some("Unavailable"));
    assert_eq!(MirrorHealth::NotConfigured.label(), "Not configured");
}
