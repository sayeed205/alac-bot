//! Rip orchestrator for the live `/alac` command contract.
//!
//! Owns job bookkeeping, resolves parsed items to tracks, serves the cache
//! fast-path, and runs two concurrent lanes: lane 1 rips and tags (one job
//! at a time, through the sequential rip queue) while lane 2 performs every
//! Telegram upload/ZIP/delivery item from all jobs on a single global
//! dispatcher — so downloads never wait on uploads and vice versa.
//!
//! Upload retry exhaustion records a failed track and continues later tasks
//! instead of rejecting the whole job: one bad upload never strands the
//! remaining work, and the failure still surfaces in the summary.

pub mod caption;
pub mod deps;
pub mod types;

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio_util::sync::CancellationToken;

use crate::{
    catalog::artwork_url_at_size,
    orchestrator::{
        caption::{
            format_album_details_caption, format_dump_caption, format_zip_dump_caption,
            html_escape, AlbumDetailsCaptionMetadata, DumpCaptionMetadata, DumpZipCaptionMetadata,
        },
        deps::{
            AlbumUpload, CachedAlbum, DumpUpload, OrchestratorDeps, RequestLog, SaveTrackInput,
            UploadProgressCallback,
        },
        types::{
            ActiveRipJob, EventCallback, FailedTrack, JobPhase, OrchestratorEvent,
            ResolutionFailure, RipJobOptions, RipJobProgress, RipJobSummary, TerminalJobState,
            ZipDeliveryInfo,
        },
    },
    progress::format_byte_progress,
    queue::{EnqueueOptions, SequentialRipQueue},
    ripper::RipProgressCallback,
    settings::BotSettings,
    types::{AlbumTracks, ArtistTracks, Codec, Provider, TargetKind, TrackKey, TrackRipResult},
    wrapper::CodecPreference,
    zip::{
        album_generation_hash, create_zip_archive, plan_zip_parts_with_codec,
        sanitize_archive_filename, ZipTrackEntry, TELEGRAM_SPLIT_THRESHOLD_BYTES,
    },
};

/// All orchestrator failures surface as plain messages, while resolution
/// failures retain every failed target for the bot to render.
#[derive(Debug)]
pub enum OrchestratorError {
    DependenciesNotSet,
    Message(String),
    ResolutionFailed { failures: Vec<ResolutionFailure> },
    AdmissionLimit,
    UserAdmissionLimit,
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DependenciesNotSet => write!(
                f,
                "RipOrchestrator dependencies not configured. Call setDependencies() first."
            ),
            Self::Message(message) => f.write_str(message),
            Self::ResolutionFailed { failures } => {
                write!(f, "Failed to resolve any tracks")?;
                if !failures.is_empty() {
                    write!(f, ": ")?;
                    for (index, failure) in failures.iter().enumerate() {
                        if index > 0 {
                            write!(f, "; ")?;
                        }
                        write!(f, "{failure}")?;
                    }
                }
                Ok(())
            }
            Self::AdmissionLimit => write!(f, "job admission limit reached"),
            Self::UserAdmissionLimit => {
                write!(f, "user already has the maximum number of active jobs")
            }
        }
    }
}

impl std::error::Error for OrchestratorError {}

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

/// A resolved track to rip.
#[derive(Debug, Clone)]
struct ResolvedTrackItem {
    id: String,
    title: Option<String>,
    artist: Option<String>,
    storefront: Option<String>,
    is_streamable: Option<bool>,
}

/// One item moving through the rip work feed.
struct PipelineItem {
    track_id: String,
    storefront: Option<String>,
    meta_title: Option<String>,
    meta_artist: Option<String>,
    is_streamable: Option<bool>,
}

/// One finished rip awaiting its upload.
struct PipelineRipResult {
    track_id: String,
    rip_result: TrackRipResult,
    start_time_ms: u64,
}

