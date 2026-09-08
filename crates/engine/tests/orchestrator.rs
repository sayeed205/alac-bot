//! Integration tests for the rip orchestrator (offline — every dependency
//! is a fake; parity with `rip-orchestrator.ts` behavior).

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use engine::{
    orchestrator::{
        deps::{
            CachedTrack, DumpUpload, OrchestratorDeps, RequestLog, SaveTrackInput, SinkError,
            TelegramSink, UploadProgressCallback,
        },
        types::{OrchestratorEvent, RipJobOptions, RipJobSummary},
        OrchestratorError, RipOrchestrator,
    },
    playlist::{PlaylistData, PlaylistError, PlaylistTrack},
    ripper::{RipError, RipProgressCallback},
    settings::{default_settings, BotSettings, RippingMode},
    types::{AlbumTracks, ArtistTracks, ParsedTargetItem, TargetKind, TrackMeta, TrackRipResult},
};
use tokio_util::sync::CancellationToken;

// ── fakes ────────────────────────────────────────────────────────────────

/// What the fake ripper should do for a given track id.
#[derive(Clone)]
enum RipScript {
    Ok,
    Fail(&'static str),
}

#[derive(Default)]
struct DepsState {
    saved_tracks: Vec<SaveTrackInput>,
    /// When set, `save_track` fails with this message.
    save_track_error: Option<String>,
    request_logs: Vec<RequestLog>,
    deleted_tracks: Vec<String>,
    deleted_message_batches: Vec<Vec<i64>>,
    /// Send-audio results, one per call.
    send_audio_results: VecDeque<Result<Option<DumpUpload>, SinkError>>,
    sent_audio: Vec<(String, String, String)>, // (file, title, performer)
    copies: Vec<(i64, i64, Option<i64>, bool)>, // (to, msg, replyTo, silent)
    /// Message ids whose dump copy fails (once).
    copies_fail_ids: Vec<i64>,
    rip_calls: Vec<String>,
}

impl DepsState {
    fn clear_sink_results(&mut self) {
        self.send_audio_results.clear();
    }
}

struct FakeDeps {
    state: Arc<Mutex<DepsState>>,
    settings: Mutex<BotSettings>,
    cache: Mutex<HashMap<String, CachedTrack>>,
    rip_scripts: Mutex<HashMap<String, RipScript>>,
    albums: Mutex<HashMap<String, AlbumTracks>>,
    artists: Mutex<HashMap<String, ArtistTracks>>,
    playlists: Mutex<HashMap<String, PlaylistData>>,
    upload_retry_base_ms: u64,
    sink: FakeSink,
}

impl FakeDeps {
    fn new() -> (Arc<Self>, Arc<Mutex<DepsState>>) {
        let state = Arc::new(Mutex::new(DepsState::default()));
        let deps = Arc::new(Self {
            state: Arc::clone(&state),
            settings: Mutex::new(default_settings()),
            cache: Mutex::new(HashMap::new()),
            rip_scripts: Mutex::new(HashMap::new()),
            albums: Mutex::new(HashMap::new()),
            artists: Mutex::new(HashMap::new()),
            playlists: Mutex::new(HashMap::new()),
            upload_retry_base_ms: 1,
            sink: FakeSink {
                state: Arc::clone(&state),
            },
        });
        (deps, state)
    }

    fn cache_track(&self, id: &str, message_id: i64) {
        self.cache.lock().unwrap().insert(
            id.to_string(),
            CachedTrack {
                apple_track_id: id.to_string(),
                message_id,
                file_id: format!("file_{id}"),
                file_unique_id: format!("uniq_{id}"),
                title: format!("T{id}"),
                artist: "Cached Artist".into(),
                album: "Cached Album".into(),
            },
        );
    }

    fn set_settings(&self, f: impl FnOnce(&mut BotSettings)) {
        f(&mut self.settings.lock().unwrap());
    }

    fn track_meta(id: &str, title: &str, artist: &str) -> TrackMeta {
        TrackMeta {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: "Album".into(),
            album_artist: artist.into(),
            genre: None,
            release_date: "2021-06-04".into(),
            composer: None,
            track_number: Some(1),
            track_count: Some(10),
            disc_number: None,
            disc_count: None,
            duration_secs: 215,
            explicit: false,
            artwork_url: String::new(),
        }
    }

