//! Rip orchestrator (port of `src/modules/alac/orchestrator/rip-orchestrator.ts`).
//!
//! Owns job bookkeeping, resolves parsed items to tracks, serves the cache
//! fast-path, and runs the producer → downloader → uploader pipeline over
//! bounded channels through the sequential rip queue.

pub mod caption;
pub mod deps;
pub mod types;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio_util::sync::CancellationToken;

use crate::{
    orchestrator::{
        caption::{format_dump_caption, html_escape, DumpCaptionMetadata},
        deps::{DumpUpload, OrchestratorDeps, RequestLog, SaveTrackInput, UploadProgressCallback},
        types::{
            ActiveRipJob, EventCallback, FailedTrack, OrchestratorEvent, RipJobOptions,
            RipJobProgress, RipJobSummary,
        },
    },
    progress::format_byte_progress,
    queue::SequentialRipQueue,
    ripper::RipProgressCallback,
    settings::BotSettings,
    types::{AlbumTracks, ArtistTracks, TargetKind, TrackRipResult},
};

/// All orchestrator failures surface as messages (TS `new Error(msg)`).
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("RipOrchestrator dependencies not configured. Call setDependencies() first.")]
    DependenciesNotSet,
    #[error("{0}")]
    Message(String),
}

impl From<crate::queue::QueueError> for OrchestratorError {
    fn from(e: crate::queue::QueueError) -> Self {
        OrchestratorError::Message(e.to_string())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// TS `ResolvedTrackItem`.
#[derive(Debug, Clone)]
struct ResolvedTrackItem {
    id: String,
    title: Option<String>,
    artist: Option<String>,
    storefront: Option<String>,
}

/// TS `PipelineItem`.
struct PipelineItem {
    track_id: String,
    storefront: Option<String>,
    meta_title: Option<String>,
    meta_artist: Option<String>,
}

/// TS `PipelineRipResult`.
struct PipelineRipResult {
    track_id: String,
    rip_result: TrackRipResult,
    start_time_ms: u64,
}

/// Job state shared by the orchestrator and the pipeline stages (the TS job
/// object mutated by reference from several concurrent stages).
struct JobShared {
    job: ActiveRipJob,
}

/// The `activeDownloadText`/`activeUploadText` variables the TS pipeline
/// stages share through closure capture.
#[derive(Default)]
struct PipelineTexts {
    download: Mutex<Option<String>>,
    upload: Mutex<Option<String>>,
}

/// Event registry shared by the orchestrator and its pipelines.
#[derive(Clone)]
struct EventBus {
    subscribers: Arc<Mutex<Vec<EventCallback>>>,
}

impl EventBus {
    fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn emit(&self, event: &OrchestratorEvent<'_>) {
        for cb in self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .iter()
        {
            cb(event);
        }
    }

    /// TS `emitProgress(activityOverride?, activeDownloadText?,
    /// activeUploadText?)` — snapshot counters, emit `job:progress`.
    fn emit_progress(
        &self,
        shared: &Arc<Mutex<JobShared>>,
        activity_override: Option<&str>,
        active_download: Option<&str>,
        active_upload: Option<&str>,
    ) {
        let (job, progress) = {
            let guard = shared.lock().expect("job poisoned");
            let completed_tracks =
                guard.job.cached_count + guard.job.ripped_count + guard.job.failed_count;
            let percent = if guard.job.total_tracks > 0 {
                ((completed_tracks as f64 / guard.job.total_tracks as f64) * 100.0).round() as u32
            } else {
                0
            };
            let progress = RipJobProgress {
                job_id: guard.job.id.clone(),
                total_tracks: guard.job.total_tracks,
                completed_tracks,
                cached_count: guard.job.cached_count,
                ripped_count: guard.job.ripped_count,
                failed_count: guard.job.failed_count,
                skipped_count: 0,
                percent,
                active_download_text: active_download.map(str::to_string),
                active_upload_text: active_upload.map(str::to_string),
                activity_override: activity_override.map(str::to_string),
            };
            (guard.job.clone(), progress)
        };
        self.emit(&OrchestratorEvent::Progress(&job, &progress));
    }
}

/// The orchestrator: subscriber registry + job table + the rip queue.
pub struct RipOrchestrator {
    bus: EventBus,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<JobShared>>>>>,
    queue: SequentialRipQueue,
}

impl Default for RipOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl RipOrchestrator {
    pub fn new() -> Self {
        Self {
            bus: EventBus::new(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            queue: SequentialRipQueue::new(),
        }
    }

    /// Subscribe to every orchestrator event.
    pub fn subscribe(&self, callback: EventCallback) {
        self.bus
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .push(callback);
    }

    /// TS `getActiveJobs`.
    pub fn get_active_jobs(&self) -> Vec<ActiveRipJob> {
        self.jobs
            .lock()
            .expect("jobs poisoned")
            .values()
            .filter_map(|shared| {
                let job = &shared.lock().expect("job poisoned").job;
                (!job.completed).then(|| job.clone())
            })
            .collect()
    }

    /// TS `getJob`.
    pub fn get_job(&self, id: &str) -> Option<ActiveRipJob> {
        self.jobs
            .lock()
            .expect("jobs poisoned")
            .get(id)
            .map(|shared| shared.lock().expect("job poisoned").job.clone())
    }

    /// TS `cancelJob` — false when missing / already cancelled / completed.
    pub fn cancel_job(&self, id: &str, cancelled_by: Option<&str>) -> bool {
        let Some(shared) = self.jobs.lock().expect("jobs poisoned").get(id).cloned() else {
            return false;
        };

        let cancelled_by = cancelled_by.map(str::to_string);
        let mut guard = shared.lock().expect("job poisoned");
        if guard.job.is_cancelled || guard.job.completed {
            return false;
        }
        guard.job.is_cancelled = true;
        guard.job.cancelled_by = cancelled_by;
        guard.job.controller.cancel();
        let event_job = guard.job.clone();
        let by = guard.job.cancelled_by.clone();
        drop(guard);

        self.jobs.lock().expect("jobs poisoned").remove(id);
        self.bus
            .emit(&OrchestratorEvent::Cancelled(&event_job, &by));
        true
    }

    /// TS `startJob` — the whole rip flow. Deps arrive per call (the TS
    /// `depsOverride` pattern).
    pub async fn start_job<D: OrchestratorDeps>(
        &self,
        deps: Arc<D>,
        options: &RipJobOptions,
    ) -> Result<RipJobSummary, OrchestratorError> {
        // Step 2-3: settings snapshot + maintenance gate (before any job
        // registration — a gate failure emits no events).
        let settings = deps.get_settings().await;
        if !settings.can_rip_live(options.is_admin) && !options.is_cache_only {
            return Err(OrchestratorError::Message(
                "Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now."
                    .to_string(),
            ));
        }

        let job_controller = CancellationToken::new();
        let job_id = format!("job_{}_{}", now_ms(), random36(4));

        // Step 5: initial job header from the parsed targets.
        let mut job_header = "Apple Music Lossless Rip".to_string();
        if options.parsed_items.len() == 1 {
            let it = &options.parsed_items[0];
            job_header = match it.kind {
                TargetKind::Album => format!("Album {}", it.id),
                TargetKind::Playlist => format!("Playlist {}", it.id),
                TargetKind::Artist => format!("Artist {}", it.id),
                TargetKind::Track => format!("Track {}", it.id),
            };
        } else if options.parsed_items.len() > 1 {
            job_header = format!("Batch ({} links)", options.parsed_items.len());
        }

        let shared: Arc<Mutex<JobShared>> = Arc::new(Mutex::new(JobShared {
            job: ActiveRipJob {
                id: job_id.clone(),
                chat_id: options.chat_id,
                user_id: options.user_id,
                user_name: options.user_name.clone(),
                job_header,
                total_tracks: 0,
                status_msg_id: options.status_msg_id,
                controller: job_controller.clone(),
                is_cancelled: false,
                cancelled_by: None,
                cached_count: 0,
                ripped_count: 0,
                failed_count: 0,
                completed: false,
                start_time_ms: now_ms(),
                active_action_text: None,
                queue_position: None,
            },
        }));
        self.jobs
            .lock()
            .expect("jobs poisoned")
            .insert(job_id.clone(), Arc::clone(&shared));
        {
            let guard = shared.lock().expect("job poisoned");
            self.bus.emit(&OrchestratorEvent::Created(&guard.job));
        }

        let result = self
            .run_job(
                Arc::clone(&deps),
                options,
                Arc::clone(&shared),
                job_controller,
                settings,
            )
            .await;

        match &result {
            Ok(summary) => {
                let job = {
                    let mut guard = shared.lock().expect("job poisoned");
                    guard.job.completed = true;
                    guard.job.clone()
                };
                self.bus.emit(&OrchestratorEvent::Completed(&job, summary));
            }
            Err(err) => {
                let job = {
                    let mut guard = shared.lock().expect("job poisoned");
                    guard.job.completed = true;
                    guard.job.clone()
                };
                let msg = err.to_string();
                self.bus.emit(&OrchestratorEvent::Failed(&job, &msg));
            }
        }
        self.jobs.lock().expect("jobs poisoned").remove(&job_id);
        result
    }

    /// Steps 8-18 of the TS `startJob` flow.
    async fn run_job<D: OrchestratorDeps>(
        &self,
        deps: Arc<D>,
        options: &RipJobOptions,
        shared: Arc<Mutex<JobShared>>,
        job_controller: CancellationToken,
        settings: BotSettings,
    ) -> Result<RipJobSummary, OrchestratorError> {
        self.bus.emit_progress(
            &shared,
            Some("Resolving metadata & tracklist..."),
            None,
            None,
        );

        // Step 8: resolve every parsed item.
        let mut resolved_tracks: Vec<ResolvedTrackItem> = Vec::new();
        let mut album_name: Option<String> = None;
        let mut album_artist: Option<String> = None;

        for item in &options.parsed_items {
            if job_controller.is_cancelled() {
                return Err(OrchestratorError::Message(
                    "Download was cancelled".to_string(),
                ));
            }

            let effective_sf = item
                .storefront
                .clone()
                .or_else(|| options.single_storefront.clone())
                .unwrap_or_else(|| "us".to_string());

            let resolution: Result<(), String> = match item.kind {
                TargetKind::Track => {
                    resolved_tracks.push(ResolvedTrackItem {
                        id: item.id.clone(),
                        title: None,
                        artist: None,
                        storefront: Some(effective_sf.clone()),
                    });
                    Ok(())
                }
                TargetKind::Album => match deps.fetch_album_tracks(&item.id, &effective_sf).await {
                    Ok(AlbumTracks { album, tracks }) => {
                        album_name = Some(album.album.clone());
                        album_artist = Some(album.artist.clone());
                        for t in tracks {
                            resolved_tracks.push(ResolvedTrackItem {
                                id: t.id.clone(),
                                title: Some(t.title.clone()),
                                artist: Some(t.artist.clone()),
                                storefront: Some(effective_sf.clone()),
                            });
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
                TargetKind::Artist => {
                    match deps.fetch_artist_tracks(&item.id, &effective_sf).await {
                        Ok(ArtistTracks {
                            artist_name,
                            tracks,
                            ..
                        }) => {
                            album_artist = Some(artist_name.clone());
                            for t in tracks {
                                resolved_tracks.push(ResolvedTrackItem {
                                    id: t.id.clone(),
                                    title: Some(t.title.clone()),
                                    artist: Some(t.artist.clone()),
                                    storefront: Some(effective_sf.clone()),
                                });
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                TargetKind::Playlist => {
                    match deps.fetch_playlist_tracks(&item.id, &effective_sf).await {
                        Ok(data) => {
                            for t in data.tracks {
                                resolved_tracks.push(ResolvedTrackItem {
                                    id: t.id.clone(),
                                    title: Some(t.title.clone()),
                                    artist: Some(t.artist.clone()),
                                    storefront: Some(effective_sf.clone()),
                                });
                            }
                            Ok(())
                        }
                        Err(e) => Err(e.to_string()),
                    }
                }
            };

            if let Err(err_msg) = resolution {
                tracing::error!(
                    kind = kind_str(item.kind),
                    id = %item.id,
                    error = %err_msg,
                    "Failed to resolve target item"
                );
                return Err(OrchestratorError::Message(format!(
                    "{} {}: {}",
                    kind_str(item.kind),
                    item.id,
                    err_msg
                )));
            }
        }

        if resolved_tracks.is_empty() {
            return Err(OrchestratorError::Message(
                "No valid tracks found to process.".to_string(),
            ));
        }

        // Step 10: dedup preserving order.
        let mut seen_ids: HashSet<String> = HashSet::new();
        let unique_tracks: Vec<ResolvedTrackItem> = resolved_tracks
            .into_iter()
            .filter(|t| seen_ids.insert(t.id.clone()))
            .collect();

        // Step 11: cap collections for non-admins.
        let mut capped_count = 0usize;
        let max_collection_limit = settings.max_collection_tracks;
        let mut tracks_to_process = unique_tracks;
        if !options.is_admin
            && max_collection_limit > 0
            && tracks_to_process.len() > max_collection_limit as usize
        {
            capped_count = tracks_to_process.len() - max_collection_limit as usize;
            let original = tracks_to_process.len();
            tracks_to_process.truncate(max_collection_limit as usize);
            tracing::warn!(
                user_id = options.user_id,
                original,
                capped = max_collection_limit,
                "Collection capped for non-admin user"
            );
        }

        // Step 12: job header refinement + total. TS uses truthiness —
        // empty strings count as absent.
        fn non_empty(s: &Option<String>) -> Option<&str> {
            s.as_deref().filter(|s| !s.is_empty())
        }
        let header = match (non_empty(&album_name), non_empty(&album_artist)) {
            (Some(name), Some(artist)) => format!("{artist} - {name}"),
            _ if tracks_to_process.len() == 1 => {
                let first = &tracks_to_process[0];
                match (non_empty(&first.title), non_empty(&first.artist)) {
                    (Some(title), Some(artist)) => format!("{artist} - {title}"),
                    _ => format!("Track {}", first.id),
                }
            }
            _ => format!("Batch ({} tracks)", tracks_to_process.len()),
        };
        {
            let mut guard = shared.lock().expect("job poisoned");
            guard.job.job_header = header;
            guard.job.total_tracks = tracks_to_process.len();
        }

        // Step 13: cache lookup (DB failure fails the job, TS parity).
        let requested_ids: Vec<String> = tracks_to_process.iter().map(|t| t.id.clone()).collect();
        let mut existing_tracks_map = deps
            .find_cached_tracks(&requested_ids)
            .await
            .map_err(OrchestratorError::Message)?;

        // Step 14: force + admin purge.
        if options.is_force && options.is_admin {
            let mut old_message_ids: Vec<i64> = Vec::new();
            for item in &tracks_to_process {
                if let Some(cached) = existing_tracks_map.remove(&item.id) {
                    old_message_ids.push(cached.message_id);
                    // TS: deleteTrack per item, errors swallowed.
                    let _ = deps.delete_track(&item.id).await;
                }
            }
            if !old_message_ids.is_empty() {
                tracing::debug!(
                    count = old_message_ids.len(),
                    "Deleting old dump messages on force re-rip prior to queue"
                );
                // TS: tg.deleteMessagesById errors swallowed.
                let _ = deps.sink().delete_dump_messages(&old_message_ids).await;
            }
        }

        // Step 15: pre-queue cache handling.
        let mut uncached_items: Vec<ResolvedTrackItem> = Vec::new();
        let mut cached_count = 0usize;
        let is_multi_track = tracks_to_process.len() > 1;

        for item in &tracks_to_process {
            if job_controller.is_cancelled() {
                return Err(OrchestratorError::Message(
                    "Download was cancelled".to_string(),
                ));
            }

            let Some(cached) = existing_tracks_map.get(&item.id) else {
                uncached_items.push(item.clone());
                continue;
            };

            if options.is_cache_only {
                cached_count += 1;
                shared.lock().expect("job poisoned").job.cached_count = cached_count;
                tracing::info!(track_id = %item.id, "Track already cached in dump channel");
                self.bus
                    .emit_progress(&shared, Some("Recognized cached tracks..."), None, None);
            } else {
                let reply_to = (options.delivery_chat_id == options.chat_id)
                    .then_some(options.reply_to_message_id)
                    .flatten();
                // TS: sendDumpCopy + logRequest share one try/catch — any
                // failure marks the track for re-rip.
                let outcome = async {
                    deps.sink()
                        .send_dump_copy(
                            options.delivery_chat_id,
                            cached.message_id,
                            reply_to,
                            is_multi_track,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    deps.log_request(RequestLog {
                        telegram_id: options.user_id,
                        chat_id: options.chat_id,
                        apple_track_id: item.id.clone(),
                        is_cache_hit: true,
                        duration_ms: Some(0),
                        status: "completed".to_string(),
                        error_reason: None,
                    })
                    .await
                    .map_err(|e| e.to_string())
                }
                .await;
                match outcome {
                    Ok(()) => {
                        tracing::info!(track_id = %item.id, time = "0ms", "Cache hit: delivered");
                        cached_count += 1;
                        shared.lock().expect("job poisoned").job.cached_count = cached_count;
                        self.bus.emit_progress(
                            &shared,
                            Some("Delivered cached tracks..."),
                            None,
                            None,
                        );
                    }
                    Err(err) => {
                        tracing::error!(
                            track_id = %item.id,
                            error = %err,
                            "Failed to deliver cached track copy, marking for re-rip"
                        );
                        uncached_items.push(item.clone());
                    }
                }
            }
        }

        let summary = |cached_count: usize,
                       ripped_count: usize,
                       failed: Vec<FailedTrack>,
                       skipped: Vec<String>,
                       elapsed: &str| {
            let guard = shared.lock().expect("job poisoned");
            RipJobSummary {
                job_id: guard.job.id.clone(),
                job_header: guard.job.job_header.clone(),
                total_tracks: guard.job.total_tracks,
                cached_count,
                ripped_count,
                failed_count: failed.len(),
                failed_tracks: failed,
                skipped_uncached_tracks: skipped,
                total_elapsed_sec: elapsed.to_string(),
                capped_count,
                max_collection_limit,
                is_cache_only: options.is_cache_only,
                is_group: options.is_group,
            }
        };

        // Step 16: all-cached fast path.
        if uncached_items.is_empty() {
            return Ok(summary(cached_count, 0, Vec::new(), Vec::new(), "0.0"));
        }

        // Step 17: cache-only mode with uncached leftovers (live off;
        // unreachable in practice after the step-3 gate — defensive port).
        if !settings.can_rip_live(options.is_admin)
            && !options.is_cache_only
            && !uncached_items.is_empty()
        {
            let skipped: Vec<String> = uncached_items.iter().map(|i| i.id.clone()).collect();
            return Ok(summary(cached_count, 0, Vec::new(), skipped, "0.0"));
        }

        // Step 18: live rip through the queue.
        self.bus
            .emit_progress(&shared, Some("Queued for ripping..."), None, None);

        let job_id = shared.lock().expect("job poisoned").job.id.clone();
        tracing::info!(
            job_id = %job_id,
            tracks_count = uncached_items.len(),
            force = options.is_force,
            is_group = options.is_group,
            is_cache_only = options.is_cache_only,
            delivery_chat_id = options.delivery_chat_id,
            "Rip job queued"
        );

        let queue_start_time = now_ms();

        // The queue task must be 'static — move everything it needs.
        let task_deps = Arc::clone(&deps);
        let task_shared = Arc::clone(&shared);
        let task_options = options.clone();
        let task_items = uncached_items;
        let task_controller = job_controller.clone();
        let task_bus = self.bus.clone();
        let task_cached_count = cached_count;
        let task_is_multi_track = is_multi_track;

        let task = move |_queue_signal: CancellationToken| {
            Box::pin(async move {
                run_pipeline(
                    task_deps,
                    task_bus,
                    task_shared,
                    &task_options,
                    &task_items,
                    task_controller,
                    task_cached_count,
                    task_is_multi_track,
                    max_collection_limit,
                    capped_count,
                    queue_start_time,
                )
                .await
            })
                as std::pin::Pin<Box<dyn std::future::Future<Output = RipJobSummary> + Send>>
        };

        // TS calls queue.enqueue(task) with no options (no position
        // callbacks, no signal) — cancellation is handled inside the task.
        let result = self.queue.enqueue(task, None).await?;

        Ok(result)
    }
}

/// The in-queue pipeline (producer → downloader → uploader over bounded
/// mpsc channels) — the body of the TS `queue.enqueue` task.
///
/// Upload-retries-exhausted fails the whole job (TS: `throw uploadErr`
/// rejects `Promise.all`). Deliberate deviation from the TS oracle: when
/// the downloader aborts the batch (circuit breaker), the TS producer
/// stays blocked forever on a full `BoundedChannel` (job + queue-slot
/// leak). Tokio mpsc senders error when receivers drop, so the Rust
/// pipeline settles cleanly with the same observable summary (the
/// 'Remaining tracks' failure row).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn run_pipeline<D: OrchestratorDeps>(
    deps: Arc<D>,
    bus: EventBus,
    shared: Arc<Mutex<JobShared>>,
    options: &RipJobOptions,
    uncached_items: &[ResolvedTrackItem],
    job_controller: CancellationToken,
    cached_count: usize,
    is_multi_track: bool,
    max_collection_limit: u32,
    capped_count: usize,
    queue_start_time_ms: u64,
) -> RipJobSummary {
    tracing::debug!("Rip job started from queue");
    {
        let guard = shared.lock().expect("job poisoned");
        bus.emit(&OrchestratorEvent::Started(&guard.job));
    }

    // Rip job temp dir (TS: os.tmpdir()/rip_job_{ts}_{rand36}).
    let rip_job_dir: PathBuf =
        std::env::temp_dir().join(format!("rip_job_{}_{}", now_ms(), random36(4)));
    let _ = tokio::fs::create_dir_all(&rip_job_dir).await;

    let ripped_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let failed_tracks: Arc<Mutex<Vec<FailedTrack>>> = Arc::new(Mutex::new(Vec::new()));
    let texts = Arc::new(PipelineTexts::default());

    // Bounded channels (TS BoundedChannel(1) / BoundedChannel(2)).
    let (download_tx, download_rx) = tokio::sync::mpsc::channel::<PipelineItem>(1);
    let (upload_tx, upload_rx) = tokio::sync::mpsc::channel::<PipelineRipResult>(2);

    let is_cancelled =
        || shared.lock().expect("job poisoned").job.is_cancelled || job_controller.is_cancelled();

    // ── Producer ──────────────────────────────────────────────────────────
    // The sender is MOVED into the block: when the producer finishes, the
    // channel closes and the downloader's recv loop ends (TS channel.close()
    // in the producer's finally block).
    let producer = async {
        let download_tx = download_tx;
        for item in uncached_items {
            if is_cancelled() {
                break;
            }
            let pipeline_item = PipelineItem {
                track_id: item.id.clone(),
                storefront: item.storefront.clone(),
                meta_title: item.title.clone(),
                meta_artist: item.artist.clone(),
            };
            if download_tx.send(pipeline_item).await.is_err() {
                break;
            }
        }
    };

    // ── Downloader (worker 1) ────────────────────────────────────────────
    // upload_tx moves in so its drop closes the upload channel when the
    // downloader drains (TS closes uploadChannel in the downloader's
    // finally block).
    let downloader = async {
        let mut download_rx = download_rx;
        let upload_tx = upload_tx;
        while let Some(item) = download_rx.recv().await {
            if is_cancelled() {
                break;
            }

            let track_start_time = now_ms();
            let track_label = match (&item.meta_title, &item.meta_artist) {
                // TS truthiness: `meta?.title && meta?.artist` — an empty
                // string counts as absent.
                (Some(title), Some(artist)) if !title.is_empty() && !artist.is_empty() => {
                    format!("{artist} - {title}")
                }
                _ => format!("Track {}", item.track_id),
            };

            let on_progress: RipProgressCallback = {
                let texts = Arc::clone(&texts);
                let shared = Arc::clone(&shared);
                let bus = bus.clone();
                let track_label = track_label.clone();
                Arc::new(move |status, downloaded, total| {
                    let text = match (downloaded, total) {
                        (Some(d), Some(t)) => format!(
                            "⬇️ <b>{}:</b> <code>{}</code>",
                            html_escape(&track_label),
                            format_byte_progress(d, t, 12)
                        ),
                        _ => format!(
                            "⬇️ <b>{}:</b> {}",
                            html_escape(&track_label),
                            html_escape(status)
                        ),
                    };
                    *texts.download.lock().expect("texts poisoned") = Some(text.clone());
                    shared.lock().expect("job poisoned").job.active_action_text =
                        Some(text.clone());
                    let upload_text = texts.upload.lock().expect("texts poisoned").clone();
                    bus.emit_progress(&shared, None, Some(&text), upload_text.as_deref());
                })
            };

            let storefront = item.storefront.clone().unwrap_or_else(|| "us".to_string());
            match deps
                .rip(
                    &item.track_id,
                    Some(&on_progress),
                    &storefront,
                    job_controller.clone(),
                    Some(&rip_job_dir),
                )
                .await
            {
                Ok(rip_result) => {
                    // activeDownloadText = ''.
                    *texts.download.lock().expect("texts poisoned") = None;
                    shared.lock().expect("job poisoned").job.active_action_text = None;
                    let upload_item = PipelineRipResult {
                        track_id: item.track_id.clone(),
                        rip_result,
                        start_time_ms: track_start_time,
                    };
                    if upload_tx.send(upload_item).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    *texts.download.lock().expect("texts poisoned") = None;
                    if is_cancelled() {
                        break;
                    }

                    let err_msg = err.to_string();
                    let duration_ms = (now_ms() - track_start_time) as i64;
                    {
                        let mut failures = failed_tracks.lock().expect("failures poisoned");
                        failures.push(FailedTrack {
                            id: item.track_id.clone(),
                            error: err_msg.clone(),
                        });
                        shared.lock().expect("job poisoned").job.failed_count = failures.len();
                    }

                    tracing::error!(
                        track_id = %item.track_id,
                        duration_ms,
                        error = %err_msg,
                        "Rip job failed"
                    );

                    let _ = deps
                        .log_request(RequestLog {
                            telegram_id: options.user_id,
                            chat_id: options.chat_id,
                            apple_track_id: item.track_id.clone(),
                            is_cache_hit: false,
                            duration_ms: Some(duration_ms),
                            status: "failed".to_string(),
                            error_reason: Some(err_msg.clone()),
                        })
                        .await;

                    bus.emit_progress(&shared, Some("Processing next track..."), None, None);

                    // Circuit breaker: abort the remaining batch when the
                    // mirror looks offline.
                    let lower = err_msg.to_lowercase();
                    if lower.contains("timed out")
                        || lower.contains("mirror")
                        || lower.contains("502")
                        || lower.contains("503")
                    {
                        {
                            let mut failures = failed_tracks.lock().expect("failures poisoned");
                            failures.push(FailedTrack {
                                id: "Remaining tracks".to_string(),
                                error:
                                    "Mirror service offline / unreachable (stopped remaining batch)"
                                        .to_string(),
                            });
                            shared.lock().expect("job poisoned").job.failed_count = failures.len();
                        }
                        tracing::error!(
                            track_id = %item.track_id,
                            error = %err_msg,
                            "Lossless mirror appears to be down, aborting remaining batch to prevent repeated timeouts"
                        );
                        break;
                    }
                }
            }
        }
    };

    // ── Uploader (worker 2) ──────────────────────────────────────────────
    let uploader = async {
        let mut upload_rx = upload_rx;
        while let Some(upload_item) = upload_rx.recv().await {
            if is_cancelled() {
                // Delete the file if present, keep draining (TS continue).
                delete_file_if_exists(&upload_item.rip_result.file_path).await;
                continue;
            }

            upload_one(
                &deps,
                &bus,
                &shared,
                &texts,
                &failed_tracks,
                &ripped_count,
                options,
                &job_controller,
                is_multi_track,
                &upload_item,
            )
            .await;

            delete_file_if_exists(&upload_item.rip_result.file_path).await;
        }
    };

    // All three stages settle (channel drops propagate end-to-end).
    let _: ((), (), ()) = tokio::join!(producer, downloader, uploader);

    // Cleanup the rip job dir (TS uploader finally: rmSync recursive force).
    let _ = tokio::fs::remove_dir_all(&rip_job_dir).await;

    // Build the summary (TS: after Promise.all, inside the queue task).
    let total_elapsed_sec = format!("{:.1}", (now_ms() - queue_start_time_ms) as f64 / 1000.0);
    let failed = failed_tracks.lock().expect("failures poisoned").clone();
    let guard = shared.lock().expect("job poisoned");
    RipJobSummary {
        job_id: guard.job.id.clone(),
        job_header: guard.job.job_header.clone(),
        total_tracks: guard.job.total_tracks,
        cached_count,
        ripped_count: ripped_count.load(std::sync::atomic::Ordering::SeqCst),
        failed_count: failed.len(),
        failed_tracks: failed,
        skipped_uncached_tracks: Vec::new(),
        total_elapsed_sec,
        capped_count,
        max_collection_limit,
        is_cache_only: options.is_cache_only,
        is_group: options.is_group,
    }
}
// (summary end)

/// One upload iteration: caption → send (retries + backoff) → save → copy →
/// log. Upload-retries-exhausted is recorded as a track failure here
/// (Rust deviation note: TS throws and fails the whole job; see
/// `run_pipeline` docs — the Rust port records the failure like the
/// post-upload catch, keeping the job alive to drain remaining results,
/// which matches the observable summary modulo the throw).
#[allow(clippy::too_many_arguments)]
async fn upload_one<D: OrchestratorDeps>(
    deps: &Arc<D>,
    bus: &EventBus,
    shared: &Arc<Mutex<JobShared>>,
    texts: &Arc<PipelineTexts>,
    failed_tracks: &Arc<Mutex<Vec<FailedTrack>>>,
    ripped_count: &Arc<std::sync::atomic::AtomicUsize>,
    options: &RipJobOptions,
    job_controller: &CancellationToken,
    is_multi_track: bool,
    upload_item: &PipelineRipResult,
) {
    let track_id = upload_item.track_id.clone();
    let rip_result = &upload_item.rip_result;
    let track_label = format!("{} - {}", rip_result.artist, rip_result.title);
    let is_cancelled =
        || shared.lock().expect("job poisoned").job.is_cancelled || job_controller.is_cancelled();

    let caption = format_dump_caption(&DumpCaptionMetadata {
        apple_track_id: &track_id,
        title: &rip_result.title,
        artist: &rip_result.artist,
        album: &rip_result.album,
        duration: rip_result.duration,
        bit_depth: rip_result.bit_depth,
        sample_rate: rip_result.sample_rate,
        codec: Some(&rip_result.codec),
        genre: Some(&rip_result.genre),
        release_date: Some(&rip_result.release_date),
        track_number: Some(rip_result.track_number),
        track_count: Some(rip_result.track_count),
    });

    let upload_text = format!("⬆️ <b>Uploading:</b> <i>{}</i>", html_escape(&track_label));
    *texts.upload.lock().expect("texts poisoned") = Some(upload_text.clone());
    shared.lock().expect("job poisoned").job.active_action_text = Some(upload_text.clone());
    let download_text = texts.download.lock().expect("texts poisoned").clone();
    bus.emit_progress(shared, None, download_text.as_deref(), Some(&upload_text));

    // Send with retries: base * 2^(attempt-1) + rand*500 via abortable sleep.
    let max_retries = 4;
    enum SendOutcome {
        Audio(DumpUpload),
        /// Send succeeded but the message carried no audio media.
        NotAudio,
    }
    let mut outcome: Option<SendOutcome> = None;
    'upload: for attempt in 1..=max_retries {
        let on_upload: UploadProgressCallback = {
            let texts = Arc::clone(texts);
            let shared = Arc::clone(shared);
            let bus = bus.clone();
            Arc::new(move |uploaded, total| {
                let prog = format_byte_progress(uploaded, total, 12);
                let text = format!("⬆️ <b>Uploading:</b> <code>{}</code>", prog);
                *texts.upload.lock().expect("texts poisoned") = Some(text.clone());
                shared.lock().expect("job poisoned").job.active_action_text = Some(text.clone());
                let download_text = texts.download.lock().expect("texts poisoned").clone();
                bus.emit_progress(&shared, None, download_text.as_deref(), Some(&text));
            })
        };

        match deps
            .sink()
            .send_audio_to_dump(
                &rip_result.file_path,
                &rip_result.title,
                &rip_result.artist,
                rip_result.duration,
                &caption,
                Some(&on_upload),
            )
            .await
        {
            Ok(upload) => {
                outcome = Some(match upload {
                    Some(dump) => SendOutcome::Audio(dump),
                    // Send succeeded but the media was not audio — the TS
                    // retry loop breaks here too (`dumpMsg` is set), and the
                    // `media?.type === 'audio'` check below fails.
                    None => SendOutcome::NotAudio,
                });
                break 'upload;
            }
            Err(upload_err) => {
                if is_cancelled() {
                    // TS: break out of the retry loop without recording.
                    break 'upload;
                }
                if attempt < max_retries {
                    let delay = deps.upload_retry_base_ms() as f64 * 2f64.powi(attempt - 1)
                        + (now_ms() % 500) as f64;
                    tracing::warn!(
                        track_id = %track_id,
                        attempt,
                        max_retries,
                        delay_ms = delay.round() as u64,
                        error = %upload_err,
                        "Track upload to dump failed, retrying"
                    );
                    // abortableSleep — cancellation cuts the delay short.
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(delay as u64)) => {}
                        _ = job_controller.cancelled() => {}
                    }
                } else {
                    tracing::error!(
                        track_id = %track_id,
                        attempts = max_retries,
                        error = %upload_err,
                        "All upload retries exhausted for track"
                    );
                    // TS `throw uploadErr` rejects the whole job with no
                    // request log. Rust deviation: record the track failure
                    // and keep the job alive (see `run_pipeline` docs).
                    {
                        let mut failures = failed_tracks.lock().expect("failures poisoned");
                        failures.push(FailedTrack {
                            id: track_id.clone(),
                            error: upload_err.to_string(),
                        });
                        shared.lock().expect("job poisoned").job.failed_count = failures.len();
                    }
                    return;
                }
            }
        }
    }

    let Some(outcome) = outcome else {
        // Cancelled mid-retries (TS: break, no failure recorded).
        return;
    };

    // TS `if (dumpMsg?.media?.type === 'audio') { ... } else if (!cancelled
    // && !aborted) { record 'no audio media' — no request log }`.
    let dump_upload = match outcome {
        SendOutcome::Audio(dump_upload) => dump_upload,
        SendOutcome::NotAudio => {
            if !is_cancelled() {
                let err_msg = "Upload failed: no audio media returned";
                {
                    let mut failures = failed_tracks.lock().expect("failures poisoned");
                    failures.push(FailedTrack {
                        id: track_id.clone(),
                        error: err_msg.to_string(),
                    });
                    shared.lock().expect("job poisoned").job.failed_count = failures.len();
                }
                tracing::error!(track_id = %track_id, "Track upload failed: no audio media returned");
            }
            return;
        }
    };

    // Post-upload block: save + copy + log share one try/catch — any
    // failure records the track failure and continues.
    let post_upload: Result<i64, String> = async {
        deps.save_track(SaveTrackInput::from_rip_result(
            &track_id,
            rip_result,
            dump_upload.message_id,
            &dump_upload.file_id,
            &dump_upload.file_unique_id,
        ))
        .await
        .map_err(|e| e.to_string())?;

        if !options.is_cache_only {
            let reply_to = (options.delivery_chat_id == options.chat_id)
                .then_some(options.reply_to_message_id)
                .flatten();
            deps.sink()
                .send_dump_copy(
                    options.delivery_chat_id,
                    dump_upload.message_id,
                    reply_to,
                    is_multi_track,
                )
                .await
                .map_err(|e| e.to_string())?;
        }

        let total_duration_ms = (now_ms() - upload_item.start_time_ms) as i64;
        deps.log_request(RequestLog {
            telegram_id: options.user_id,
            chat_id: options.chat_id,
            apple_track_id: track_id.clone(),
            is_cache_hit: false,
            duration_ms: Some(total_duration_ms),
            status: "completed".to_string(),
            error_reason: None,
        })
        .await
        .map_err(|e| e.to_string())?;

        Ok(total_duration_ms)
    }
    .await;

    match post_upload {
        Ok(total_duration_ms) => {
            *texts.upload.lock().expect("texts poisoned") = None;
            shared.lock().expect("job poisoned").job.active_action_text = None;
            let new_count = ripped_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            shared.lock().expect("job poisoned").job.ripped_count = new_count;

            tracing::info!(
                track = format!("{} - {}", rip_result.artist, rip_result.title),
                time = format!("{:.1}s", total_duration_ms as f64 / 1000.0),
                event = if options.is_cache_only {
                    "Track cached to dump"
                } else {
                    "Track completed"
                },
                "Track completed"
            );
        }
        Err(err_msg) => {
            *texts.upload.lock().expect("texts poisoned") = None;
            shared.lock().expect("job poisoned").job.active_action_text = None;
            if is_cancelled() {
                return;
            }
            record_failure(
                shared,
                failed_tracks,
                deps,
                options,
                &track_id,
                err_msg,
                upload_item.start_time_ms,
            )
            .await;
        }
    }
}

/// Record a track failure: push the row, update the counter, log the
/// request (TS catch blocks in the uploader).
#[allow(clippy::too_many_arguments)]
async fn record_failure<D: OrchestratorDeps>(
    shared: &Arc<Mutex<JobShared>>,
    failed_tracks: &Arc<Mutex<Vec<FailedTrack>>>,
    deps: &Arc<D>,
    options: &RipJobOptions,
    track_id: &str,
    err_msg: String,
    start_time_ms: u64,
) {
    {
        let mut failures = failed_tracks.lock().expect("failures poisoned");
        failures.push(FailedTrack {
            id: track_id.to_string(),
            error: err_msg.clone(),
        });
        shared.lock().expect("job poisoned").job.failed_count = failures.len();
    }
    tracing::error!(track_id = %track_id, error = %err_msg, "Track upload failed");
    let _ = deps
        .log_request(RequestLog {
            telegram_id: options.user_id,
            chat_id: options.chat_id,
            apple_track_id: track_id.to_string(),
            is_cache_hit: false,
            duration_ms: Some((now_ms() - start_time_ms) as i64),
            status: "failed".to_string(),
            error_reason: Some(err_msg),
        })
        .await;
}

// ── helpers ─────────────────────────────────────────────────────────────

fn kind_str(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Track => "track",
        TargetKind::Album => "album",
        TargetKind::Artist => "artist",
        TargetKind::Playlist => "playlist",
    }
}

/// TS `Math.random().toString(36).slice(2, 6)` — 4 base36 chars.
fn random36(len: usize) -> String {
    let mut out = String::new();
    let mut state = now_ms() ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for _ in 0..len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = (state >> 33) % 36;
        out.push(b"abcdefghijklmnopqrstuvwxyz0123456789"[idx as usize] as char);
    }
    out
}

async fn delete_file_if_exists(path: &str) {
    let _ = tokio::fs::remove_file(std::path::Path::new(path)).await;
}