fn stream_display_label(status: &str) -> Option<&str> {
    status
        .strip_prefix("Connecting stream for ")
        .map(|value| value.strip_suffix("...").unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
}

/// A cached track queued for zip staging: lane 1 downloads it from the
/// dump channel into the zip workspace while the rip loop runs.
#[derive(Debug, Clone)]
struct StageItem {
    /// Resolved track, kept so a staging failure can re-rip it.
    item: ResolvedTrackItem,
    message_id: i64,
    /// Caption label (title - artist, falling back to the resolved metadata).
    label: String,
    /// Precomputed `"{title} - {artist} [{id}].m4a"` archive filename.
    archive_filename: String,
}

/// Everything the two lanes need about one job, shared by `Arc` into the
/// lane-2 upload items and the finalize marker.
struct JobContext {
    options: RipJobOptions,
    zip_build: bool,
    zip_deliver: bool,
    zip_dir: Option<PathBuf>,
    zip_sources: Arc<Mutex<Vec<ZipTrackEntry>>>,
    zip_album: String,
    zip_artist: String,
    zip_album_id: String,
    zip_storefront: String,
    zip_genre: Option<String>,
    zip_record_label: Option<String>,
    zip_copyright: Option<String>,
    zip_generation_hash: Option<String>,
    /// Highest-quality codec seen in this job's rips (`alac`, `mp4a.40.2`,
    /// `ec-3`); drives the album-details quality bullet.
    zip_codec: Arc<std::sync::Mutex<Option<String>>>,
    zip_artwork_url: Option<String>,
    zip_release_date: String,
    warnings: Vec<String>,
    /// Set once when an Atmos-requested job delivers a non-Atmos rip; folded
    /// into the summary warnings.
    atmos_warning: Arc<std::sync::Mutex<Option<String>>>,
    cached_count: usize,
    is_multi_track: bool,
    max_collection_limit: u32,
    capped_count: usize,
    queue_start_time_ms: u64,
    ripped_count: Arc<std::sync::atomic::AtomicUsize>,
    failed_tracks: Arc<Mutex<Vec<FailedTrack>>>,
    texts: Arc<PipelineTexts>,
    first_delivered_msg_id: Arc<Mutex<Option<i32>>>,
}

/// Job state shared by the orchestrator and both lanes (mutated by
/// reference from several concurrent stages).
struct JobShared {
    job: ActiveRipJob,
}

/// The current lane-1/lane-2 progress texts, shared by closure capture
/// across pipeline stages.
#[derive(Default)]
struct PipelineTexts {
    download: Mutex<Option<String>>,
    upload: Mutex<Option<String>>,
}

impl PipelineTexts {
    /// Snapshot both lane slots for an emit. Each lane owns its slot:
    /// emitting with `None` would erase the other lane's live progress
    /// from the dashboard.
    fn snapshot(&self) -> (Option<String>, Option<String>) {
        (
            self.download.lock().expect("texts poisoned").clone(),
            self.upload.lock().expect("texts poisoned").clone(),
        )
    }
}

/// Event registry shared by the orchestrator and its pipelines.
#[derive(Clone)]
struct EventBus {
    subscribers: Arc<Mutex<Vec<EventCallback>>>,
}

#[derive(Default)]
struct Admissions {
    jobs: HashMap<String, (i64, bool)>,
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

    /// Snapshot counters and emit a progress event, optionally overriding
    /// the activity text.
    fn emit_progress(
        &self,
        shared: &Arc<Mutex<JobShared>>,
        activity_override: Option<&str>,
        active_download: Option<&str>,
        active_upload: Option<&str>,
    ) {
        let (job, progress) = {
            let guard = shared.lock().expect("job poisoned");
            let completed_tracks = guard.job.cached_count
                + guard.job.ripped_count
                + guard.job.failed_count
                + guard.job.skipped_count;
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
                skipped_count: guard.job.skipped_count,
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
    /// Lane 2: a single global dispatcher serializing every Telegram-I/O
    /// task (track uploads, zip packaging, deliveries). Items from ALL
    /// jobs interleave here while lane 1 keeps ripping — download never
    /// waits for upload and vice versa. Bounded at 16 pending items so a
    /// stalled upload lane back-pressures lane 1 instead of eating disk.
    upload_lane: Arc<Mutex<Option<tokio::sync::mpsc::Sender<LaneTask>>>>,
    admissions: Arc<Mutex<Admissions>>,
}

impl Default for RipOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

/// A lane-2 task: a self-contained boxed closure capturing everything it
/// needs (the job's `Arc<D>` deps, shared job context, files to act on).
/// The dispatcher runs items FIFO; the job's cancellation token is captured
/// inside each closure, which must check it and clean up after itself.
struct LaneTask {
    run: Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
    /// Label for panic diagnostics only.
    label: &'static str,
}

/// Push one item into the global upload lane (FIFO across all jobs, so a
/// job's items run in order and its finalize marker runs last).
/// Back-pressures the caller when the 16-item buffer is full, which pauses
/// lane 1 instead of eating disk.
///
/// Returns false when the lane was never initialized — the item is dropped
/// and the caller must finish without it (the job is going away anyway).
async fn push_lane_task(
    lane: &Arc<Mutex<Option<tokio::sync::mpsc::Sender<LaneTask>>>>,
    label: &'static str,
    run: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + 'static,
) -> bool {
    let tx = {
        let guard = lane.lock().expect("upload lane poisoned");
        match guard.as_ref() {
            Some(tx) => tx.clone(),
            None => return false,
        }
    };
    let task = LaneTask {
        run: Box::new(run),
        label,
    };
    match tx.send(task).await {
        Ok(()) => true,
        Err(_) => {
            tracing::error!(
                lane = "upload",
                item = label,
                "upload lane closed; task dropped"
            );
            false
        }
    }
}

impl RipOrchestrator {
    pub fn new() -> Self {
        Self {
            bus: EventBus::new(),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            queue: SequentialRipQueue::new(),
            upload_lane: Arc::new(Mutex::new(None)),
            admissions: Arc::new(Mutex::new(Admissions::default())),
        }
    }

    /// Lazily spawn the global lane-2 dispatcher (idempotent).
    fn ensure_upload_lane(&self) -> tokio::sync::mpsc::Sender<LaneTask> {
        let mut guard = self.upload_lane.lock().expect("upload lane poisoned");
        if let Some(tx) = guard.as_ref() {
            return tx.clone();
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel::<LaneTask>(16);
        tokio::spawn(async move {
            while let Some(task) = rx.recv().await {
                let result = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
                    (task.run)(),
                ))
                .await;
                if result.is_err() {
                    tracing::error!(
                        lane = "upload",
                        item = task.label,
                        "upload lane task panicked; lane continues"
                    );
                }
            }
        });
        *guard = Some(tx.clone());
        tx
    }

    /// Subscribe to every orchestrator event.
    pub fn subscribe(&self, callback: EventCallback) {
        self.bus
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .push(callback);
    }

    /// All jobs that have not reached a terminal state.
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

    /// Look up one job's live snapshot.
    pub fn get_job(&self, id: &str) -> Option<ActiveRipJob> {
        self.jobs
            .lock()
            .expect("jobs poisoned")
            .get(id)
            .map(|shared| shared.lock().expect("job poisoned").job.clone())
    }

    fn set_phase(&self, shared: &Arc<Mutex<JobShared>>, phase: JobPhase) {
        shared.lock().expect("job poisoned").job.phase = phase;
    }

    /// Emit one and only one terminal event.  Cancellation removes the job
    /// from the public table immediately, so late queue/pipeline completion
    /// cannot manufacture a second terminal event.
    fn terminalize(
        &self,
        shared: &Arc<Mutex<JobShared>>,
        state: TerminalJobState,
        summary: Option<&RipJobSummary>,
        error: Option<&str>,
    ) -> bool {
        let (job, cancelled_by) = {
            let mut guard = shared.lock().expect("job poisoned");
            if guard.job.terminal_state.is_some() {
                return false;
            }
            guard.job.terminal_state = Some(state);
            guard.job.completed = true;
            (guard.job.clone(), guard.job.cancelled_by.clone())
        };
        match state {
            TerminalJobState::Completed => {
                if let Some(summary) = summary {
                    self.bus.emit(&OrchestratorEvent::Completed(&job, summary));
                }
            }
            TerminalJobState::Cancelled => {
                self.bus
                    .emit(&OrchestratorEvent::Cancelled(&job, &cancelled_by));
            }
            TerminalJobState::Failed => {
                if let Some(error) = error {
                    self.bus.emit(&OrchestratorEvent::Failed(&job, error));
                }
            }
        }
        true
    }

    /// Cancel a job; false when missing, already cancelled, or completed.
    pub fn cancel_job(&self, id: &str, cancelled_by: Option<&str>) -> bool {
        let Some(shared) = self.jobs.lock().expect("jobs poisoned").get(id).cloned() else {
            return false;
        };

        let cancelled_by = cancelled_by.map(str::to_string);
        let mut guard = shared.lock().expect("job poisoned");
        if guard.job.is_cancelled || guard.job.terminal_state.is_some() {
            return false;
        }
        guard.job.is_cancelled = true;
        guard.job.cancelled_by = cancelled_by;
        guard.job.terminal_state = Some(TerminalJobState::Cancelled);
        guard.job.completed = true;
        guard.job.controller.cancel();
        let event_job = guard.job.clone();
        let by = guard.job.cancelled_by.clone();
        drop(guard);

        self.jobs.lock().expect("jobs poisoned").remove(id);
        self.release_admission(id);
        self.bus
            .emit(&OrchestratorEvent::Cancelled(&event_job, &by));
        true
    }

    /// Run the whole rip flow for one request. Deps arrive per call.
    pub async fn start_job<D: OrchestratorDeps>(
        &self,
        deps: Arc<D>,
        options: &RipJobOptions,
    ) -> Result<RipJobSummary, OrchestratorError> {
        let job_id = cuid2::create_id();
        self.admit(&job_id, options)?;

        // Settings are a snapshot.  The live-availability decision is made
        // after resolution and cache delivery, not as an early gate.
        let settings = deps.get_settings().await;

        let job_controller = CancellationToken::new();
        // Initial job header from the parsed targets.
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
                delivery_chat_id: options.delivery_chat_id,
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
                phase: JobPhase::Resolving,
                terminal_state: None,
                skipped_count: 0,
                is_cache_only: options.is_cache_only,
                is_group: options.is_group,
                reply_to_message_id: options.reply_to_message_id,
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
                self.terminalize(&shared, TerminalJobState::Completed, Some(summary), None);
            }
            Err(err) => {
                let message = err.to_string();
                let cancelled = shared
                    .lock()
                    .expect("job poisoned")
                    .job
                    .terminal_state
                    .is_some_and(|state| state == TerminalJobState::Cancelled);
                if !cancelled {
                    self.terminalize(&shared, TerminalJobState::Failed, None, Some(&message));
                }
            }
        }
        self.jobs.lock().expect("jobs poisoned").remove(&job_id);
        self.release_admission(&job_id);
        result
    }

    fn admit(&self, job_id: &str, options: &RipJobOptions) -> Result<(), OrchestratorError> {
        let mut admissions = self.admissions.lock().expect("admissions poisoned");
        // Admins bypass every admission cap (user + global). Their jobs still
        // occupy a slot so `/cancel` bookkeeping and the dashboard can find
        // them, but they never crowd anyone out nor get crowded out.
        if !options.is_admin {
            let non_admin_jobs = admissions
                .jobs
                .values()
                .filter(|(_, is_admin)| !*is_admin)
                .count();
            if non_admin_jobs >= 16 {
                return Err(OrchestratorError::AdmissionLimit);
            }
            let user_jobs = admissions
                .jobs
                .values()
                .filter(|(user_id, _)| *user_id == options.user_id)
                .count();
            if user_jobs >= 4 {
                return Err(OrchestratorError::UserAdmissionLimit);
            }
        }
        admissions
            .jobs
            .insert(job_id.to_owned(), (options.user_id, options.is_admin));
        Ok(())
    }

    fn release_admission(&self, job_id: &str) {
        self.admissions
            .lock()
            .expect("admissions poisoned")
            .jobs
            .remove(job_id);
    }

    /// The queue phase of the job flow: resolve → cap → cache lookup →
    /// admission of the two-lane pipeline.
    async fn run_job<D: OrchestratorDeps>(
        &self,
        deps: Arc<D>,
        options: &RipJobOptions,
        shared: Arc<Mutex<JobShared>>,
        job_controller: CancellationToken,
        settings: BotSettings,
    ) -> Result<RipJobSummary, OrchestratorError> {
        self.set_phase(&shared, JobPhase::Resolving);
        shared.lock().expect("job poisoned").job.active_action_text =
            Some("🔍 Resolving metadata & tracklist...".to_string());
        self.bus.emit_progress(
            &shared,
            Some("Resolving metadata & tracklist..."),
            Some("🔍 Resolving metadata & tracklist..."),
            None,
        );

        // Resolve every parsed item.
        let mut resolved_tracks: Vec<ResolvedTrackItem> = Vec::new();
        let mut album_name: Option<String> = None;
        let mut album_artist: Option<String> = None;
        let mut album_artwork_url: Option<String> = None;
        let mut album_release_date: Option<String> = None;
        let mut album_genre: Option<String> = None;
        let mut album_record_label: Option<String> = None;
        let mut album_copyright: Option<String> = None;
        let mut album_id: Option<String> = None;
        let mut album_sf: Option<String> = None;
        let mut resolution_failures = Vec::new();

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
                        is_streamable: None,
                    });
                    Ok(())
                }
                TargetKind::Album => match deps.fetch_album_tracks(&item.id, &effective_sf).await {
                    Ok(AlbumTracks { album, tracks }) => {
                        album_name = Some(album.album.clone());
                        album_artist = Some(album.artist.clone());
                        album_artwork_url = Some(album.artwork_url.clone());
                        album_release_date = Some(album.release_date.clone());
                        album_genre = album.genre.clone();
                        album_record_label = album.record_label.clone();
                        album_copyright = album.copyright.clone();
                        album_id = Some(item.id.clone());
                        album_sf = Some(effective_sf.clone());
                        for t in tracks {
                            resolved_tracks.push(ResolvedTrackItem {
                                id: t.id.clone(),
                                title: Some(t.title.clone()),
                                artist: Some(t.artist.clone()),
                                storefront: Some(effective_sf.clone()),
                                is_streamable: t.is_streamable,
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
                                    is_streamable: None,
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
                                    is_streamable: None,
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
                resolution_failures.push(ResolutionFailure {
                    kind: item.kind,
                    id: item.id.clone(),
                    error: err_msg,
                });
            }
        }

        if resolved_tracks.is_empty() {
            if resolution_failures.is_empty() {
                resolution_failures.push(ResolutionFailure {
                    kind: TargetKind::Track,
                    id: String::new(),
                    error: "No valid tracks found to process.".to_string(),
                });
            }
            return Err(OrchestratorError::ResolutionFailed {
                failures: resolution_failures,
            });
        }

        // Dedup preserving order.
        let mut seen_ids: HashSet<String> = HashSet::new();
        let unique_tracks: Vec<ResolvedTrackItem> = resolved_tracks
            .into_iter()
            .filter(|t| seen_ids.insert(t.id.clone()))
            .collect();

        // Cap collections for non-admins.
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

        // Job header refinement + total. Empty strings count as absent.
        fn non_empty(s: &Option<String>) -> Option<&str> {
            s.as_deref().filter(|s| !s.is_empty())
        }
        let header = match (non_empty(&album_name), non_empty(&album_artist)) {
            (Some(name), Some(artist)) => {
                if let (Some(id), Some(sf)) = (&album_id, &album_sf) {
                    let album_url = format!("https://music.apple.com/{sf}/album/{id}");
                    format!(
                        "Album: <a href=\"{album_url}\"><b>{}</b></a> by <b>{}</b>",
                        html_escape(name),
                        html_escape(artist)
                    )
                } else {
                    format!(
                        "Album: <b>{}</b> by <b>{}</b>",
                        html_escape(name),
                        html_escape(artist)
                    )
                }
            }
            _ if tracks_to_process.len() == 1 => {
                let first = &tracks_to_process[0];
                match (non_empty(&first.title), non_empty(&first.artist)) {
                    (Some(title), Some(artist)) => {
                        format!(
                            "<b>{}</b> - <b>{}</b>",
                            html_escape(title),
                            html_escape(artist)
                        )
                    }
                    _ if options.is_cache_only => {
                        format!("Track Cache: <code>{}</code>", html_escape(&first.id))
                    }
                    _ => format!("Track ID: <code>{}</code>", html_escape(&first.id)),
                }
            }
            _ if options.is_cache_only => {
                format!("Batch Cache: <b>{} tracks</b>", tracks_to_process.len())
            }
            _ => format!("Batch: <b>{} tracks</b>", tracks_to_process.len()),
        };
        {
            let mut guard = shared.lock().expect("job poisoned");
            guard.job.job_header = header;
            guard.job.total_tracks = tracks_to_process.len();
        }

        // Two-lane ZIP semantics:
        // - `zip_build`: every single-album job with >1 track packages and
        //   caches the archive in the dump channel (the always-zip rule), so
        //   any later `-z` request is a cache hit.
        // - `zip_deliver`: the user asked for the archive (`-z`, `/zip`,
        //   `/dump`) — individual track delivery is replaced by the ZIP.
        let is_album_job = options.parsed_items.len() == 1
            && options.parsed_items[0].kind == TargetKind::Album
            && tracks_to_process.len() > 1;
        let zip_build = is_album_job;
        // Delivery only happens when the user asked for it AND the archive
        // will actually be built (single-track albums can't zip).
        let zip_deliver = options.zip && is_album_job;
        // Warn only when the user explicitly asked for a ZIP and the
        // album turned out to have a single track (implicit auto-attempts
        // never warn).
        let mut warnings = Vec::new();
        if options.zip_explicit
            && options.zip
            && options.parsed_items.len() == 1
            && options.parsed_items[0].kind == TargetKind::Album
            && tracks_to_process.len() == 1
        {
            warnings.push(
                "ZIP packaging skipped: album has a single track; delivered as a normal track download."
                    .to_owned(),
            );
        }
        // Generation identity of the resolved track set. Cached ZIP
        // parts recorded with this hash can be reused instead of rebuilt.
        let zip_generation_hash = zip_build.then(|| {
            let ids: Vec<&str> = tracks_to_process.iter().map(|t| t.id.as_str()).collect();
            album_generation_hash(Provider::Apple.as_str(), &options.parsed_items[0].id, &ids)
        });
        let zip_dir = if zip_build {
            let dir = std::env::temp_dir().join(format!("zip_job_{}", cuid2::create_id()));
            tokio::fs::create_dir_all(&dir).await.map_err(|error| {
                OrchestratorError::Message(format!("create ZIP workspace: {error}"))
            })?;
            Some(dir)
        } else {
            None
        };
        let zip_sources = Arc::new(Mutex::new(Vec::<ZipTrackEntry>::new()));

        // Cache lookup; a DB failure fails the whole job.
        self.set_phase(&shared, JobPhase::CheckingCache);
        let check_label = {
            let header = shared.lock().expect("job poisoned").job.job_header.clone();
            format!("🔍 Checking cache: {header}")
        };
        shared.lock().expect("job poisoned").job.active_action_text = Some(check_label.clone());
        self.bus.emit_progress(
            &shared,
            Some("Checking local cache..."),
            Some(&check_label),
            None,
        );
        let target_codec = match options.codec_preference {
            CodecPreference::Atmos => Codec::Ec3,
            CodecPreference::HighestQuality => Codec::Alac,
        };
        let requested_ids: Vec<TrackKey> = tracks_to_process
            .iter()
            .map(|t| TrackKey::new(Provider::Apple, t.id.clone()).with_codec(target_codec))
            .collect();
        let mut existing_tracks_map = deps
            .find_cached_tracks(&requested_ids)
            .await
            .map_err(OrchestratorError::Message)?;

        shared.lock().expect("job poisoned").job.active_action_text = None;

        // Force + admin purge.
        if options.is_force && options.is_admin {
            let mut old_message_ids: Vec<i64> = Vec::new();
            for item in &tracks_to_process {
                let lookup_key =
                    TrackKey::new(Provider::Apple, item.id.clone()).with_codec(target_codec);
                if let Some(cached) = existing_tracks_map.remove(&lookup_key) {
                    old_message_ids.push(cached.message_id);
                    // Per-item delete; errors are swallowed.
                    let _ = deps.delete_track(&lookup_key).await;
                }
            }
            if !old_message_ids.is_empty() {
                tracing::debug!(
                    count = old_message_ids.len(),
                    "Deleting old dump messages on force re-rip prior to queue"
                );
                // Bulk delete; errors are swallowed.
                let _ = deps.sink().delete_dump_messages(&old_message_ids).await;
            }
        }

        // Pre-queue cache handling.
        let mut uncached_items: Vec<ResolvedTrackItem> = Vec::new();
        let mut cached_count = 0usize;
        let is_multi_track = tracks_to_process.len() > 1;
        let mut first_delivered_msg_id: Option<i32> = None;
        // Cached tracks queued for lane-1 staging (zip-build jobs whose
        // archive isn't already cached). Downloaded concurrently with the
        // rip loop, feeding the rebuild.
        let mut stage_items: Vec<StageItem> = Vec::new();

        // ZIP reuse precondition. Cached parts recorded under the same
        // generation hash can serve this job directly — but only when every
        // track is a cache hit (otherwise the rebuild path needs staged
        // sources anyway) and the user is not forcing a re-rip. When set,
        // the per-track ZIP staging downloads below are skipped: they exist
        // only to feed the rebuild.
        let mut zip_reuse: Option<Vec<CachedAlbum>> = None;
        if let Some(hash) = &zip_generation_hash {
            if !options.is_force && existing_tracks_map.len() == tracks_to_process.len() {
                match deps
                    .find_albums(
                        Provider::Apple,
                        &options.parsed_items[0].id,
                        Some(target_codec),
                    )
                    .await
                {
                    Ok(rows) => {
                        let complete_set = !rows.is_empty()
                            && rows.iter().all(|row| row.generation_hash == *hash)
                            && {
                                let total = rows[0].total_parts.max(1) as usize;
                                rows.len() == total
                                    && (1..=total)
                                        .zip(&rows)
                                        .all(|(expected, row)| row.part_index as usize == expected)
                            };
                        if complete_set {
                            tracing::info!(
                                album_id = %options.parsed_items[0].id,
                                parts = rows.len(),
                                "Reusing cached album ZIP parts"
                            );
                            zip_reuse = Some(rows);
                        }
                    }
                    Err(error) => {
                        // Reuse is an optimization; treat lookup failure as
                        // a rebuild and keep going.
                        tracing::warn!(%error, "album ZIP cache lookup failed; rebuilding");
                    }
                }
            }
        }

        for item in &tracks_to_process {
            if job_controller.is_cancelled() {
                return Err(OrchestratorError::Message(
                    "Download was cancelled".to_string(),
                ));
            }

            let lookup_key =
                TrackKey::new(Provider::Apple, item.id.clone()).with_codec(target_codec);
            let Some(cached) = existing_tracks_map.get(&lookup_key) else {
                uncached_items.push(item.clone());
                continue;
            };

            // Zip-build jobs (single album, >1 track) stage cached tracks
            // into the zip workspace — in lane 1, alongside the rips. If
            // staging later finds the dump message undownloadable the track
            // is re-ripped so the archive still has a chance to complete.
            if zip_build && zip_reuse.is_none() {
                let filename = format!(
                    "{} - {} [{}].m4a",
                    sanitize_archive_filename(&cached.title),
                    sanitize_archive_filename(&cached.artist),
                    item.id
                );
                let label = match (&cached.title, &cached.artist) {
                    (t, a) if !t.is_empty() && !a.is_empty() => format!("{t} - {a}"),
                    (t, _) if !t.is_empty() => t.clone(),
                    _ => match (&item.title, &item.artist) {
                        (Some(t), Some(a)) if !t.is_empty() && !a.is_empty() => {
                            format!("{t} - {a}")
                        }
                        (Some(t), _) if !t.is_empty() => t.clone(),
                        _ => format!("Track {}", item.id),
                    },
                };
                stage_items.push(StageItem {
                    item: item.clone(),
                    message_id: cached.message_id,
                    label,
                    archive_filename: filename,
                });
            }

            if options.is_cache_only {
                cached_count += 1;
                shared.lock().expect("job poisoned").job.cached_count = cached_count;
                tracing::info!(track_id = %item.id, "Track already cached in dump channel");
                self.bus
                    .emit_progress(&shared, Some("Recognized cached tracks..."), None, None);
            } else if zip_deliver {
                // ZIP-delivery jobs deliver the archive, not the individual
                // track files. The cache row is still validated by the lane-1
                // staging download; only the request log remains.
                let outcome = deps
                    .log_request(RequestLog {
                        telegram_id: options.user_id,
                        chat_id: options.chat_id,
                        track_key: TrackKey::new(Provider::Apple, item.id.clone()),
                        is_cache_hit: true,
                        duration_ms: Some(0),
                        status: "completed".to_string(),
                        error_reason: None,
                    })
                    .await
                    .map_err(|e| e.to_string());
                match outcome {
                    Ok(()) => {
                        tracing::info!(track_id = %item.id, time = "0ms", "Cache hit: staged for ZIP");
                    }
                    Err(err) => {
                        // Logging alone cannot prove the dump row stale (the
                        // staging download already re-validates it), so only
                        // warn and keep the cache hit.
                        tracing::warn!(
                            track_id = %item.id,
                            error = %err,
                            "Failed to log cached track request for ZIP job"
                        );
                    }
                }
                cached_count += 1;
                shared.lock().expect("job poisoned").job.cached_count = cached_count;
                self.bus
                    .emit_progress(&shared, Some("Staged cached tracks..."), None, None);
            } else {
                let reply_to = (options.delivery_chat_id == options.chat_id)
                    .then_some(options.reply_to_message_id)
                    .flatten();
                let outcome: Result<i32, String> = async {
                    let sent_id = deps
                        .sink()
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
                        track_key: TrackKey::new(Provider::Apple, item.id.clone()),
                        is_cache_hit: true,
                        duration_ms: Some(0),
                        status: "completed".to_string(),
                        error_reason: None,
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                    Ok(sent_id)
                }
                .await;
                match outcome {
                    Ok(sent_id) => {
                        if first_delivered_msg_id.is_none() {
                            first_delivered_msg_id = Some(sent_id);
                        }
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
                        // The database row points at a dump message that no
                        // longer exists (for example after channel cleanup).
                        // Remove it immediately so a failed re-rip cannot
                        // leave a ghost cache entry behind.
                        let stale_key = TrackKey::new(Provider::Apple, item.id.clone())
                            .with_codec(cached.codec);
                        if let Err(delete_error) = deps.delete_track(&stale_key).await {
                            tracing::warn!(
                                track_id = %item.id,
                                error = %delete_error,
                                "Failed to remove stale cache row after dump copy failure"
                            );
                        }
                        uncached_items.push(item.clone());
                        // Any queued staging for this track is now moot: the
                        // cache row was deleted, so the re-rip supplies the
                        // zip source instead.
                        if let Some(pos) = stage_items
                            .iter()
                            .position(|stage| stage.item.id == item.id)
                        {
                            stage_items.remove(pos);
                        }
                    }
                }
            }
        }

        // Delivery metadata for the ZIP details message. Populated by
        // the reuse fast path and the rebuild block; None on cache-only.
        let mut zip_delivery: Option<ZipDeliveryInfo> = None;

        let summary = |cached_count: usize,
                       ripped_count: usize,
                       failed: Vec<FailedTrack>,
                       skipped: Vec<String>,
                       elapsed: &str,
                       zip_delivery: Option<ZipDeliveryInfo>,
                       first_msg_id: Option<i32>| {
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
                warnings: warnings.clone(),
                zip_delivery,
                first_delivered_msg_id: first_msg_id,
            }
        };

        // All-cached fast path.
        if uncached_items.is_empty() && !zip_build {
            let elapsed = format!(
                "{:.1}",
                (now_ms().saturating_sub(shared.lock().expect("job poisoned").job.start_time_ms)
                    as f64)
                    / 1000.0
            );
            return Ok(summary(
                cached_count,
                0,
                Vec::new(),
                Vec::new(),
                &elapsed,
                zip_delivery,
                first_delivered_msg_id,
            ));
        }

        // Fully-cached album with a reusable ZIP set. The parts are
        // already in the dump channel; nothing needs staging, building, or
        // re-uploading. Cache-only jobs are done; `-z` user jobs get the
        // cached parts delivered (with a cover preview when artwork is
        // available). Plain jobs already received every cached track
        // instantly in the pre-queue loop — the archive stays cached
        // without a second delivery.
        if let Some(rows) = &zip_reuse {
            if uncached_items.is_empty() {
                if !options.is_cache_only && zip_deliver {
                    let reply_to = (options.delivery_chat_id == options.chat_id)
                        .then_some(options.reply_to_message_id)
                        .flatten();
                    let total_parts = rows.len();
                    let mut delivered_all = true;
                    for row in rows {
                        match deps
                            .sink()
                            .send_dump_copy(
                                options.delivery_chat_id,
                                row.message_id,
                                reply_to,
                                total_parts > 1,
                            )
                            .await
                        {
                            Ok(sent_id) => {
                                if first_delivered_msg_id.is_none() {
                                    first_delivered_msg_id = Some(sent_id);
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "cached ZIP part delivery failed");
                                delivered_all = false;
                                break;
                            }
                        }
                    }
                    if !delivered_all {
                        // The dump message behind a cached part is gone. Purge
                        // the album's ZIP rows so the next request rebuilds
                        // instead of skipping staging and failing forever.
                        if let Err(error) = deps
                            .delete_albums(
                                Provider::Apple,
                                &options.parsed_items[0].id,
                                Some(target_codec),
                            )
                            .await
                        {
                            tracing::warn!(%error, "failed to purge undeliverable album ZIP rows");
                        }
                    }
                    if delivered_all {
                        let release_year: String = album_release_date
                            .as_deref()
                            .map(|date| date.chars().take(4).collect())
                            .unwrap_or_default();
                        let total_size: i64 = rows.iter().map(|row| row.file_size).sum();
                        let caption_meta = AlbumDetailsCaptionMetadata {
                            album: album_name.as_deref().unwrap_or_default(),
                            artist: album_artist.as_deref().unwrap_or_default(),
                            album_id: album_id.as_deref().unwrap_or_default(),
                            storefront: album_sf.as_deref().unwrap_or("us"),
                            total_tracks: tracks_to_process.len(),
                            delivered_tracks: tracks_to_process.len(),
                            size_bytes: total_size,
                            total_parts: rows.len(),
                            release_year: &release_year,
                            genre: album_genre.as_deref(),
                            record_label: album_record_label.as_deref(),
                            is_partial: false,
                            user_name: options.user_name.as_deref(),
                            user_id: options.user_id,
                            codec: None,
                        };
                        let details_caption = format_album_details_caption(&caption_meta);
                        let mut photo_delivered = false;
                        if let Some(artwork_url) =
                            album_artwork_url.as_deref().filter(|url| !url.is_empty())
                        {
                            if let Some(bytes) = deps.fetch_artwork(artwork_url).await {
                                if let Err(error) = deps
                                    .sink()
                                    .send_photo_to_chat(
                                        options.delivery_chat_id,
                                        &bytes,
                                        &details_caption,
                                    )
                                    .await
                                {
                                    tracing::warn!(%error, "cover preview send failed");
                                } else {
                                    photo_delivered = true;
                                }
                            }
                        }
                        zip_delivery = Some(ZipDeliveryInfo {
                            album: album_name.clone().unwrap_or_default(),
                            artist: album_artist.clone().unwrap_or_default(),
                            release_year,
                            total_tracks: tracks_to_process.len(),
                            delivered_tracks: tracks_to_process.len(),
                            total_parts: rows.len(),
                            size_bytes: total_size,
                            is_partial: false,
                            album_id: album_id.clone().unwrap_or_default(),
                            storefront: album_sf.clone().unwrap_or_else(|| "us".to_owned()),
                            artwork_url: album_artwork_url.clone(),
                            genre: album_genre.clone(),
                            record_label: album_record_label.clone(),
                            copyright: album_copyright.clone(),
                            photo_delivered,
                            codec: None,
                        });
                    }
                }
                let elapsed = format!(
                    "{:.1}",
                    (now_ms().saturating_sub(shared.lock().expect("job poisoned").job.start_time_ms)
                        as f64)
                        / 1000.0
                );
                // The zip workspace was created for this album job but the
                // reusable parts made it unnecessary — remove the empty dir.
                if let Some(dir) = &zip_dir {
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
                return Ok(summary(
                    cached_count,
                    0,
                    Vec::new(),
                    Vec::new(),
                    &elapsed,
                    zip_delivery,
                    first_delivered_msg_id,
                ));
            }
        }

        // Maintenance mode skips misses. Cache-only jobs still rip
        // uncached tracks, but keep the resulting audio in the dump channel
        // instead of delivering a copy to the requester.
        if !settings.can_rip_live(options.is_admin) && (!zip_build || !uncached_items.is_empty()) {
            let skipped: Vec<String> = uncached_items.iter().map(|i| i.id.clone()).collect();
            {
                let mut guard = shared.lock().expect("job poisoned");
                guard.job.skipped_count = skipped.len();
            }
            self.bus
                .emit_progress(&shared, Some("Skipping uncached tracks..."), None, None);
            let elapsed = format!(
                "{:.1}",
                (now_ms().saturating_sub(shared.lock().expect("job poisoned").job.start_time_ms)
                    as f64)
                    / 1000.0
            );
            // No staging or packaging will happen for the skipped tracks;
            // drop the zip workspace too.
            if let Some(dir) = &zip_dir {
                let _ = tokio::fs::remove_dir_all(dir).await;
            }
            return Ok(summary(
                cached_count,
                0,
                Vec::new(),
                skipped,
                &elapsed,
                zip_delivery,
                first_delivered_msg_id,
            ));
        }

        // Live rip through the queue.
        self.set_phase(&shared, JobPhase::Queued);
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

        // Lane 2 must exist before lane 1 can push items into it. The
        // dispatcher is global and lazily spawned once per orchestrator.
        let _ = self.ensure_upload_lane();

        // The job context moves into both lanes: every lane-2 item holds a
        // clone, and the finalize marker holds the last one.
        let job_ctx = Arc::new(JobContext {
            options: options.clone(),
            zip_build,
            zip_deliver,
            zip_dir: zip_dir.clone(),
            zip_sources: Arc::clone(&zip_sources),
            zip_album: album_name.clone().unwrap_or_else(|| "Album".to_owned()),
            zip_artist: album_artist
                .clone()
                .unwrap_or_else(|| "Unknown Artist".to_owned()),
            zip_album_id: options
                .parsed_items
                .first()
                .map(|item| item.id.clone())
                .unwrap_or_default(),
            zip_storefront: album_sf.clone().unwrap_or_else(|| "us".to_owned()),
            zip_genre: album_genre.clone(),
            zip_record_label: album_record_label.clone(),
            zip_copyright: album_copyright.clone(),
            zip_generation_hash: zip_generation_hash.clone(),
            zip_codec: Arc::new(std::sync::Mutex::new(None)),
            zip_artwork_url: album_artwork_url.clone().filter(|url| !url.is_empty()),
            zip_release_date: album_release_date.clone().unwrap_or_default(),
            warnings: warnings.clone(),
            atmos_warning: Arc::new(std::sync::Mutex::new(None)),
            cached_count,
            is_multi_track,
            max_collection_limit,
            capped_count,
            queue_start_time_ms: queue_start_time,
            ripped_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            failed_tracks: Arc::new(Mutex::new(Vec::new())),
            texts: Arc::new(PipelineTexts::default()),
            first_delivered_msg_id: Arc::new(Mutex::new(first_delivered_msg_id)),
        });

        // The finalize marker (last lane-2 item for this job) resolves the
        // job summary; `start_job` awaits it after the enqueue returns.
        let (summary_tx, summary_rx) = tokio::sync::oneshot::channel::<RipJobSummary>();
        let summary_tx = Arc::new(Mutex::new(Some(summary_tx)));

        // The queue task must be 'static — move everything it needs.
        let task_deps = Arc::clone(&deps);
        let task_shared = Arc::clone(&shared);
        let task_items = uncached_items;
        let task_controller = job_controller.clone();
        let task_bus = self.bus.clone();
        let task_stage_items = stage_items;
        let task_job_ctx = Arc::clone(&job_ctx);
        let task_upload_lane = self.upload_lane.clone();
        let task_summary_tx = Arc::clone(&summary_tx);

        let callback_shared = Arc::clone(&shared);
        let callback_bus = self.bus.clone();
        let on_position_change = Arc::new(move |position: u64| {
            callback_shared
                .lock()
                .expect("job poisoned")
                .job
                .queue_position = Some(position);
            callback_bus.emit_progress(
                &callback_shared,
                Some(&format!("In Queue: Position #{position}")),
                None,
                None,
            );
        });
        let callback_shared = Arc::clone(&shared);
        let callback_bus = self.bus.clone();
        let on_start = Arc::new(move || {
            {
                let mut guard = callback_shared.lock().expect("job poisoned");
                guard.job.phase = JobPhase::Processing;
                guard.job.queue_position = Some(0);
            }
            let guard = callback_shared.lock().expect("job poisoned");
            callback_bus.emit(&OrchestratorEvent::Started(&guard.job));
        });

        let task = move |queue_signal: CancellationToken| {
            Box::pin(async move {
                run_lane_one(LaneOneContext {
                    deps: task_deps,
                    bus: task_bus,
                    shared: task_shared,
                    uncached_items: &task_items,
                    job_controller: task_controller,
                    queue_signal,
                    ctx: task_job_ctx,
                    stage_items: &task_stage_items,
                    upload_lane: task_upload_lane,
                    summary_tx: task_summary_tx,
                })
                .await
            })
                as std::pin::Pin<Box<dyn std::future::Future<Output = RipJobSummary> + Send>>
        };

        // Lane 1 always resolves a summary (possibly with failures); the
        // finalize marker in lane 2 later upgrades it if it runs at all.
        let lane_one_summary = match self
            .queue
            .enqueue(
                task,
                Some(EnqueueOptions {
                    signal: Some(job_controller.clone()),
                    on_position_change: Some(on_position_change),
                    on_start: Some(on_start),
                }),
            )
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                // The task never ran (aborted while pending, queue cleared,
                // or it panicked): the rip workspace was never created, but
                // the zip workspace exists since the pre-queue phase and no
                // marker will ever clean it — remove it here. Any staged
                // downloads for the archive never happened either.
                if let Some(dir) = &zip_dir {
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
                return Err(error.into());
            }
        };

        // Wait for the finalize marker (the last lane-2 item for this job)
        // so the job only goes terminal once every upload and the ZIP are
        // done. If the marker was dropped without resolving (dispatcher
        // panic), fall back to lane 1's snapshot so `start_job` still
        // settles instead of hanging forever.
        let final_summary = match summary_rx.await {
            Ok(summary) => summary,
            Err(_) => lane_one_summary,
        };
        Ok(final_summary)
    }
}