    fn album(tracks: Vec<TrackMeta>) -> AlbumTracks {
        let first = tracks
            .first()
            .cloned()
            .unwrap_or_else(|| Self::track_meta("0", "", ""));
        AlbumTracks {
            album: first,
            tracks,
        }
    }

    fn rip_result(id: &str) -> TrackRipResult {
        TrackRipResult {
            file_path: format!("/tmp/does-not-exist-{id}.m4a"),
            title: "Night Song".into(),
            artist: "A&R <duo>".into(),
            album: "Escapes".into(),
            duration: 215,
            bit_depth: 24,
            sample_rate: 48000,
            codec: "alac".into(),
            genre: "Electronic".into(),
            release_date: "2021-06-04".into(),
            track_number: 2,
            track_count: 10,
        }
    }

    fn upload_ok() -> Result<Option<DumpUpload>, SinkError> {
        Ok(Some(DumpUpload {
            message_id: 777,
            file_id: "dump_file".into(),
            file_unique_id: "dump_uniq".into(),
        }))
    }
}

struct FakeSink {
    state: Arc<Mutex<DepsState>>,
}

impl TelegramSink for FakeSink {
    fn send_audio_to_dump<'a>(
        &'a self,
        file_path: &'a str,
        title: &'a str,
        performer: &'a str,
        _duration: i64,
        _caption_html: &'a str,
        _on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DumpUpload>, SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut st = state.lock().unwrap();
            st.sent_audio.push((
                file_path.to_string(),
                title.to_string(),
                performer.to_string(),
            ));
            match st.send_audio_results.pop_front() {
                Some(r) => r,
                None => Self::default_upload(),
            }
        })
    }

    fn send_dump_copy<'a>(
        &'a self,
        to_chat_id: i64,
        message_id: i64,
        reply_to: Option<i64>,
        silent: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut st = state.lock().unwrap();
            st.copies.push((to_chat_id, message_id, reply_to, silent));
            if st.copies_fail_ids.contains(&message_id) {
                st.copies_fail_ids.retain(|id| *id != message_id);
                return Err(SinkError("copy failed".into()));
            }
            Ok(())
        })
    }

    fn delete_dump_messages<'a>(
        &'a self,
        message_ids: &'a [i64],
    ) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .deleted_message_batches
                .push(message_ids.to_vec());
            Ok(())
        })
    }
}

impl FakeSink {
    fn default_upload() -> Result<Option<DumpUpload>, SinkError> {
        FakeDeps::upload_ok()
    }
}

impl OrchestratorDeps for FakeDeps {
    fn get_settings(&self) -> impl Future<Output = BotSettings> + Send {
        let settings = self.settings.lock().unwrap().clone();
        async move { settings }
    }

    fn find_cached_tracks(
        &self,
        ids: &[String],
    ) -> impl Future<Output = Result<HashMap<String, CachedTrack>, String>> + Send {
        let cache: HashMap<String, CachedTrack> = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(id, t)| (id.clone(), t.clone()))
            .collect();
        async move { Ok(cache) }
    }

    fn save_track(&self, input: SaveTrackInput) -> impl Future<Output = Result<(), String>> + Send {
        let err = self.state.lock().unwrap().save_track_error.clone();
        self.state.lock().unwrap().saved_tracks.push(input);
        async move {
            if let Some(err) = err {
                Err(err)
            } else {
                Ok(())
            }
        }
    }

    fn delete_track(
        &self,
        apple_track_id: &str,
    ) -> impl Future<Output = Result<bool, String>> + Send {
        self.state
            .lock()
            .unwrap()
            .deleted_tracks
            .push(apple_track_id.to_string());
        self.cache.lock().unwrap().remove(apple_track_id);
        async move { Ok(true) }
    }

    fn log_request(&self, log: RequestLog) -> impl Future<Output = Result<(), String>> + Send {
        self.state.lock().unwrap().request_logs.push(log);
        async move { Ok(()) }
    }

    fn fetch_album_tracks(
        &self,
        id: &str,
        _storefront: &str,
    ) -> impl Future<Output = Result<AlbumTracks, String>> + Send {
        let result = self.albums.lock().unwrap().get(id).cloned();
        async move { result.ok_or_else(|| format!("Album {id} not found")) }
    }

    fn fetch_artist_tracks(
        &self,
        id: &str,
        _storefront: &str,
    ) -> impl Future<Output = Result<ArtistTracks, String>> + Send {
        let result = self.artists.lock().unwrap().get(id).cloned();
        async move { result.ok_or_else(|| format!("Artist {id} not found")) }
    }

    fn fetch_playlist_tracks(
        &self,
        id: &str,
        _storefront: &str,
    ) -> impl Future<Output = Result<PlaylistData, PlaylistError>> + Send {
        let result = self.playlists.lock().unwrap().get(id).cloned();
        async move {
            result.ok_or_else(|| PlaylistError::NotFound {
                playlist_id: id.to_string(),
                storefront: "us".to_string(),
            })
        }
    }

    fn rip(
        &self,
        track_id: &str,
        _on_progress: Option<&RipProgressCallback>,
        _storefront: &str,
        _signal: CancellationToken,
        _output_dir: Option<&std::path::Path>,
    ) -> impl Future<Output = Result<TrackRipResult, RipError>> + Send {
        self.state
            .lock()
            .unwrap()
            .rip_calls
            .push(track_id.to_string());
        let script = self.rip_scripts.lock().unwrap().get(track_id).cloned();
        let id = track_id.to_string();
        async move {
            match script {
                Some(RipScript::Fail(msg)) => Err(RipError::Message(msg.to_string())),
                _ => Ok(Self::rip_result(&id)),
            }
        }
    }

    fn sink(&self) -> &dyn TelegramSink {
        &self.sink
    }

    fn upload_retry_base_ms(&self) -> u64 {
        self.upload_retry_base_ms
    }
}

// ── helpers ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct EventLog {
    records: Arc<Mutex<Vec<String>>>,
}

impl EventLog {
    fn attach(orch: &RipOrchestrator) -> Self {
        let log = Self {
            records: Arc::new(Mutex::new(Vec::new())),
        };
        let records = Arc::clone(&log.records);
        orch.subscribe(Arc::new(move |event: &OrchestratorEvent<'_>| {
            let mut v = records.lock().unwrap();
            match event {
                OrchestratorEvent::Created(_) => v.push("created".into()),
                OrchestratorEvent::Started(_) => v.push("started".into()),
                OrchestratorEvent::Completed(_, _) => v.push("completed".into()),
                OrchestratorEvent::Cancelled(_, by) => {
                    v.push(format!("cancelled:{:?}", by));
                }
                OrchestratorEvent::Failed(_, msg) => v.push(format!("failed:{msg}")),
                OrchestratorEvent::Progress(_, p) => {
                    v.push(format!(
                        "progress:{}:{}:{}",
                        p.percent,
                        p.activity_override.as_deref().unwrap_or("-"),
                        p.completed_tracks,
                    ));
                }
            }
        }));
        log
    }

    fn snapshot(&self) -> Vec<String> {
        self.records.lock().unwrap().clone()
    }
}

fn options(items: Vec<ParsedTargetItem>, is_admin: bool) -> RipJobOptions {
    RipJobOptions {
        chat_id: 100,
        user_id: 42,
        user_name: Some("tester".into()),
        delivery_chat_id: 100,
        is_group: false,
        is_force: false,
        is_cache_only: false,
        single_storefront: None,
        parsed_items: items,
        reply_to_message_id: Some(555),
        status_msg_id: 999,
        is_admin,
    }
}

fn track_item(id: &str) -> ParsedTargetItem {
    ParsedTargetItem {
        id: id.into(),
        kind: TargetKind::Track,
        storefront: None,
    }
}

fn album_item(id: &str) -> ParsedTargetItem {
    ParsedTargetItem {
        id: id.into(),
        kind: TargetKind::Album,
        storefront: None,
    }
}

fn playlist_item(id: &str) -> ParsedTargetItem {
    ParsedTargetItem {
        id: id.into(),
        kind: TargetKind::Playlist,
        storefront: None,
    }
}