/// Lane 1: staged-cache downloads running concurrently with the rip loop.
/// Runs as the sequential rip queue's task (one job at a time), so the
/// queue slot is held only while rips happen: each finished rip hands its
/// upload off to lane 2 (the global upload dispatcher), and the job's
/// finalize marker — pushed after the last rip — completes the archive,
/// cleans both workspaces, and resolves the summary `start_job` awaits.
///
/// Staging failures are fed back into the work feed as re-rip items, so an
/// undownloadable cache row still leaves the archive a chance to complete.
///
struct LaneOneContext<'a, D: OrchestratorDeps> {
    deps: Arc<D>,
    bus: EventBus,
    shared: Arc<Mutex<JobShared>>,
    uncached_items: &'a [ResolvedTrackItem],
    job_controller: CancellationToken,
    queue_signal: CancellationToken,
    ctx: Arc<JobContext>,
    stage_items: &'a [StageItem],
    upload_lane: Arc<Mutex<Option<tokio::sync::mpsc::Sender<LaneTask>>>>,
    summary_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<RipJobSummary>>>>,
}

/// Upload retry exhaustion is recorded as a track failure and the job
/// drains the remaining results.
async fn run_lane_one<D: OrchestratorDeps>(input: LaneOneContext<'_, D>) -> RipJobSummary {
    let LaneOneContext {
        deps,
        bus,
        shared,
        uncached_items,
        job_controller,
        queue_signal,
        ctx,
        stage_items,
        upload_lane,
        summary_tx,
    } = input;
    tracing::debug!("Rip job started from queue");

    let is_cancelled = || {
        shared.lock().expect("job poisoned").job.is_cancelled
            || job_controller.is_cancelled()
            || queue_signal.is_cancelled()
    };

    let rip_job_dir: PathBuf =
        std::env::temp_dir().join(format!("rip_job_{id}", id = cuid2::create_id()));
    let _ = tokio::fs::create_dir_all(&rip_job_dir).await;

    // The work feed: the producer streams uncached items in, the staging
    // task feeds failures back for a re-rip. The rip loop ends once both
    // senders drop (the join below settles them all).
    let (work_tx, mut work_rx) = tokio::sync::mpsc::channel::<PipelineItem>(2);

    // Producer
    let producer_tx = work_tx.clone();
    let producer = async {
        // The sender is MOVED into the block (async blocks otherwise
        // capture borrows, keeping the channel open and wedging the rip
        // loop's recv): when the producer finishes, the last work-feed
        // sender drops and the rip loop ends.
        let producer_tx = producer_tx;
        for item in uncached_items {
            if is_cancelled() {
                break;
            }
            let pipeline_item = PipelineItem {
                track_id: item.id.clone(),
                storefront: item.storefront.clone(),
                meta_title: item.title.clone(),
                meta_artist: item.artist.clone(),
                is_streamable: item.is_streamable,
            };
            if producer_tx.send(pipeline_item).await.is_err() {
                break;
            }
        }
    };

    // Staging task
    // Cached tracks queued for the archive are downloaded from the dump
    // channel into the zip workspace concurrently with the rips. A cache
    // row that cannot be materialized is re-ripped instead.
    let staging = async {
        let staging_tx = work_tx; // last clone: its drop closes the feed
        let Some(zip_dir) = ctx.zip_dir.clone() else {
            return;
        };
        for stage in stage_items {
            if is_cancelled() {
                break;
            }
            let destination = zip_dir.join(&stage.archive_filename);
            let on_download_progress: UploadProgressCallback = {
                let texts = Arc::clone(&ctx.texts);
                let shared = Arc::clone(&shared);
                let bus = bus.clone();
                let label = stage.label.clone();
                Arc::new(move |done: u64, total: u64| {
                    let text = if total > 0 {
                        format!(
                            "⬇️ Downloading from TG: <b>{}</b> <code>{}</code>",
                            html_escape(&label),
                            format_byte_progress(done, total, 12)
                        )
                    } else {
                        format!("⬇️ Downloading from TG: <b>{}</b>", html_escape(&label))
                    };
                    *texts.download.lock().expect("texts poisoned") = Some(text.clone());
                    shared.lock().expect("job poisoned").job.active_action_text =
                        Some(text.clone());
                    let (download_text, upload_text) = texts.snapshot();
                    bus.emit_progress(
                        &shared,
                        None,
                        download_text.as_deref(),
                        upload_text.as_deref(),
                    );
                })
            };
            let initial_text = format!(
                "⬇️ Downloading from TG: <b>{}</b>",
                html_escape(&stage.label)
            );
            *ctx.texts.download.lock().expect("texts poisoned") = Some(initial_text.clone());
            shared.lock().expect("job poisoned").job.active_action_text = Some(initial_text);
            let (download_text, upload_text) = ctx.texts.snapshot();
            bus.emit_progress(
                &shared,
                None,
                download_text.as_deref(),
                upload_text.as_deref(),
            );

            let download_res = deps
                .sink()
                .download_dump_file(stage.message_id, &destination, Some(&on_download_progress))
                .await;

            // Lane 1 owns the download slot only; the upload slot (lane 2)
            // is preserved by snapshotting both into the emit.
            *ctx.texts.download.lock().expect("texts poisoned") = None;
            shared.lock().expect("job poisoned").job.active_action_text = None;
            let (download_text, upload_text) = ctx.texts.snapshot();
            bus.emit_progress(
                &shared,
                None,
                download_text.as_deref(),
                upload_text.as_deref(),
            );

            match download_res {
                Ok(()) => {
                    let size = tokio::fs::metadata(&destination)
                        .await
                        .map(|m| m.len())
                        .unwrap_or(0);
                    ctx.zip_sources
                        .lock()
                        .expect("zip sources poisoned")
                        .push(ZipTrackEntry {
                            file_path: destination,
                            archive_filename: stage.archive_filename.clone(),
                            file_size: size,
                        });
                }
                Err(error) => {
                    tracing::warn!(
                        track_id = %stage.item.id,
                        %error,
                        "failed to materialize cached track for ZIP; queueing re-rip"
                    );
                    // Feed the resolved track back for a re-rip: the cache
                    // row cannot participate in the archive (the re-rip's
                    // save overwrites the stale row).
                    let pipeline_item = PipelineItem {
                        track_id: stage.item.id.clone(),
                        storefront: stage.item.storefront.clone(),
                        meta_title: stage.item.title.clone(),
                        meta_artist: stage.item.artist.clone(),
                        is_streamable: stage.item.is_streamable,
                    };
                    if staging_tx.send(pipeline_item).await.is_err() {
                        break;
                    }
                }
            }
        }
    };

    // Rip loop (lane 1)
    let rip_loop = async {
        while let Some(item) = work_rx.recv().await {
            if is_cancelled() {
                break;
            }

            if item.is_streamable == Some(false) {
                let err_msg = "Unavailable on Apple Music (not streamable)".to_string();
                let duration_ms = 0;
                {
                    let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
                    failures.push(FailedTrack {
                        id: item.track_id.clone(),
                        error: err_msg.clone(),
                        title: item.meta_title.clone(),
                        artist: item.meta_artist.clone(),
                        storefront: item.storefront.clone(),
                    });
                    shared.lock().expect("job poisoned").job.failed_count = failures.len();
                }
                tracing::warn!(
                    track_id = %item.track_id,
                    "Track is not streamable in Apple Music catalog, skipping rip"
                );
                let _ = deps
                    .log_request(RequestLog {
                        telegram_id: ctx.options.user_id,
                        chat_id: ctx.options.chat_id,
                        track_key: TrackKey::new(Provider::Apple, item.track_id.clone()),
                        is_cache_hit: false,
                        duration_ms: Some(duration_ms),
                        status: "failed".to_string(),
                        error_reason: Some(err_msg.clone()),
                    })
                    .await;
                let (download_text, upload_text) = ctx.texts.snapshot();
                bus.emit_progress(
                    &shared,
                    None,
                    download_text.as_deref(),
                    upload_text.as_deref(),
                );
                continue;
            }

            let track_start_time = now_ms();
            let track_label = match (&item.meta_title, &item.meta_artist) {
                // An empty title or artist counts as absent.
                (Some(title), Some(artist)) if !title.is_empty() && !artist.is_empty() => {
                    format!("{title} - {artist}")
                }
                _ => format!("Track {}", item.track_id),
            };
            let dynamic_label = Arc::new(Mutex::new(track_label.clone()));
            let update_single_track_header =
                !ctx.is_multi_track && item.meta_title.is_none() && item.meta_artist.is_none();

            let on_progress: RipProgressCallback = {
                let texts = Arc::clone(&ctx.texts);
                let shared = Arc::clone(&shared);
                let bus = bus.clone();
                let dynamic_label = Arc::clone(&dynamic_label);
                Arc::new(move |status, downloaded, total| {
                    if let Some(label) = stream_display_label(status) {
                        *dynamic_label.lock().expect("label poisoned") = label.to_owned();
                        if update_single_track_header {
                            shared.lock().expect("job poisoned").job.job_header =
                                format!("<b>{}</b>", html_escape(label));
                        }
                    }
                    let track_label = dynamic_label.lock().expect("label poisoned").clone();
                    let text = match (downloaded, total) {
                        (Some(d), Some(t)) => format!(
                            "⬇️ Downloading: <b>{}</b> <code>{}</code>",
                            html_escape(&track_label),
                            format_byte_progress(d, t, 12)
                        ),
                        _ => {
                            let lower = status.to_lowercase();
                            if lower.contains("tag") {
                                format!("🏷️ Tagging: <b>{}</b>", html_escape(&track_label))
                            } else if lower.contains("decrypt") || lower.contains("remux") {
                                format!("🔓 Decrypting: <b>{}</b>", html_escape(&track_label))
                            } else if lower.contains("connect") || lower.contains("key") {
                                format!("🌐 Connecting: <b>{}</b>", html_escape(&track_label))
                            } else if lower.contains("meta") || lower.contains("fetch") {
                                format!("🔍 Resolving: <b>{}</b>", html_escape(&track_label))
                            } else if lower.contains("download") {
                                format!("⬇️ Downloading: <b>{}</b>", html_escape(&track_label))
                            } else {
                                format!(
                                    "⬇️ <b>{}:</b> {}",
                                    html_escape(&track_label),
                                    html_escape(status)
                                )
                            }
                        }
                    };
                    *texts.download.lock().expect("texts poisoned") = Some(text.clone());
                    shared.lock().expect("job poisoned").job.active_action_text =
                        Some(text.clone());
                    let (download_text, upload_text) = texts.snapshot();
                    bus.emit_progress(
                        &shared,
                        None,
                        download_text.as_deref(),
                        upload_text.as_deref(),
                    );
                })
            };

            let storefront = item.storefront.clone().unwrap_or_else(|| "us".to_string());
            match deps
                .rip(
                    &item.track_id,
                    Some(&on_progress),
                    &storefront,
                    queue_signal.clone(),
                    Some(&rip_job_dir),
                    ctx.options.codec_preference,
                )
                .await
            {
                Ok(rip_result) => {
                    // activeDownloadText = ''.
                    *ctx.texts.download.lock().expect("texts poisoned") = None;
                    shared.lock().expect("job poisoned").job.active_action_text = None;
                    let (download_text, upload_text) = ctx.texts.snapshot();
                    bus.emit_progress(
                        &shared,
                        None,
                        download_text.as_deref(),
                        upload_text.as_deref(),
                    );

                    // Hand the finished rip to lane 2 — FIFO behind this
                    // job's earlier items. Back-pressure (16 pending
                    // items) pauses ripping until uploads drain.
                    if let Ok(mut codec) = ctx.zip_codec.lock() {
                        // Keep the strongest codec seen across the album's
                        // rips: ALAC (lossless) > Atmos ec-3 > lossy AAC.
                        let rank = |c: &str| match c {
                            "alac" => 3,
                            "ec-3" => 2,
                            _ => 1,
                        };
                        let better = match codec.as_deref() {
                            None => true,
                            Some(current) => rank(&rip_result.codec) > rank(current),
                        };
                        if better {
                            *codec = Some(rip_result.codec.clone());
                        }
                    }
                    if ctx.options.codec_preference == CodecPreference::Atmos
                        && rip_result.codec != "ec-3"
                    {
                        let mut warnings = ctx.atmos_warning.lock().expect("atmos poisoned");
                        if warnings.is_none() {
                            *warnings = Some(
                                "Dolby Atmos was not available for this track; delivered the highest available quality instead."
                                    .to_owned(),
                            );
                        }
                    }
                    let upload_item = PipelineRipResult {
                        track_id: item.track_id.clone(),
                        rip_result,
                        start_time_ms: track_start_time,
                    };
                    let item_deps = Arc::clone(&deps);
                    let item_bus = bus.clone();
                    let item_shared = Arc::clone(&shared);
                    let item_ctx = Arc::clone(&ctx);
                    let item_controller = job_controller.clone();
                    let lane = Arc::clone(&upload_lane);
                    let pushed = push_lane_task(&lane, "upload_track", move || {
                        Box::pin(async move {
                            run_upload_item(
                                item_deps,
                                item_bus,
                                item_shared,
                                item_ctx,
                                item_controller,
                                upload_item,
                            )
                            .await;
                        })
                    })
                    .await;
                    if !pushed {
                        // The dispatcher is gone (runtime shutdown); the
                        // upload can never run, so stop ripping.
                        break;
                    }
                }
                Err(err) => {
                    *ctx.texts.download.lock().expect("texts poisoned") = None;
                    if is_cancelled() {
                        break;
                    }

                    let err_msg = err.to_string();
                    let duration_ms = (now_ms() - track_start_time) as i64;
                    {
                        let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
                        failures.push(FailedTrack {
                            id: item.track_id.clone(),
                            error: err_msg.clone(),
                            title: item.meta_title.clone(),
                            artist: item.meta_artist.clone(),
                            storefront: item.storefront.clone(),
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
                            telegram_id: ctx.options.user_id,
                            chat_id: ctx.options.chat_id,
                            track_key: TrackKey::new(Provider::Apple, item.track_id.clone()),
                            is_cache_hit: false,
                            duration_ms: Some(duration_ms),
                            status: "failed".to_string(),
                            error_reason: Some(err_msg.clone()),
                        })
                        .await;

                    let (download_text, upload_text) = ctx.texts.snapshot();
                    bus.emit_progress(
                        &shared,
                        Some("Processing next track..."),
                        download_text.as_deref(),
                        upload_text.as_deref(),
                    );

                    // Circuit breaker: abort the remaining batch when the
                    // mirror looks offline.
                    let is_mirror_down = [
                        "Mirror /status check timed out",
                        "Mirror health check failed",
                        "Lossless wrapper is currently offline",
                        "Mirror manifest lookup timed out",
                        "Mirror service is currently offline",
                        "Failed to connect to mirror stream",
                    ]
                    .iter()
                    .any(|phrase| err_msg.contains(phrase));
                    if is_mirror_down {
                        {
                            let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
                            failures.push(FailedTrack {
                                id: "Remaining tracks".to_string(),
                                error:
                                    "Mirror service offline / unreachable (stopped remaining batch)"
                                        .to_string(),
                                title: None,
                                artist: None,
                                storefront: None,
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

    // All three stages settle (channel drops propagate end-to-end).
    let _: ((), (), ()) = tokio::join!(producer, staging, rip_loop);

    // Finalize marker: the last lane-2 item for this job (FIFO after all
    // of its track uploads). Builds the archive, publishes it, cleans both
    // workspaces, and resolves the summary `start_job` awaits. Pushed even
    // after cancellation — the workspace still needs cleanup.
    let marker_deps = Arc::clone(&deps);
    let marker_bus = bus.clone();
    let marker_shared = Arc::clone(&shared);
    let marker_ctx = Arc::clone(&ctx);
    let marker_controller = job_controller.clone();
    let marker_job_dir = rip_job_dir;
    let marker_summary_tx = Arc::clone(&summary_tx);
    let marker_lane = Arc::clone(&upload_lane);
    let _ = push_lane_task(&marker_lane, "finalize_job", move || {
        Box::pin(async move {
            finalize_job(
                marker_deps,
                marker_bus,
                marker_shared,
                marker_ctx,
                marker_controller,
                marker_job_dir,
                marker_summary_tx,
            )
            .await;
        })
    })
    .await;

    // Lane-1 fallback summary (counts at the last rip). Normally
    // superseded by the finalize marker's resolution; used only if the
    // marker was dropped without resolving (dispatcher panic).
    build_job_summary(&shared, &ctx, None)
}

/// Snapshot the shared job state into a `RipJobSummary` for the given
/// lane results. Elapsed time is measured from queue admission.
fn build_job_summary(
    shared: &Arc<Mutex<JobShared>>,
    ctx: &JobContext,
    zip_delivery: Option<ZipDeliveryInfo>,
) -> RipJobSummary {
    let total_elapsed_sec = format!(
        "{:.1}",
        (now_ms().saturating_sub(ctx.queue_start_time_ms)) as f64 / 1000.0
    );
    let failed = ctx.failed_tracks.lock().expect("failures poisoned").clone();
    let first_msg_id = *ctx.first_delivered_msg_id.lock().unwrap();
    let guard = shared.lock().expect("job poisoned");
    RipJobSummary {
        job_id: guard.job.id.clone(),
        job_header: guard.job.job_header.clone(),
        total_tracks: guard.job.total_tracks,
        cached_count: ctx.cached_count,
        ripped_count: ctx.ripped_count.load(std::sync::atomic::Ordering::SeqCst),
        failed_count: failed.len(),
        failed_tracks: failed,
        skipped_uncached_tracks: Vec::new(),
        total_elapsed_sec,
        capped_count: ctx.capped_count,
        max_collection_limit: ctx.max_collection_limit,
        is_cache_only: ctx.options.is_cache_only,
        is_group: ctx.options.is_group,
        warnings: ctx
            .atmos_warning
            .lock()
            .expect("atmos poisoned")
            .clone()
            .into_iter()
            .chain(ctx.warnings.clone())
            .collect(),
        zip_delivery,
        first_delivered_msg_id: first_msg_id,
    }
}

/// One lane-2 track item: dump upload (retries/backoff), cache row, user
/// copy (unless the archive replaces individual delivery), request log,
/// file cleanup, and — for zip jobs — staging the audio into the zip
/// workspace for the finalize marker to package.
async fn run_upload_item<D: OrchestratorDeps>(
    deps: Arc<D>,
    bus: EventBus,
    shared: Arc<Mutex<JobShared>>,
    ctx: Arc<JobContext>,
    job_controller: CancellationToken,
    upload_item: PipelineRipResult,
) {
    let uploaded_ok = upload_one(&deps, &bus, &shared, &ctx, &job_controller, &upload_item).await;

    if uploaded_ok && ctx.zip_build {
        if let Some(dir) = &ctx.zip_dir {
            let filename = format!(
                "{:02} - {} - {} [{}].m4a",
                upload_item.rip_result.track_number,
                sanitize_archive_filename(&upload_item.rip_result.title),
                sanitize_archive_filename(&upload_item.rip_result.artist),
                upload_item.track_id
            );
            let destination = dir.join(&filename);
            if let Err(error) =
                tokio::fs::copy(&upload_item.rip_result.file_path, &destination).await
            {
                tracing::warn!(%error, track_id = %upload_item.track_id, "failed to stage track for ZIP");
            } else {
                let size = tokio::fs::metadata(&destination)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(0);
                ctx.zip_sources
                    .lock()
                    .expect("zip sources poisoned")
                    .push(ZipTrackEntry {
                        file_path: destination,
                        archive_filename: filename,
                        file_size: size,
                    });
            }
        }
    }

    delete_file_if_exists(&upload_item.rip_result.file_path).await;
}

/// The lane-2 finalize marker: the last item for a job. Packages the
/// staged sources into a (possibly split) archive, publishes it, cleans
/// both workspaces, and resolves the job summary on the oneshot
/// `start_job` awaits.
///
/// Publication gating (the always-zip rule):
/// - Complete archive → always uploaded to the dump and `save_album`'d
///   (the cache), regardless of who asked; user copies/details only when
///   the user asked for the archive (`zip_deliver`) on a user job.
/// - Incomplete archive → never cached; delivered as `[Partial].zip`
///   straight to the delivery chat only for `zip_deliver` user jobs;
///   otherwise skipped entirely.
async fn finalize_job<D: OrchestratorDeps>(
    deps: Arc<D>,
    bus: EventBus,
    shared: Arc<Mutex<JobShared>>,
    ctx: Arc<JobContext>,
    job_controller: CancellationToken,
    rip_job_dir: PathBuf,
    summary_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<RipJobSummary>>>>,
) {
    let zip_delivery = finalize_zip(&deps, &bus, &shared, &ctx, &job_controller).await;

    // Cleanup both workspaces. Runs on every path, including cancellation.
    let _ = tokio::fs::remove_dir_all(&rip_job_dir).await;
    if let Some(dir) = &ctx.zip_dir {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    let summary = build_job_summary(&shared, &ctx, zip_delivery);
    if let Some(tx) = summary_tx.lock().expect("summary sender poisoned").take() {
        let _ = tx.send(summary);
    }
}

/// The archive half of the finalize marker. Returns the delivery info for
/// the summary when parts reached the user, `None` otherwise.
async fn finalize_zip<D: OrchestratorDeps>(
    deps: &Arc<D>,
    bus: &EventBus,
    shared: &Arc<Mutex<JobShared>>,
    ctx: &Arc<JobContext>,
    job_controller: &CancellationToken,
) -> Option<ZipDeliveryInfo> {
    let options = &ctx.options;
    if !ctx.zip_build {
        return None;
    }
    let Some(dir) = &ctx.zip_dir else {
        return None;
    };

    let entries = ctx
        .zip_sources
        .lock()
        .expect("zip sources poisoned")
        .clone();
    let failures = ctx.failed_tracks.lock().expect("failures poisoned").clone();
    let expected_tracks = shared.lock().expect("job poisoned").job.total_tracks;
    let complete = failures.is_empty() && entries.len() == expected_tracks;
    // A cache job publishes only complete ZIPs; a user ZIP request may
    // receive a partial archive directly, but it is never persisted in
    // the dump. Plain jobs (no `-z`) never receive the archive itself —
    // the complete one is still cached for later `-z` requests.
    let should_publish =
        complete || (ctx.zip_deliver && !options.is_cache_only && !entries.is_empty());
    if !should_publish || entries.is_empty() {
        return None;
    }

    // Metadata for the ZIP details message (user deliveries).
    let delivered_track_count = entries.len();
    let mut delivered_part_count = 0usize;
    let mut delivered_size_bytes = 0i64;
    // Fetch the cover once; it feeds both the archive entry and (for
    // user jobs) the chat preview. Any failure degrades to a coverless
    // archive.
    let cover_bytes = match &ctx.zip_artwork_url {
        Some(url) => deps.fetch_artwork(url).await,
        None => None,
    };
    // Telegram document thumbnail (320x320 artwork, best-effort: failures
    // degrade to a thumbless document).
    let thumb_path = match &ctx.zip_artwork_url {
        Some(url) if !url.is_empty() => {
            let thumb_url = artwork_url_at_size(url, 320);
            match deps.fetch_artwork(&thumb_url).await {
                Some(bytes) if !bytes.is_empty() => {
                    let path = dir.join("cover_thumb.jpg");
                    match tokio::fs::write(&path, bytes).await {
                        Ok(()) => Some(path),
                        Err(error) => {
                            tracing::warn!(%error, "failed to stage ZIP thumbnail");
                            None
                        }
                    }
                }
                _ => None,
            }
        }
        _ => None,
    };
    let thumb_path_str = thumb_path
        .as_deref()
        .map(|path| path.to_string_lossy().into_owned());
    let cover_path = match &cover_bytes {
        Some(bytes) => {
            let path = dir.join("cover.jpg");
            match tokio::fs::write(&path, bytes).await {
                Ok(()) => Some(path),
                Err(error) => {
                    tracing::warn!(%error, "failed to stage cover for ZIP");
                    None
                }
            }
        }
        None => None,
    };
    let mut entries = entries;
    entries.sort_by(|a, b| a.archive_filename.cmp(&b.archive_filename));
    let zip_codec = ctx
        .zip_codec
        .lock()
        .expect("zip codec poisoned")
        .clone()
        .unwrap_or_else(|| "alac".to_owned());
    let plan_result = plan_zip_parts_with_codec(
        &ctx.zip_artist,
        &ctx.zip_album,
        &ctx.zip_release_date,
        &entries,
        cover_path.clone(),
        TELEGRAM_SPLIT_THRESHOLD_BYTES,
        &zip_codec,
    );

    // Before republishing complete parts, drop the previous rows so a
    // shrinking part count cannot leave stale parts behind. The upsert
    // below re-saves each fresh part.
    let album_codec = if ctx.options.codec_preference == CodecPreference::Atmos {
        Codec::Ec3
    } else {
        zip_codec.parse::<Codec>().unwrap_or(Codec::Alac)
    };
    if complete {
        if let Err(error) = deps
            .delete_albums(Provider::Apple, &ctx.zip_album_id, Some(album_codec))
            .await
        {
            tracing::warn!(%error, "failed to purge stale album ZIP rows");
        }
    }
    match plan_result {
        Ok(plans) => {
            for original_plan in plans {
                let mut plan = original_plan;
                if !complete {
                    plan.archive_filename = plan
                        .archive_filename
                        .strip_suffix(".zip")
                        .map(|name| format!("{name} [Partial].zip"))
                        .unwrap_or_else(|| format!("{} [Partial]", plan.archive_filename));
                }
                let output = dir.join(&plan.archive_filename);
                let zip_title = if plan.total_parts > 1 {
                    format!(
                        "{} (Part {}/{})",
                        ctx.zip_album, plan.part_index, plan.total_parts
                    )
                } else {
                    ctx.zip_album.clone()
                };

                let build = tokio::task::spawn_blocking({
                    let output = output.clone();
                    let plan = plan.clone();
                    let cancel = job_controller.clone();
                    let bus = bus.clone();
                    let shared = Arc::clone(shared);
                    let texts = Arc::clone(&ctx.texts);
                    let zip_title = zip_title.clone();
                    move || {
                        let last_emit = std::sync::Mutex::new(
                            std::time::Instant::now()
                                .checked_sub(std::time::Duration::from_secs(1))
                                .unwrap_or_else(std::time::Instant::now),
                        );
                        let cb = |written: u64, total: u64| {
                            let mut last = last_emit.lock().expect("last_emit poisoned");
                            if last.elapsed() >= std::time::Duration::from_millis(500)
                                || (total > 0 && written >= total)
                            {
                                *last = std::time::Instant::now();
                                let progress_bar = format_byte_progress(written, total, 12);
                                let text = format!(
                                    "📦 Zipping: <b>{}</b> <code>{}</code>",
                                    html_escape(&zip_title),
                                    progress_bar
                                );
                                *texts.upload.lock().expect("texts poisoned") = Some(text.clone());
                                shared.lock().expect("job poisoned").job.active_action_text =
                                    Some(text.clone());
                                let (download_text, upload_text) = texts.snapshot();
                                bus.emit_progress(
                                    &shared,
                                    None,
                                    download_text.as_deref(),
                                    upload_text.as_deref(),
                                );
                            }
                        };
                        let initial_text =
                            format!("📦 Zipping: <b>{}</b>", html_escape(&zip_title));
                        *texts.upload.lock().expect("texts poisoned") = Some(initial_text.clone());
                        shared.lock().expect("job poisoned").job.active_action_text =
                            Some(initial_text);
                        let (download_text, upload_text) = texts.snapshot();
                        bus.emit_progress(
                            &shared,
                            None,
                            download_text.as_deref(),
                            upload_text.as_deref(),
                        );

                        let progress_ref: &dyn Fn(u64, u64) = &cb;
                        let res =
                            create_zip_archive(&output, &plan, Some(progress_ref), Some(&cancel));

                        *texts.upload.lock().expect("texts poisoned") = None;
                        shared.lock().expect("job poisoned").job.active_action_text = None;
                        let (download_text, upload_text) = texts.snapshot();
                        bus.emit_progress(
                            &shared,
                            None,
                            download_text.as_deref(),
                            upload_text.as_deref(),
                        );

                        res
                    }
                })
                .await;
                let Ok(Ok(size)) = build else {
                    tracing::warn!(album_id = %ctx.zip_album_id, "ZIP creation failed");
                    continue;
                };
                if size > TELEGRAM_SPLIT_THRESHOLD_BYTES {
                    tracing::warn!(size, "ZIP exceeds Telegram upload ceiling");
                    let _ = tokio::fs::remove_file(&output).await;
                    continue;
                }
                let album_codec = if ctx.options.codec_preference == CodecPreference::Atmos {
                    Codec::Ec3
                } else {
                    ctx.zip_codec
                        .lock()
                        .ok()
                        .and_then(|c| c.as_deref().and_then(|s| s.parse().ok()))
                        .unwrap_or(Codec::Alac)
                };
                let caption = format_zip_dump_caption(
                    &DumpZipCaptionMetadata {
                        provider: Provider::Apple,
                        album_id: &ctx.zip_album_id,
                        codec: Some(album_codec.as_str()),
                        album: &ctx.zip_album,
                        artist: &ctx.zip_artist,
                        filename: &plan.archive_filename,
                        part_index: plan.part_index as i32,
                        total_parts: plan.total_parts as i32,
                        generation_hash: ctx.zip_generation_hash.as_deref().unwrap_or(""),
                    },
                    complete,
                    failures.len(),
                );

                let on_zip_upload: UploadProgressCallback = {
                    let last_emit = Arc::new(Mutex::new(
                        std::time::Instant::now()
                            .checked_sub(std::time::Duration::from_secs(1))
                            .unwrap_or_else(std::time::Instant::now),
                    ));
                    let bus = bus.clone();
                    let shared = Arc::clone(shared);
                    let texts = Arc::clone(&ctx.texts);
                    let zip_title = zip_title.clone();
                    Arc::new(move |uploaded: u64, total: u64| {
                        let mut last = last_emit.lock().expect("last_emit poisoned");
                        if last.elapsed() >= std::time::Duration::from_millis(500)
                            || (total > 0 && uploaded >= total)
                        {
                            *last = std::time::Instant::now();
                            let progress_bar = format_byte_progress(uploaded, total, 12);
                            let text = format!(
                                "⬆️ Uploading: <b>{}</b> <code>{}</code>",
                                html_escape(&zip_title),
                                progress_bar
                            );
                            *texts.upload.lock().expect("texts poisoned") = Some(text.clone());
                            shared.lock().expect("job poisoned").job.active_action_text =
                                Some(text.clone());
                            let (download_text, upload_text) = texts.snapshot();
                            bus.emit_progress(
                                &shared,
                                None,
                                download_text.as_deref(),
                                upload_text.as_deref(),
                            );
                        }
                    })
                };
                let initial_upload_text =
                    format!("⬆️ Uploading: <b>{}</b>", html_escape(&zip_title));
                *ctx.texts.upload.lock().expect("texts poisoned") =
                    Some(initial_upload_text.clone());
                shared.lock().expect("job poisoned").job.active_action_text =
                    Some(initial_upload_text);
                let (download_text, upload_text) = ctx.texts.snapshot();
                bus.emit_progress(
                    shared,
                    None,
                    download_text.as_deref(),
                    upload_text.as_deref(),
                );

                let output_path = output.to_string_lossy().into_owned();
                // Complete archives go to the dump (the cache); partial
                // ones are only ever sent directly to the delivery chat.
                let upload = if complete {
                    deps.sink()
                        .send_document_to_dump(
                            &output_path,
                            thumb_path_str.as_deref(),
                            &caption,
                            Some(&on_zip_upload),
                        )
                        .await
                } else {
                    let res = deps
                        .sink()
                        .send_document_to_chat(
                            options.delivery_chat_id,
                            &output_path,
                            thumb_path_str.as_deref(),
                            &caption,
                            Some(&on_zip_upload),
                        )
                        .await;
                    match res {
                        Ok(sent_id) => {
                            let mut guard = ctx.first_delivered_msg_id.lock().unwrap();
                            if guard.is_none() {
                                *guard = Some(sent_id);
                            }
                            Ok(None)
                        }
                        Err(err) => Err(err),
                    }
                };

                *ctx.texts.upload.lock().expect("texts poisoned") = None;
                shared.lock().expect("job poisoned").job.active_action_text = None;
                let (download_text, upload_text) = ctx.texts.snapshot();
                bus.emit_progress(
                    shared,
                    None,
                    download_text.as_deref(),
                    upload_text.as_deref(),
                );
                match upload {
                    Ok(Some(upload)) if complete => {
                        // Archive copies go to the user only for `-z`
                        // user jobs; plain jobs got the individual tracks.
                        if ctx.zip_deliver && !options.is_cache_only {
                            delivered_part_count += 1;
                            delivered_size_bytes += size as i64;
                            let reply_to = if options.delivery_chat_id == options.chat_id {
                                options.reply_to_message_id
                            } else {
                                None
                            };
                            match deps
                                .sink()
                                .send_dump_copy(
                                    options.delivery_chat_id,
                                    upload.message_id,
                                    reply_to,
                                    plan.total_parts > 1,
                                )
                                .await
                            {
                                Ok(sent_id) => {
                                    let mut guard = ctx.first_delivered_msg_id.lock().unwrap();
                                    if guard.is_none() {
                                        *guard = Some(sent_id);
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "ZIP DM delivery failed");
                                }
                            }
                        }
                        let _ = deps
                            .save_album(AlbumUpload {
                                provider: Provider::Apple,
                                album_id: ctx.zip_album_id.clone(),
                                codec: album_codec,
                                part_index: plan.part_index as i32,
                                total_parts: plan.total_parts as i32,
                                message_id: upload.message_id,
                                file_id: upload.file_id,
                                file_unique_id: upload.file_unique_id,
                                file_size: size as i64,
                                file_name: plan.archive_filename.clone(),
                                generation_hash: ctx
                                    .zip_generation_hash
                                    .clone()
                                    .unwrap_or_default(),
                            })
                            .await;
                    }
                    Ok(_) => {
                        // Partial user deliveries land here (sent straight
                        // to the delivery chat).
                        if !complete && !options.is_cache_only {
                            delivered_part_count += 1;
                            delivered_size_bytes += size as i64;
                        }
                    }
                    Err(error) => tracing::warn!(%error, "ZIP upload failed"),
                }
            }
        }
        Err(error) => tracing::warn!(%error, "ZIP planning failed"),
    }
    if ctx.zip_deliver && !options.is_cache_only && delivered_part_count > 0 {
        let release_year: String = ctx.zip_release_date.chars().take(4).collect();
        let zip_codec = ctx.zip_codec.lock().expect("zip codec poisoned").clone();
        let caption_meta = AlbumDetailsCaptionMetadata {
            album: &ctx.zip_album,
            artist: &ctx.zip_artist,
            album_id: &ctx.zip_album_id,
            storefront: &ctx.zip_storefront,
            total_tracks: expected_tracks,
            delivered_tracks: delivered_track_count,
            size_bytes: delivered_size_bytes,
            total_parts: delivered_part_count,
            release_year: &release_year,
            genre: ctx.zip_genre.as_deref(),
            record_label: ctx.zip_record_label.as_deref(),
            is_partial: !complete,
            user_name: options.user_name.as_deref(),
            user_id: options.user_id,
            codec: zip_codec.as_deref(),
        };
        let details_caption = format_album_details_caption(&caption_meta);
        let mut photo_delivered = false;
        if let Some(bytes) = &cover_bytes {
            if let Err(error) = deps
                .sink()
                .send_photo_to_chat(options.delivery_chat_id, bytes, &details_caption)
                .await
            {
                tracing::warn!(%error, "cover preview send failed");
            } else {
                photo_delivered = true;
            }
        }
        return Some(ZipDeliveryInfo {
            album: ctx.zip_album.clone(),
            artist: ctx.zip_artist.clone(),
            release_year,
            total_tracks: expected_tracks,
            delivered_tracks: delivered_track_count,
            total_parts: delivered_part_count,
            size_bytes: delivered_size_bytes,
            is_partial: !complete,
            album_id: ctx.zip_album_id.clone(),
            storefront: ctx.zip_storefront.clone(),
            artwork_url: ctx.zip_artwork_url.clone(),
            genre: ctx.zip_genre.clone(),
            record_label: ctx.zip_record_label.clone(),
            copyright: ctx.zip_copyright.clone(),
            photo_delivered,
            codec: ctx.zip_codec.lock().expect("zip codec poisoned").clone(),
        });
    }
    None
}

/// Best-effort cleanup for a cancellation after an upload has completed.
async fn rollback_cancelled<D: OrchestratorDeps>(
    deps: &Arc<D>,
    track_id: &str,
    dump_message_id: i64,
    delete_record: bool,
    codec: Option<Codec>,
) {
    let dump_removed = deps
        .sink()
        .delete_dump_messages(&[dump_message_id])
        .await
        .is_ok();
    if dump_removed && delete_record {
        let mut key = TrackKey::new(Provider::Apple, track_id);
        if let Some(c) = codec {
            key = key.with_codec(c);
        }
        let _ = deps.delete_track(&key).await;
    }
}

/// One upload iteration: caption → send (retries + backoff) → save → copy →
/// log. Upload-retries-exhausted is recorded as a track failure here, which
/// keeps the job alive to drain remaining results while preserving the
/// failure in the summary.
///
/// Runs on lane 2, so it only checks the job's own cancellation token:
/// rip-queue lifecycle tokens (position, queue abort) do not apply.
async fn upload_one<D: OrchestratorDeps>(
    deps: &Arc<D>,
    bus: &EventBus,
    shared: &Arc<Mutex<JobShared>>,
    ctx: &Arc<JobContext>,
    job_controller: &CancellationToken,
    upload_item: &PipelineRipResult,
) -> bool {
    let options = &ctx.options;
    let texts = &ctx.texts;
    let track_id = upload_item.track_id.clone();
    let rip_result = &upload_item.rip_result;
    let track_label = format!("{} - {}", rip_result.title, rip_result.artist);
    let is_cancelled =
        || shared.lock().expect("job poisoned").job.is_cancelled || job_controller.is_cancelled();

    let caption = format_dump_caption(&DumpCaptionMetadata {
        track_key: TrackKey::new(Provider::Apple, track_id.clone()),
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
    let plain_caption = format!(
        "{} - {}\n{}",
        rip_result.title, rip_result.artist, rip_result.album
    );
    let mut current_caption = caption.clone();
    let mut used_plain_caption = false;

    let upload_text = format!("⬆️ Uploading: <b>{}</b>", html_escape(&track_label));
    *texts.upload.lock().expect("texts poisoned") = Some(upload_text.clone());
    shared.lock().expect("job poisoned").job.active_action_text = Some(upload_text);
    let (download_text, upload_text) = texts.snapshot();
    bus.emit_progress(
        shared,
        None,
        download_text.as_deref(),
        upload_text.as_deref(),
    );

    // Send with configured retries.  The initial call is attempt zero, so
    // `max_retries + 1` calls are made in the ordinary case.
    let max_retries = deps.upload_max_retries();
    enum SendOutcome {
        Audio(DumpUpload),
        /// Send succeeded but the message carried no audio media.
        NotAudio,
    }
    let mut outcome: Option<SendOutcome> = None;
    'upload: for attempt in 0..=max_retries {
        let on_upload: UploadProgressCallback = {
            let texts = Arc::clone(texts);
            let shared = Arc::clone(shared);
            let bus = bus.clone();
            let label = track_label.clone();
            Arc::new(move |uploaded, total| {
                let prog = format_byte_progress(uploaded, total, 12);
                let text = format!(
                    "⬆️ Uploading: <b>{}</b> <code>{}</code>",
                    html_escape(&label),
                    prog
                );
                *texts.upload.lock().expect("texts poisoned") = Some(text.clone());
                shared.lock().expect("job poisoned").job.active_action_text = Some(text.clone());
                let (download_text, upload_text) = texts.snapshot();
                bus.emit_progress(
                    &shared,
                    None,
                    download_text.as_deref(),
                    upload_text.as_deref(),
                );
            })
        };

        match deps
            .sink()
            .send_audio_to_dump(
                &rip_result.file_path,
                &rip_result.title,
                &rip_result.artist,
                rip_result.duration,
                &current_caption,
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
                    // Stop retrying without recording a failure.
                    break 'upload;
                }
                if upload_err.to_string().contains("ENTITY_BOUNDS_INVALID") && !used_plain_caption {
                    used_plain_caption = true;
                    current_caption = plain_caption.clone();
                    continue 'upload;
                }
                if attempt < max_retries {
                    let jitter = 0.8 + (now_ms() % 400) as f64 / 1000.0;
                    let delay =
                        deps.upload_retry_base_ms() as f64 * 2f64.powi(attempt as i32) * jitter;
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
                        attempts = max_retries + 1,
                        error = %upload_err,
                        "All upload retries exhausted for track"
                    );
                    // Record the track failure and keep the job alive so
                    // later tracks still upload.
                    {
                        let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
                        failures.push(FailedTrack {
                            id: track_id.clone(),
                            error: upload_err.to_string(),
                            title: Some(rip_result.title.clone()),
                            artist: Some(rip_result.artist.clone()),
                            storefront: None,
                        });
                        shared.lock().expect("job poisoned").job.failed_count = failures.len();
                    }
                    *texts.upload.lock().expect("texts poisoned") = None;
                    shared.lock().expect("job poisoned").job.active_action_text = None;
                    let (download_text, upload_text) = texts.snapshot();
                    bus.emit_progress(
                        shared,
                        None,
                        download_text.as_deref(),
                        upload_text.as_deref(),
                    );
                    return false;
                }
            }
        }
    }

    let Some(outcome) = outcome else {
        // Cancelled mid-retries: stop without recording a failure.
        *texts.upload.lock().expect("texts poisoned") = None;
        shared.lock().expect("job poisoned").job.active_action_text = None;
        let (download_text, upload_text) = texts.snapshot();
        bus.emit_progress(
            shared,
            None,
            download_text.as_deref(),
            upload_text.as_deref(),
        );
        return false;
    };

    // A successful send with no audio media records a track failure (no
    // request log) unless the job was cancelled.
    let dump_upload = match outcome {
        SendOutcome::Audio(dump_upload) => dump_upload,
        SendOutcome::NotAudio => {
            if !is_cancelled() {
                let err_msg = "Upload failed: no audio media returned";
                {
                    let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
                    failures.push(FailedTrack {
                        id: track_id.clone(),
                        error: err_msg.to_string(),
                        title: Some(rip_result.title.clone()),
                        artist: Some(rip_result.artist.clone()),
                        storefront: None,
                    });
                    shared.lock().expect("job poisoned").job.failed_count = failures.len();
                }
                tracing::error!(track_id = %track_id, "Track upload failed: no audio media returned");
            }
            *texts.upload.lock().expect("texts poisoned") = None;
            shared.lock().expect("job poisoned").job.active_action_text = None;
            let (download_text, upload_text) = texts.snapshot();
            bus.emit_progress(
                shared,
                None,
                download_text.as_deref(),
                upload_text.as_deref(),
            );
            return false;
        }
    };

    // Post-upload block: save + copy + log share one try/catch — any
    // failure records the track failure and continues.
    let post_upload: Result<i64, String> = async {
        if is_cancelled() {
            let _ = deps
                .sink()
                .delete_dump_messages(&[dump_upload.message_id])
                .await;
            return Err("cancelled".to_owned());
        }
        deps.save_track(SaveTrackInput::from_rip_result(
            &track_id,
            rip_result,
            dump_upload.message_id,
            &dump_upload.file_id,
            &dump_upload.file_unique_id,
        ))
        .await
        .map_err(|e| e.to_string())?;

        if is_cancelled() {
            let rip_codec = rip_result.codec.parse::<Codec>().ok();
            rollback_cancelled(deps, &track_id, dump_upload.message_id, true, rip_codec).await;
            return Err("cancelled".to_owned());
        }

        // The user copy is skipped when the archive replaces individual
        // delivery (`zip_deliver`) or on cache-only jobs.
        if !options.is_cache_only && !ctx.zip_deliver {
            let reply_to = (options.delivery_chat_id == options.chat_id)
                .then_some(options.reply_to_message_id)
                .flatten();
            let sent_id = deps
                .sink()
                .send_dump_copy(
                    options.delivery_chat_id,
                    dump_upload.message_id,
                    reply_to,
                    ctx.is_multi_track,
                )
                .await
                .map_err(|e| e.to_string())?;
            let mut guard = ctx.first_delivered_msg_id.lock().unwrap();
            if guard.is_none() {
                *guard = Some(sent_id);
            }
        }

        if is_cancelled() {
            rollback_cancelled(
                deps,
                &track_id,
                dump_upload.message_id,
                true,
                rip_result.codec.parse::<Codec>().ok(),
            )
            .await;
            return Err("cancelled".to_owned());
        }

        let total_duration_ms = (now_ms() - upload_item.start_time_ms) as i64;
        deps.log_request(RequestLog {
            telegram_id: options.user_id,
            chat_id: options.chat_id,
            track_key: TrackKey::new(Provider::Apple, track_id.clone()),
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
            let new_count = ctx
                .ripped_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            shared.lock().expect("job poisoned").job.ripped_count = new_count;

            // Publish the completed upload immediately. Without this event a
            // single-track job could leave the dashboard showing
            // `Uploading` until the terminal refresh arrived.
            let (download_text, upload_text) = texts.snapshot();
            bus.emit_progress(
                shared,
                None,
                download_text.as_deref(),
                upload_text.as_deref(),
            );

            tracing::info!(
                track = format!("{} - {}", rip_result.title, rip_result.artist),
                time = format!("{:.1}s", total_duration_ms as f64 / 1000.0),
                event = if options.is_cache_only {
                    "Track cached to dump"
                } else {
                    "Track completed"
                },
                "Track completed"
            );
            true
        }
        Err(err_msg) => {
            *texts.upload.lock().expect("texts poisoned") = None;
            shared.lock().expect("job poisoned").job.active_action_text = None;
            if is_cancelled() {
                return false;
            }
            record_failure(
                shared,
                ctx,
                deps,
                TrackFailureDetails {
                    track_id: &track_id,
                    err_msg,
                    start_time_ms: upload_item.start_time_ms,
                    title: Some(upload_item.rip_result.title.clone()),
                    artist: Some(upload_item.rip_result.artist.clone()),
                },
            )
            .await;
            let (download_text, upload_text) = texts.snapshot();
            bus.emit_progress(
                shared,
                None,
                download_text.as_deref(),
                upload_text.as_deref(),
            );
            false
        }
    }
}

struct TrackFailureDetails<'a> {
    track_id: &'a str,
    err_msg: String,
    start_time_ms: u64,
    title: Option<String>,
    artist: Option<String>,
}

/// Record a track failure: push the row, update the counter, and log the
/// request.
async fn record_failure<D: OrchestratorDeps>(
    shared: &Arc<Mutex<JobShared>>,
    ctx: &JobContext,
    deps: &Arc<D>,
    details: TrackFailureDetails<'_>,
) {
    {
        let mut failures = ctx.failed_tracks.lock().expect("failures poisoned");
        failures.push(FailedTrack {
            id: details.track_id.to_string(),
            error: details.err_msg.clone(),
            title: details.title,
            artist: details.artist,
            storefront: None,
        });
        shared.lock().expect("job poisoned").job.failed_count = failures.len();
    }
    tracing::error!(track_id = %details.track_id, error = %details.err_msg, "Track upload failed");
    let _ = deps
        .log_request(RequestLog {
            telegram_id: ctx.options.user_id,
            chat_id: ctx.options.chat_id,
            track_key: TrackKey::new(Provider::Apple, details.track_id),
            is_cache_hit: false,
            duration_ms: Some((now_ms() - details.start_time_ms) as i64),
            status: "failed".to_string(),
            error_reason: Some(details.err_msg),
        })
        .await;
}

// helpers
fn kind_str(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Track => "track",
        TargetKind::Album => "album",
        TargetKind::Artist => "artist",
        TargetKind::Playlist => "playlist",
    }
}

async fn delete_file_if_exists(path: &str) {
    let _ = tokio::fs::remove_file(std::path::Path::new(path)).await;
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn options(user_id: i64, is_admin: bool) -> RipJobOptions {
        RipJobOptions {
            chat_id: user_id,
            user_id,
            user_name: None,
            delivery_chat_id: user_id,
            is_group: false,
            is_force: false,
            is_cache_only: false,
            zip: false,
            zip_explicit: false,
            single_storefront: None,
            parsed_items: Vec::new(),
            reply_to_message_id: None,
            status_msg_id: 0,
            is_admin,
            codec_preference: CodecPreference::HighestQuality,
        }
    }

    #[test]
    fn job_ids_are_cuid2_and_fit_callbacks() {
        let id = cuid2::create_id();
        let next_id = cuid2::create_id();
        assert!(cuid2::is_cuid2(&id));
        assert_ne!(id, next_id);
        assert_eq!(id.len(), 24);
        let callback_data = format!("cancel:{id}");
        assert!(callback_data.len() <= 64);
        assert_eq!(
            callback_data.strip_prefix("cancel:").map(str::trim),
            Some(id.as_str())
        );
    }

    #[test]
    fn stream_status_extracts_human_label() {
        assert_eq!(
            stream_display_label("Connecting stream for Never Gonna Give You Up - Rick Astley..."),
            Some("Never Gonna Give You Up - Rick Astley")
        );
        assert_eq!(stream_display_label("Fetching track metadata..."), None);
    }

    #[test]
    fn admission_limits_users_and_global_jobs() {
        let orchestrator = RipOrchestrator::new();
        // Per-user cap: 4 concurrent jobs for normal users.
        let user = options(1, false);
        for job in ["u1", "u2", "u3", "u4"] {
            orchestrator.admit(job, &user).expect("user job");
        }
        assert!(matches!(
            orchestrator.admit("u5", &user),
            Err(OrchestratorError::UserAdmissionLimit)
        ));

        // Admins bypass both caps (per-user and global) but still occupy a
        // slot for `/cancel` bookkeeping.
        let admin = options(2, true);
        for job in ["a1", "a2", "a3", "a4", "a5"] {
            orchestrator.admit(job, &admin).expect("admin job");
        }

        // Global cap: 16 non-admin jobs. The admin jobs above do not count.
        for user_id in 3..=14 {
            orchestrator
                .admit(&format!("j{user_id}"), &options(user_id, false))
                .expect("global capacity");
        }
        assert!(matches!(
            orchestrator.admit("overflow", &options(100, false)),
            Err(OrchestratorError::AdmissionLimit)
        ));
        // The global cap frees a slot when a non-admin job is released.
        orchestrator.release_admission("j3");
        orchestrator
            .admit("after-release", &options(100, false))
            .expect("released slot");
    }
}