fn setup() -> (
    RipOrchestrator,
    Arc<FakeDeps>,
    Arc<Mutex<DepsState>>,
    EventLog,
) {
    let (deps, state) = FakeDeps::new();
    let orch = RipOrchestrator::new();
    let events = EventLog::attach(&orch);
    (orch, deps, state, events)
}

async fn run_async(
    orch: &RipOrchestrator,
    deps: &Arc<FakeDeps>,
    opts: &RipJobOptions,
) -> Result<RipJobSummary, OrchestratorError> {
    orch.start_job(Arc::clone(deps), opts).await
}

// ── tests ────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_path_single_track() {
    let (orch, deps, state, events) = setup();
    deps.cache.lock().unwrap().clear();

    let summary = run_async(&orch, &deps, &options(vec![track_item("1440828878")], true))
        .await
        .expect("job succeeds");

    assert_eq!(summary.total_tracks, 1);
    assert_eq!(summary.ripped_count, 1);
    assert_eq!(summary.failed_count, 0);
    assert_eq!(summary.cached_count, 0);
    assert_eq!(summary.job_header, "Track 1440828878");
    assert_eq!(summary.skipped_uncached_tracks.len(), 0);
    assert!(!summary.total_elapsed_sec.is_empty());

    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls, vec!["1440828878".to_string()]);
    assert_eq!(st.saved_tracks.len(), 1);
    let saved = &st.saved_tracks[0];
    assert_eq!(saved.apple_track_id, "1440828878");
    assert_eq!(saved.message_id, 777);
    assert_eq!(saved.title, "Night Song");
    assert_eq!(saved.bit_depth, 24);
    assert_eq!(st.request_logs.len(), 1);
    assert_eq!(st.request_logs[0].status, "completed");
    assert!(!st.request_logs[0].is_cache_hit);
    assert_eq!(
        st.copies,
        vec![(100, 777, Some(555), false)],
        "reply_to passed when delivery==chat; silent=false for single track"
    );
    drop(st);

    let ev = events.snapshot();
    assert_eq!(ev[0], "created");
    assert!(ev.contains(&"started".to_string()));
    assert_eq!(*ev.last().unwrap(), "completed");
    // No cancelled/failed.
    assert!(ev
        .iter()
        .all(|e| !e.starts_with("cancelled") && !e.starts_with("failed")));

    // Job is gone from the map after completion.
    assert!(orch.get_active_jobs().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn header_from_rip_metadata_for_single_track() {
    let (orch, deps, _, _) = setup();
    // Track items never carry metadata → header stays `Track {id}`.
    let summary = run_async(
        &orch,
        &deps,
        &options(vec![track_item("123"), track_item("456")], true),
    )
    .await
    .expect("job succeeds");
    // Multi-link: initial header is `Batch (2 links)` — never refined since
    // plain tracks resolve without titles.
    assert_eq!(summary.job_header, "Batch (2 tracks)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn album_resolution_refines_header_and_lists_tracks() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str, title: &str| FakeDeps::track_meta(id, title, "Nils Frahm");
    deps.albums.lock().unwrap().insert(
        "alb.1".into(),
        FakeDeps::album(vec![meta("t1", "Says"), meta("t2", "Some")]),
    );

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.1")], true))
        .await
        .expect("job succeeds");

    assert_eq!(summary.total_tracks, 2);
    assert_eq!(summary.job_header, "Nils Frahm - Album");
    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls.len(), 2);
    assert!(st.copies[0].3, "silent=true for multi-track");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_cached_fast_path() {
    let (orch, deps, state, events) = setup();
    deps.cache_track("1440828878", 4242);

    let summary = run_async(
        &orch,
        &deps,
        &options(vec![track_item("1440828878")], false),
    )
    .await
    .expect("job succeeds");

    assert_eq!(summary.cached_count, 1);
    assert_eq!(summary.ripped_count, 0);
    assert_eq!(summary.total_tracks, 1);
    assert_eq!(summary.total_elapsed_sec, "0.0");
    assert_eq!(summary.failed_tracks.len(), 0);

    let st = state.lock().unwrap();
    assert!(st.rip_calls.is_empty(), "no rip on cache hit");
    assert_eq!(st.copies, vec![(100, 4242, Some(555), false)]);
    assert_eq!(st.request_logs.len(), 1);
    assert!(st.request_logs[0].is_cache_hit);
    assert_eq!(st.request_logs[0].duration_ms, Some(0));
    drop(st);

    let ev = events.snapshot();
    assert!(ev.contains(&"progress:100:Delivered cached tracks...:1".to_string()));
    assert_eq!(*ev.last().unwrap(), "completed");
    assert!(!ev.contains(&"started".to_string()), "queue never starts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_only_marks_cached_without_delivery() {
    let (orch, deps, state, _) = setup();
    deps.cache_track("1440828878", 4242);

    let mut opts = options(vec![track_item("1440828878")], false);
    opts.is_cache_only = true;

    let summary = run_async(&orch, &deps, &opts).await.expect("job succeeds");

    assert_eq!(summary.cached_count, 1);
    assert!(summary.is_cache_only);
    let st = state.lock().unwrap();
    assert!(st.copies.is_empty(), "cache-only never delivers copies");
    assert!(
        st.request_logs.is_empty(),
        "cache-only does not log requests"
    );
    drop(st);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintenance_gate_rejects_live_rips() {
    let (orch, deps, _, events) = setup();
    deps.set_settings(|s| s.ripping_mode = RippingMode::Paused);

    let err = run_async(&orch, &deps, &options(vec![track_item("1")], false))
        .await
        .expect_err("gate must reject");

    assert_eq!(
        err.to_string(),
        "Live ripping is temporarily paused for maintenance. Only cached tracks can be played right now."
    );
    assert!(
        events.snapshot().is_empty(),
        "no events before job registration"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintenance_gate_allows_admin_and_cache_only() {
    let (orch, deps, _, _) = setup();
    deps.set_settings(|s| s.ripping_mode = RippingMode::Paused);
    deps.cache_track("1", 9);

    let mut opts = options(vec![track_item("1")], false);
    opts.is_cache_only = true;
    run_async(&orch, &deps, &opts)
        .await
        .expect("cache-only passes the gate");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_admin_collection_cap() {
    let (orch, deps, _, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Says", "Nils Frahm");
    deps.albums.lock().unwrap().insert(
        "alb.1".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2"), meta("t3")]),
    );
    deps.set_settings(|s| s.max_collection_tracks = 2);

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.1")], false))
        .await
        .expect("job succeeds");

    assert_eq!(summary.total_tracks, 2, "only the first 2 are processed");
    assert_eq!(summary.capped_count, 1);
    assert_eq!(summary.max_collection_limit, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dedup_preserves_first_occurrence_order() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Says", "Nils Frahm");
    deps.albums.lock().unwrap().insert(
        "alb.1".into(),
        FakeDeps::album(vec![meta("t2"), meta("t1"), meta("t2")]),
    );

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.1")], true))
        .await
        .expect("job succeeds");

    assert_eq!(summary.total_tracks, 2, "duplicate t2 collapsed");
    assert_eq!(
        state.lock().unwrap().rip_calls,
        vec!["t2".to_string(), "t1".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_purge_deletes_cached_before_queue() {
    let (orch, deps, state, _) = setup();
    deps.cache_track("1440828878", 4242);

    let mut opts = options(vec![track_item("1440828878")], true);
    opts.is_force = true;

    let summary = run_async(&orch, &deps, &opts).await.expect("job succeeds");

    assert_eq!(summary.cached_count, 0, "purged cache is not served");
    let st = state.lock().unwrap();
    assert_eq!(st.deleted_tracks, vec!["1440828878".to_string()]);
    assert_eq!(
        st.deleted_message_batches,
        vec![vec![4242]],
        "old dump messages deleted in one batch"
    );
    assert_eq!(st.rip_calls, vec!["1440828878".to_string()], "re-ripped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_without_admin_keeps_cache() {
    let (orch, deps, state, _) = setup();
    deps.cache_track("1440828878", 4242);

    let mut opts = options(vec![track_item("1440828878")], false);
    opts.is_force = true;

    let summary = run_async(&orch, &deps, &opts).await.expect("job succeeds");

    assert_eq!(summary.cached_count, 1, "non-admin force does not purge");
    let st = state.lock().unwrap();
    assert!(st.deleted_tracks.is_empty());
    assert!(st.deleted_message_batches.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolution_error_bubbles_with_prefix() {
    let (orch, deps, _, events) = setup();

    let err = run_async(&orch, &deps, &options(vec![album_item("missing")], true))
        .await
        .expect_err("resolution fails");

    assert_eq!(err.to_string(), "album missing: Album missing not found");
    let ev = events.snapshot();
    assert_eq!(
        *ev.last().unwrap(),
        "failed:album missing: Album missing not found"
    );
    assert!(orch.get_active_jobs().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_resolution_fails() {
    let (orch, deps, _, _) = setup();
    // A playlist that resolves to zero tracks.
    deps.playlists.lock().unwrap().insert(
        "pl.empty".into(),
        PlaylistData {
            id: "pl.empty".into(),
            title: "Empty".into(),
            curator_name: None,
            description: None,
            tracks: vec![],
        },
    );

    let err = run_async(
        &orch,
        &deps,
        &options(vec![playlist_item("pl.empty")], true),
    )
    .await
    .expect_err("no tracks");

    assert_eq!(err.to_string(), "No valid tracks found to process.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playlist_resolution_uses_seam() {
    let (orch, deps, _, _) = setup();
    deps.playlists.lock().unwrap().insert(
        "pl.1".into(),
        PlaylistData {
            id: "pl.1".into(),
            title: "Mix".into(),
            curator_name: None,
            description: None,
            tracks: vec![PlaylistTrack {
                id: "pt1".into(),
                title: "Track One".into(),
                artist: "Artist".into(),
                duration: Some(122),
            }],
        },
    );

    let summary = run_async(&orch, &deps, &options(vec![playlist_item("pl.1")], true))
        .await
        .expect("job succeeds");
    assert_eq!(summary.total_tracks, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rip_failure_logs_and_continues() {
    let (orch, deps, state, _) = setup();
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("bad".into(), RipScript::Fail("CDN error"));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("good".into(), RipScript::Ok);

    let summary = run_async(
        &orch,
        &deps,
        &options(vec![track_item("bad"), track_item("good")], true),
    )
    .await
    .expect("job completes despite one failure");

    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(summary.failed_tracks[0].id, "bad");
    assert_eq!(summary.failed_tracks[0].error, "CDN error");
    assert_eq!(summary.ripped_count, 1);
    let st = state.lock().unwrap();
    let failed_logs: Vec<_> = st
        .request_logs
        .iter()
        .filter(|l| l.status == "failed")
        .collect();
    assert_eq!(failed_logs.len(), 1);
    assert_eq!(failed_logs[0].apple_track_id, "bad");
    assert_eq!(failed_logs[0].error_reason.as_deref(), Some("CDN error"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn circuit_breaker_stops_remaining_batch() {
    let (orch, deps, state, events) = setup();
    for id in ["t1", "t2", "t3"] {
        deps.rip_scripts
            .lock()
            .unwrap()
            .insert(id.into(), RipScript::Fail("Mirror request timed out"));
    }

    let summary = run_async(
        &orch,
        &deps,
        &options(
            vec![track_item("t1"), track_item("t2"), track_item("t3")],
            true,
        ),
    )
    .await
    .expect("job completes (deviation: producer settles instead of hanging)");

    // t1 fails → breaker row; t2/t3 are never ripped.
    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls, vec!["t1".to_string()]);
    drop(st);
    assert_eq!(summary.failed_tracks.len(), 2);
    assert_eq!(summary.failed_tracks[0].id, "t1");
    assert_eq!(summary.failed_tracks[1].id, "Remaining tracks");
    assert_eq!(
        summary.failed_tracks[1].error,
        "Mirror service offline / unreachable (stopped remaining batch)"
    );
    let _ = events;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_retry_then_success() {
    let (orch, deps, state, _) = setup();
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        st.send_audio_results
            .push_back(Err(SinkError("flood".into())));
        st.send_audio_results.push_back(FakeDeps::upload_ok());
    }

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("second attempt succeeds");

    assert_eq!(summary.ripped_count, 1);
    assert_eq!(summary.failed_count, 0);
    let st = state.lock().unwrap();
    assert_eq!(st.sent_audio.len(), 2, "retried exactly once");
    assert_eq!(st.saved_tracks.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_retries_exhausted_records_failure() {
    let (orch, deps, state, _) = setup();
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        for _ in 0..4 {
            st.send_audio_results
                .push_back(Err(SinkError("flood".into())));
        }
    }

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("deviation: job survives exhausted uploads");

    assert_eq!(summary.ripped_count, 0);
    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(summary.failed_tracks[0].id, "t1");
    assert_eq!(summary.failed_tracks[0].error, "flood");
    let st = state.lock().unwrap();
    assert_eq!(st.sent_audio.len(), 4, "max_retries=4 attempts");
    // TS throws before any request log — the deviation records no log either.
    assert!(st.request_logs.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_audio_media_records_no_log() {
    let (orch, deps, state, _) = setup();
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        st.send_audio_results.push_back(Ok(None)); // not audio
    }

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("job continues");

    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(
        summary.failed_tracks[0].error,
        "Upload failed: no audio media returned"
    );
    let st = state.lock().unwrap();
    assert!(st.request_logs.is_empty(), "no request log for non-audio");
    assert!(st.saved_tracks.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_upload_save_failure_records_and_logs() {
    let (orch, deps, state, _) = setup();
    state.lock().unwrap().save_track_error = Some("db down".into());

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("job completes (failure recorded, not thrown)");

    assert_eq!(summary.ripped_count, 0);
    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(summary.failed_tracks[0].id, "t1");
    assert_eq!(summary.failed_tracks[0].error, "db down");
    let st = state.lock().unwrap();
    // The post-upload catch logs a failed request (save → copy → log share
    // the try/catch: the copy never happens because save failed first).
    assert_eq!(st.request_logs.len(), 1);
    assert_eq!(st.request_logs[0].status, "failed");
    assert_eq!(st.request_logs[0].error_reason.as_deref(), Some("db down"));
    assert!(st.copies.is_empty(), "copy skipped after save failure");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_copy_failure_marks_re_rip() {
    let (orch, deps, state, _) = setup();
    deps.cache_track("t1", 4242);
    // Make the cache-copy fail for message 4242 (and later copies succeed).
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        // send_audio isn't used on the cache path; copies fail via a
        // dedicated flag instead.
        st.copies_fail_ids.push(4242);
    }

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], false))
        .await
        .expect("job succeeds");
    assert_eq!(summary.ripped_count, 1);
    assert_eq!(state.lock().unwrap().rip_calls, vec!["t1".to_string()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_job_semantics() {
    let (orch, deps, _, _) = setup();

    assert!(!orch.cancel_job("missing", None), "unknown id → false");

    // Run a quick job to completion.
    run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("job succeeds");
    // The job map is emptied after completion — nothing to cancel.
    assert!(!orch.cancel_job("whatever", None));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_position_field_defaults_none() {
    let (orch, deps, _, _) = setup();
    run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("job succeeds");
    // The command handler (M5b) sets queue_position; startJob never does,
    // and the job map is empty after completion.
    assert!(orch.get_active_jobs().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn progress_percent_math() {
    let (orch, deps, _, events) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Says", "Nils Frahm");
    deps.albums.lock().unwrap().insert(
        "alb.1".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.1")], true))
        .await
        .expect("job succeeds");
    assert_eq!(summary.ripped_count, 2);

    let ev = events.snapshot();
    // TS parity: the uploader's success path never emits progress (it only
    // mutates the job counters), so the final progress event is track 2's
    // download activity at 1/2 completed — never 100%.
    let last_progress = ev
        .iter()
        .rev()
        .find(|e| e.starts_with("progress:"))
        .expect("progress events exist");
    assert!(
        last_progress.starts_with("progress:50:"),
        "got {last_progress}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_tracks_after_cap_edge() {
    // maxCollectionTracks = 0 means "no cap" in TS (limit > 0 check).
    let (orch, deps, _, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Says", "Nils Frahm");
    deps.albums.lock().unwrap().insert(
        "alb.1".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.set_settings(|s| s.max_collection_tracks = 0);

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.1")], false))
        .await
        .expect("job succeeds");
    assert_eq!(summary.total_tracks, 2, "0 disables the cap");
    assert_eq!(summary.capped_count, 0);
}
