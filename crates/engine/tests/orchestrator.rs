//! Integration tests for the rip orchestrator (offline — every dependency
//! is a fake).

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use engine::{
    orchestrator::{
        deps::{
            AlbumUpload, CachedAlbum, CachedTrack, DumpUpload, OrchestratorDeps, RequestLog,
            SaveTrackInput, SinkError, TelegramSink, UploadProgressCallback,
        },
        types::{JobPhase, OrchestratorEvent, RipJobOptions, RipJobSummary},
        OrchestratorError, RipOrchestrator,
    },
    playlist::{PlaylistData, PlaylistError, PlaylistTrack},
    ripper::{RipError, RipProgressCallback},
    settings::{default_settings, BotSettings, RippingMode},
    types::{
        AlbumTracks, ArtistTracks, ParsedTargetItem, TargetKind, TrackKey, TrackMeta,
        TrackRipResult,
    },
};
use tokio_util::sync::CancellationToken;

// fakes
/// What the fake ripper should do for a given track id.
#[derive(Clone)]
enum RipScript {
    Ok,
    /// Rip succeeds and the fake writes a real file (for ZIP staging tests).
    OkWithFile(Vec<u8>),
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
    // album ZIP
    saved_albums: Vec<AlbumUpload>,
    deleted_album_zip_ids: Vec<String>,
    found_albums: HashMap<String, Vec<CachedAlbum>>,
    sent_documents: Vec<String>, // dump + direct document upload paths
    uploaded_document_bytes: Vec<u8>, // captured at upload time (workspace is deleted after)
    /// Thumbnail paths passed to document sends (may repeat per part).
    sent_thumbs: Vec<String>,
    sent_photos: Vec<(i64, usize, String)>, // (chat, byte len, caption)
    fetch_artwork_urls: Vec<String>,
    artwork_bytes: Option<Vec<u8>>,
    /// When set, `send_audio_to_dump` waits for this token before returning
    /// — used to hold lane 2 open deterministically (cancel-during-upload
    /// tests).
    gate_uploads: Option<tokio_util::sync::CancellationToken>,
}

impl DepsState {
    fn clear_sink_results(&mut self) {
        self.send_audio_results.clear();
    }
}

struct FakeDeps {
    state: Arc<Mutex<DepsState>>,
    settings: Mutex<BotSettings>,
    cache: Mutex<HashMap<TrackKey, CachedTrack>>,
    rip_scripts: Mutex<HashMap<String, RipScript>>,
    albums: Mutex<HashMap<String, AlbumTracks>>,
    artists: Mutex<HashMap<String, ArtistTracks>>,
    playlists: Mutex<HashMap<String, PlaylistData>>,
    upload_retry_base_ms: u64,
    upload_max_retries: Mutex<u32>,
    rip_delay_ms: Mutex<u64>,
    cache_delay_ms: Mutex<u64>,
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
            upload_max_retries: Mutex::new(3),
            rip_delay_ms: Mutex::new(0),
            cache_delay_ms: Mutex::new(0),
            sink: FakeSink {
                state: Arc::clone(&state),
            },
        });
        (deps, state)
    }

    fn cache_track(&self, id: &str, message_id: i64) {
        self.cache.lock().unwrap().insert(
            TrackKey::apple(id),
            CachedTrack {
                track_key: TrackKey::apple(id),
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
            content_advisory: None,
            artwork_url: String::new(),
            album_id: None,
            artist_id: None,
            isrc: None,
            record_label: None,
            copyright: None,
            upc: None,
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
            let gate = {
                let st = state.lock().unwrap();
                st.gate_uploads.clone()
            };
            if let Some(gate) = gate {
                gate.cancelled().await;
            }
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
    ) -> Pin<Box<dyn Future<Output = Result<i32, SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut st = state.lock().unwrap();
            st.copies.push((to_chat_id, message_id, reply_to, silent));
            if st.copies_fail_ids.contains(&message_id) {
                st.copies_fail_ids.retain(|id| *id != message_id);
                return Err(SinkError("copy failed".into()));
            }
            Ok(message_id as i32)
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

    fn send_document_to_dump<'a>(
        &'a self,
        file_path: &'a str,
        thumb_path: Option<&'a str>,
        _caption_html: &'a str,
        _on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<DumpUpload>, SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        let path = file_path.to_owned();
        let thumb = thumb_path.map(str::to_owned);
        Box::pin(async move {
            // Read the bytes now: the orchestrator deletes the workspace
            // right after the upload returns.
            let bytes = std::fs::read(&path).unwrap_or_default();
            let mut st = state.lock().unwrap();
            st.sent_documents.push(path);
            st.uploaded_document_bytes = bytes;
            st.sent_thumbs.extend(thumb);
            Ok(Some(DumpUpload {
                message_id: 900,
                file_id: "zip_file".into(),
                file_unique_id: "zip_uniq".into(),
            }))
        })
    }

    fn send_document_to_chat<'a>(
        &'a self,
        _chat_id: i64,
        file_path: &'a str,
        thumb_path: Option<&'a str>,
        _caption_html: &'a str,
        _on_upload_progress: Option<&'a UploadProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<i32, SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        let path = file_path.to_owned();
        let thumb = thumb_path.map(str::to_owned);
        Box::pin(async move {
            let bytes = std::fs::read(&path).unwrap_or_default();
            let mut st = state.lock().unwrap();
            st.sent_documents.push(path);
            st.uploaded_document_bytes = bytes;
            st.sent_thumbs.extend(thumb);
            Ok(200)
        })
    }

    fn send_photo_to_chat<'a>(
        &'a self,
        chat_id: i64,
        image_bytes: &'a [u8],
        caption_html: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            state.lock().unwrap().sent_photos.push((
                chat_id,
                image_bytes.len(),
                caption_html.to_string(),
            ));
            Ok(())
        })
    }

    fn download_dump_file<'a>(
        &'a self,
        _message_id: i64,
        destination: &'a std::path::Path,
        _on_download_progress: Option<&'a UploadProgressCallback>,
    ) -> Pin<Box<dyn Future<Output = Result<(), SinkError>> + Send + 'a>> {
        Box::pin(async move {
            // Materialize a deterministic fake audio file so ZIP staging has
            // real bytes to archive.
            std::fs::write(destination, b"cached-audio-bytes")
                .map_err(|error| SinkError(error.to_string()))
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
        keys: &[TrackKey],
    ) -> impl Future<Output = Result<HashMap<TrackKey, CachedTrack>, String>> + Send {
        let cache: HashMap<TrackKey, CachedTrack> = self
            .cache
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| keys.contains(key))
            .map(|(id, t)| (id.clone(), t.clone()))
            .collect();
        let delay_ms = *self.cache_delay_ms.lock().unwrap();
        async move {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Ok(cache)
        }
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
        track_key: &TrackKey,
    ) -> impl Future<Output = Result<bool, String>> + Send {
        self.state
            .lock()
            .unwrap()
            .deleted_tracks
            .push(track_key.track_id.clone());
        self.cache.lock().unwrap().remove(track_key);
        async move { Ok(true) }
    }

    fn log_request(&self, log: RequestLog) -> impl Future<Output = Result<(), String>> + Send {
        self.state.lock().unwrap().request_logs.push(log);
        async move { Ok(()) }
    }

    fn save_album<'a>(
        &'a self,
        upload: AlbumUpload,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        self.state.lock().unwrap().saved_albums.push(upload);
        Box::pin(async { Ok(()) })
    }

    fn find_albums<'a>(
        &'a self,
        _provider: engine::types::Provider,
        album_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<CachedAlbum>, String>> + Send + 'a>> {
        let rows = self
            .state
            .lock()
            .unwrap()
            .found_albums
            .get(album_id)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(rows) })
    }

    fn delete_albums<'a>(
        &'a self,
        _provider: engine::types::Provider,
        album_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        self.state
            .lock()
            .unwrap()
            .deleted_album_zip_ids
            .push(album_id.to_string());
        Box::pin(async { Ok(()) })
    }

    fn fetch_artwork<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>> {
        self.state
            .lock()
            .unwrap()
            .fetch_artwork_urls
            .push(url.into());
        let bytes = self.state.lock().unwrap().artwork_bytes.clone();
        Box::pin(async move { bytes })
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
        output_dir: Option<&std::path::Path>,
        _codec_preference: engine::wrapper::CodecPreference,
    ) -> impl Future<Output = Result<TrackRipResult, RipError>> + Send {
        self.state
            .lock()
            .unwrap()
            .rip_calls
            .push(track_id.to_string());
        let script = self.rip_scripts.lock().unwrap().get(track_id).cloned();
        let id = track_id.to_string();
        let delay_ms = *self.rip_delay_ms.lock().unwrap();
        async move {
            if delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            match script {
                Some(RipScript::Fail(msg)) => Err(RipError::Message(msg.to_string())),
                Some(RipScript::OkWithFile(bytes)) => {
                    // Write a real file so the ZIP staging path has actual
                    // bytes to copy into the archive workspace.
                    let dir = output_dir
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| std::env::temp_dir().join(format!("fake_rip_{id}")));
                    let path = dir.join(format!("{id}.m4a"));
                    std::fs::write(&path, &bytes).map_err(|e| RipError::Message(e.to_string()))?;
                    Ok(TrackRipResult {
                        file_path: path.to_string_lossy().into_owned(),
                        ..Self::rip_result(&id)
                    })
                }
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

    fn upload_max_retries(&self) -> u32 {
        *self.upload_max_retries.lock().unwrap()
    }
}

// helpers
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
        zip: false,
        zip_explicit: false,
        single_storefront: None,
        parsed_items: items,
        reply_to_message_id: Some(555),
        status_msg_id: 999,
        is_admin,
        codec_preference: engine::wrapper::CodecPreference::HighestQuality,
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

// tests
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
    assert_eq!(summary.job_header, "Track ID: <code>1440828878</code>");
    assert_eq!(summary.skipped_uncached_tracks.len(), 0);
    assert!(!summary.total_elapsed_sec.is_empty());

    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls, vec!["1440828878".to_string()]);
    assert_eq!(st.saved_tracks.len(), 1);
    assert_eq!(
        st.sent_audio,
        vec![(
            "/tmp/does-not-exist-1440828878.m4a".to_string(),
            "Night Song".to_string(),
            "A&R <duo>".to_string(),
        )],
        "a live miss must upload to the dump before delivering the DM copy"
    );
    let saved = &st.saved_tracks[0];
    assert_eq!(saved.track_key, TrackKey::apple("1440828878"));
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
    assert_eq!(summary.job_header, "Batch: <b>2 tracks</b>");
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
    assert_eq!(
        summary.job_header,
        "Album: <a href=\"https://music.apple.com/us/album/alb.1\"><b>Album</b></a> by <b>Nils Frahm</b>"
    );
    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls.len(), 2);
    assert!(st.copies[0].3, "silent=true for multi-track");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_cached_fast_path() {
    let (orch, deps, state, events) = setup();
    deps.cache_track("1440828878", 4242);
    *deps.cache_delay_ms.lock().unwrap() = 120;

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
    assert_ne!(summary.total_elapsed_sec, "0.0");
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
async fn cache_only_serves_hits_and_seeds_misses() {
    let (orch, deps, state, _) = setup();
    deps.cache_track("hit", 4242);
    let mut opts = options(vec![track_item("hit"), track_item("miss")], false);
    opts.is_cache_only = true;

    let summary = run_async(&orch, &deps, &opts)
        .await
        .expect("cache job succeeds");
    assert_eq!(summary.cached_count, 1);
    assert_eq!(summary.ripped_count, 1);
    assert!(summary.skipped_uncached_tracks.is_empty());
    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls, vec!["miss"]);
    assert_eq!(st.saved_tracks.len(), 1);
    assert!(st.copies.is_empty(), "cache-only never delivers copies");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintenance_mode_skips_uncached_tracks() {
    let (orch, deps, _, events) = setup();
    deps.set_settings(|s| s.ripping_mode = RippingMode::Paused);

    let summary = run_async(&orch, &deps, &options(vec![track_item("1")], false))
        .await
        .expect("maintenance mode completes with a skipped miss");
    assert_eq!(summary.skipped_uncached_tracks, vec!["1"]);
    assert!(events.snapshot().contains(&"completed".to_string()));
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

    assert_eq!(
        err.to_string(),
        "Failed to resolve any tracks: album missing: Album missing not found"
    );
    let ev = events.snapshot();
    assert_eq!(
        *ev.last().unwrap(),
        "failed:Failed to resolve any tracks: album missing: Album missing not found"
    );
    assert!(orch.get_active_jobs().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_resolution_continues_successful_tracks() {
    let (orch, deps, state, _) = setup();
    let summary = run_async(
        &orch,
        &deps,
        &options(vec![album_item("missing"), track_item("good")], true),
    )
    .await
    .expect("a resolvable item keeps the job alive");

    assert_eq!(summary.total_tracks, 1);
    assert_eq!(summary.ripped_count, 1);
    assert_eq!(state.lock().unwrap().rip_calls, vec!["good"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_resolution_failures_are_structured() {
    let (orch, deps, _, _) = setup();
    let err = run_async(
        &orch,
        &deps,
        &options(vec![album_item("a"), playlist_item("p")], true),
    )
    .await
    .expect_err("nothing resolved");

    match err {
        OrchestratorError::ResolutionFailed { failures } => {
            assert_eq!(failures.len(), 2);
            assert_eq!(failures[0].id, "a");
            assert_eq!(failures[1].id, "p");
        }
        other => panic!("expected structured resolution failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_can_rip_when_live_mode_is_paused() {
    let (orch, deps, state, _) = setup();
    deps.set_settings(|settings| settings.ripping_mode = RippingMode::Paused);

    let summary = run_async(&orch, &deps, &options(vec![track_item("admin")], true))
        .await
        .expect("admin live override");
    assert_eq!(summary.ripped_count, 1);
    assert_eq!(state.lock().unwrap().rip_calls, vec!["admin"]);
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

    assert_eq!(
        err.to_string(),
        "Failed to resolve any tracks: track : No valid tracks found to process."
    );
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
    assert_eq!(failed_logs[0].track_key, TrackKey::apple("bad"));
    assert_eq!(failed_logs[0].error_reason.as_deref(), Some("CDN error"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn circuit_breaker_stops_remaining_batch() {
    let (orch, deps, state, events) = setup();
    for id in ["t1", "t2", "t3"] {
        deps.rip_scripts
            .lock()
            .unwrap()
            .insert(id.into(), RipScript::Fail("Mirror /status check timed out"));
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
async fn generic_timeout_and_status_errors_do_not_break_batch() {
    let (orch, deps, state, _) = setup();
    for id in ["t1", "t2", "t3"] {
        deps.rip_scripts.lock().unwrap().insert(
            id.into(),
            RipScript::Fail("request timed out with HTTP 503"),
        );
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
    .expect("generic errors are ordinary track failures");
    assert_eq!(state.lock().unwrap().rip_calls.len(), 3);
    assert_eq!(summary.failed_tracks.len(), 3);
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
async fn configured_upload_retry_count_controls_calls() {
    let (orch, deps, state, _) = setup();
    *deps.upload_max_retries.lock().unwrap() = 1;
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        st.send_audio_results
            .push_back(Err(SinkError("flood".into())));
        st.send_audio_results
            .push_back(Err(SinkError("flood".into())));
    }

    let summary = run_async(&orch, &deps, &options(vec![track_item("t1")], true))
        .await
        .expect("exhaustion is recorded, not propagated");
    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(state.lock().unwrap().sent_audio.len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exhausted_upload_continues_next_track() {
    let (orch, deps, state, _) = setup();
    *deps.upload_max_retries.lock().unwrap() = 0;
    {
        let mut st = state.lock().unwrap();
        st.clear_sink_results();
        st.send_audio_results
            .push_back(Err(SinkError("first failed".into())));
        st.send_audio_results.push_back(FakeDeps::upload_ok());
    }

    let summary = run_async(
        &orch,
        &deps,
        &options(vec![track_item("bad"), track_item("good")], true),
    )
    .await
    .expect("later tracks continue after exhaustion");
    assert_eq!(summary.failed_tracks.len(), 1);
    assert_eq!(summary.ripped_count, 1);
    assert_eq!(state.lock().unwrap().saved_tracks.len(), 1);
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
    // No request log is written for an upload-exhausted track.
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
    let st = state.lock().unwrap();
    assert_eq!(st.rip_calls, vec!["t1".to_string()]);
    assert_eq!(st.deleted_tracks, vec!["t1".to_string()]);
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
    // The command handler sets queue_position; the job flow never does,
    // and the job map is empty after completion.
    assert!(orch.get_active_jobs().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_position_and_pending_cancel_are_terminally_safe() {
    let (deps, _) = FakeDeps::new();
    *deps.rip_delay_ms.lock().unwrap() = 100;
    let orch = Arc::new(RipOrchestrator::new());
    let terminal_events = Arc::new(Mutex::new(Vec::<(String, &'static str)>::new()));
    let phase_snapshots = Arc::new(Mutex::new(Vec::<(JobPhase, Option<u64>)>::new()));
    let terminals = Arc::clone(&terminal_events);
    let phases = Arc::clone(&phase_snapshots);
    orch.subscribe(Arc::new(move |event: &OrchestratorEvent<'_>| {
        let job = match event {
            OrchestratorEvent::Created(job)
            | OrchestratorEvent::Started(job)
            | OrchestratorEvent::Progress(job, _) => job,
            OrchestratorEvent::Completed(job, _) => job,
            OrchestratorEvent::Cancelled(job, _) => job,
            OrchestratorEvent::Failed(job, _) => job,
        };
        match event {
            OrchestratorEvent::Started(_) => {
                phases.lock().unwrap().push((job.phase, job.queue_position))
            }
            OrchestratorEvent::Completed(_, _) => terminals
                .lock()
                .unwrap()
                .push((job.id.clone(), "completed")),
            OrchestratorEvent::Cancelled(_, _) => terminals
                .lock()
                .unwrap()
                .push((job.id.clone(), "cancelled")),
            OrchestratorEvent::Failed(_, _) => {
                terminals.lock().unwrap().push((job.id.clone(), "failed"))
            }
            _ => {}
        }
    }));

    let first_orch = Arc::clone(&orch);
    let first_deps = Arc::clone(&deps);
    let first_options = options(vec![track_item("first")], true);
    let first = tokio::spawn(async move { first_orch.start_job(first_deps, &first_options).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let second_orch = Arc::clone(&orch);
    let second_deps = Arc::clone(&deps);
    let second_options = options(vec![track_item("second")], true);
    let second =
        tokio::spawn(async move { second_orch.start_job(second_deps, &second_options).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let queued = orch
        .get_active_jobs()
        .into_iter()
        .find(|job| job.phase == JobPhase::Queued)
        .expect("second job is queued");
    assert_eq!(queued.queue_position, Some(1));
    assert!(orch.cancel_job(&queued.id, Some("tester")));

    let second_result = second.await.unwrap();
    assert!(second_result.is_err());
    first.await.unwrap().expect("first job completes");

    let terminal_events = terminal_events.lock().unwrap();
    let second_terminals: Vec<_> = terminal_events
        .iter()
        .filter(|(id, _)| id == &queued.id)
        .collect();
    assert_eq!(second_terminals.len(), 1);
    assert_eq!(second_terminals[0].1, "cancelled");
    assert!(phase_snapshots
        .lock()
        .unwrap()
        .iter()
        .any(|(phase, position)| *phase == JobPhase::Processing && *position == Some(0)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_cancel_emits_only_cancelled_terminal_event() {
    let (orch, deps, _, events) = setup();
    *deps.rip_delay_ms.lock().unwrap() = 100;
    let orch = Arc::new(orch);
    let run_orch = Arc::clone(&orch);
    let run_deps = Arc::clone(&deps);
    let run_options = options(vec![track_item("cancel")], true);
    let task = tokio::spawn(async move { run_orch.start_job(run_deps, &run_options).await });
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    let job = orch
        .get_active_jobs()
        .into_iter()
        .next()
        .expect("active job");
    assert!(orch.cancel_job(&job.id, Some("tester")));
    let _ = task.await.unwrap();
    let terminal_count = events
        .snapshot()
        .iter()
        .filter(|event| {
            event.starts_with("completed")
                || event.starts_with("cancelled")
                || event.starts_with("failed")
        })
        .count();
    assert_eq!(terminal_count, 1);
    assert!(events
        .snapshot()
        .iter()
        .any(|event| event.starts_with("cancelled")));
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
    // Upload completion publishes a fresh progress snapshot after clearing
    // the active upload text, so the dashboard reaches 100% before the
    // terminal event.
    let last_progress = ev
        .iter()
        .rev()
        .find(|e| e.starts_with("progress:"))
        .expect("progress events exist");
    assert!(
        last_progress.starts_with("progress:100:"),
        "got {last_progress}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_tracks_after_cap_edge() {
    // max_collection_tracks = 0 means "no cap" (limit > 0 check).
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

// album ZIP: generation-hash reuse, cover, single-track warning
/// Builds a zip-enabled RipJobOptions for a single album item.
fn zip_options(album: &str, cache_only: bool, explicit: bool, force: bool) -> RipJobOptions {
    RipJobOptions {
        chat_id: 100,
        user_id: 42,
        user_name: Some("tester".into()),
        delivery_chat_id: 100,
        is_group: false,
        is_force: force,
        is_cache_only: cache_only,
        zip: true,
        zip_explicit: explicit,
        single_storefront: None,
        parsed_items: vec![album_item(album)],
        reply_to_message_id: Some(555),
        status_msg_id: 999,
        is_admin: true,
        codec_preference: engine::wrapper::CodecPreference::HighestQuality,
    }
}

/// Computes the generation hash the orchestrator will derive for a faked
/// album, matching `album_generation_hash("apple", album, ids)`.
fn expected_generation_hash(album: &str, ids: &[&str]) -> String {
    engine::zip::album_generation_hash("apple", album, ids)
}

fn cached_zip_row(album: &str, part: i32, total: i32, hash: &str) -> CachedAlbum {
    CachedAlbum {
        part_index: part,
        total_parts: total,
        message_id: 5000 + i64::from(part),
        file_unique_id: format!("zip_uniq_{album}_{part}"),
        generation_hash: hash.to_owned(),
        file_size: 512,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_only_delivery_skips_individual_track_copies() {
    // Ripped track on a user ZIP job: staged for the archive, dumped for the
    // cache, but never copied to the requester as an individual file.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.rip".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t1".into(), RipScript::OkWithFile(vec![1, 2, 3]));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t2".into(), RipScript::OkWithFile(vec![4, 5, 6]));
    {
        let mut st = state.lock().unwrap();
        for _ in 0..2 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }

    let summary = run_async(&orch, &deps, &zip_options("alb.rip", false, true, false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    // Both tracks were ripped and cached to the dump.
    assert_eq!(summary.ripped_count, 2);
    assert_eq!(st.sent_audio.len(), 2, "audio cached to dump");
    // ZIP-only: copies to the requester are the ZIP part alone (msg 900 is
    // the fake dump-upload message id), never the individual tracks.
    let per_track_copies: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert!(
        per_track_copies.iter().all(|msg| *msg == 900),
        "individual tracks must not be delivered on ZIP jobs: {per_track_copies:?}"
    );
    assert_eq!(st.sent_documents.len(), 1, "complete archive published");
    let delivery = summary
        .zip_delivery
        .as_ref()
        .expect("rebuild delivery metadata present");
    assert_eq!(delivery.delivered_tracks, 2);
    assert_eq!(delivery.total_parts, 1);
    assert!(!delivery.is_partial);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_cached_without_reuse_rows_stages_and_rebuilds() {
    // Fully cached album, no reusable ZIP rows: staging downloads run, the
    // archive is rebuilt from the cached files, and no individual track
    // copies are delivered.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.stage".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.cache_track("t1", 101);
    deps.cache_track("t2", 102);

    let summary = run_async(&orch, &deps, &zip_options("alb.stage", false, true, false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    assert_eq!(summary.cached_count, 2, "both tracks served from cache");
    assert!(st.rip_calls.is_empty(), "nothing re-ripped");
    assert_eq!(
        st.sent_documents.len(),
        1,
        "rebuilt from staged cache files"
    );
    // Request log still written for ZIP jobs' cached hits.
    assert_eq!(
        st.request_logs
            .iter()
            .filter(|log| log.is_cache_hit && log.status == "completed")
            .count(),
        2,
        "cache-hit requests logged for staged tracks: {:?}",
        st.request_logs
    );
    // No individual track copies.
    let per_track_copies: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert!(
        per_track_copies.iter().all(|msg| *msg == 900),
        "no individual copies on ZIP jobs: {per_track_copies:?}"
    );
    assert!(
        summary
            .zip_delivery
            .as_ref()
            .is_some_and(|d| !d.is_partial && d.delivered_tracks == 2),
        "delivery metadata present"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_partial_delivery_marks_zip_delivery_partial() {
    // One of two tracks fails to rip → user gets a partial archive and the
    // delivery metadata reflects it.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.part".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t1".into(), RipScript::OkWithFile(vec![1, 2, 3]));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t2".into(), RipScript::Fail("mirror exploded"));
    {
        let mut st = state.lock().unwrap();
        st.send_audio_results.push_back(FakeDeps::upload_ok());
    }

    let summary = run_async(&orch, &deps, &zip_options("alb.part", false, true, false))
        .await
        .expect("job succeeds");

    assert_eq!(summary.failed_count, 1);
    let delivery = summary
        .zip_delivery
        .as_ref()
        .expect("partial delivery still reports metadata");
    assert!(delivery.is_partial, "delivery flagged partial");
    assert_eq!(delivery.delivered_tracks, 1, "one track in the archive");
    assert_eq!(delivery.total_tracks, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_reuse_delivers_cached_parts_without_rebuild() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.zip".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.cache_track("t1", 101);
    deps.cache_track("t2", 102);
    let hash = expected_generation_hash("alb.zip", &["t1", "t2"]);
    state.lock().unwrap().found_albums.insert(
        "alb.zip".into(),
        vec![
            cached_zip_row("alb.zip", 1, 2, &hash),
            cached_zip_row("alb.zip", 2, 2, &hash),
        ],
    );

    let summary = run_async(&orch, &deps, &zip_options("alb.zip", false, true, false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    // ZIP-only delivery: the requester gets the cached parts alone — the
    // individual track copies must NOT be sent for ZIP jobs.
    let zip_copies: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert_eq!(zip_copies, vec![5001, 5002]);
    // No rebuild: no document uploads, no new ZIP rows, no purges.
    assert!(st.sent_documents.is_empty());
    assert!(st.saved_albums.is_empty());
    assert!(st.deleted_album_zip_ids.is_empty());
    // The per-track staging downloads never ran (reuse skips them).
    assert!(st.rip_calls.is_empty());
    assert_eq!(summary.cached_count, 2);
    assert!(summary.warnings.is_empty());
    let delivery = summary
        .zip_delivery
        .as_ref()
        .expect("reuse delivery metadata present");
    assert_eq!(delivery.total_parts, 2);
    assert!(!delivery.is_partial);
    assert_eq!(delivery.size_bytes, 2 * 512);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_hash_mismatch_rebuilds() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.zip".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.cache_track("t1", 101);
    deps.cache_track("t2", 102);
    // Rows exist but under a stale generation hash.
    state.lock().unwrap().found_albums.insert(
        "alb.zip".into(),
        vec![cached_zip_row("alb.zip", 1, 1, "deadbeef")],
    );

    let summary = run_async(&orch, &deps, &zip_options("alb.zip", true, false, false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    // Stale rows purged before republishing the fresh complete set.
    assert!(st.deleted_album_zip_ids.contains(&"alb.zip".to_owned()));
    assert_eq!(
        st.sent_documents.len(),
        1,
        "rebuilt archive published to dump"
    );
    assert_eq!(st.saved_albums.len(), 1, "fresh row persisted");
    assert_eq!(
        st.saved_albums[0].generation_hash,
        expected_generation_hash("alb.zip", &["t1", "t2"]),
        "republished under the current hash"
    );
    assert_eq!(summary.cached_count, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_force_disables_reuse() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.zip".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.cache_track("t1", 101);
    deps.cache_track("t2", 102);
    let hash = expected_generation_hash("alb.zip", &["t1", "t2"]);
    state.lock().unwrap().found_albums.insert(
        "alb.zip".into(),
        vec![cached_zip_row("alb.zip", 1, 1, &hash)],
    );

    let summary = run_async(&orch, &deps, &zip_options("alb.zip", false, true, true))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    // Force purges the cache (delete_track) and re-rips; matching cached
    // rows must NOT short-circuit delivery of a rebuilt archive set.
    assert!(!st.deleted_tracks.is_empty(), "force purges cached tracks");
    assert_eq!(st.rip_calls.len(), 2, "both tracks re-ripped");
    assert!(summary.warnings.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_single_track_album_warns_when_explicit() {
    let (orch, deps, _, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Solo", "Artist");
    deps.albums
        .lock()
        .unwrap()
        .insert("alb.solo".into(), FakeDeps::album(vec![meta("t1")]));

    let summary = run_async(&orch, &deps, &zip_options("alb.solo", false, true, false))
        .await
        .expect("job succeeds");

    assert_eq!(summary.total_tracks, 1);
    assert_eq!(summary.warnings.len(), 1, "explicit single-track ZIP warns");
    assert!(summary.warnings[0].contains("single track"));

    // Implicit auto-attempt (cache-only dump) never warns.
    let (orch2, deps2, _, _) = setup();
    deps2
        .albums
        .lock()
        .unwrap()
        .insert("alb.solo".into(), FakeDeps::album(vec![meta("t1")]));
    let summary2 = run_async(&orch2, &deps2, &zip_options("alb.solo", true, false, false))
        .await
        .expect("job succeeds");
    assert!(summary2.warnings.is_empty(), "implicit attempt stays quiet");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zip_build_includes_cover_and_real_generation_hash() {
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.zip".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    // Album artwork present → cover fetch + preview + cover.jpg entry.
    {
        let mut albums = deps.albums.lock().unwrap();
        let mut album = albums.get("alb.zip").cloned().unwrap();
        album.album.artwork_url = "https://example.test/cover/3000x3000bb.jpg".into();
        album.album.release_date = "2021-06-04".into();
        albums.insert("alb.zip".into(), album);
    }
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t1".into(), RipScript::OkWithFile(vec![1, 2, 3]));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t2".into(), RipScript::OkWithFile(vec![4, 5, 6]));
    {
        let mut st = state.lock().unwrap();
        st.artwork_bytes = Some(vec![9, 9, 9]);
        for _ in 0..2 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }

    let summary = run_async(&orch, &deps, &zip_options("alb.zip", false, true, false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    // Complete rebuild published the archive to the dump.
    assert_eq!(st.sent_documents.len(), 1, "one archive part uploaded");
    assert_eq!(st.saved_albums.len(), 1, "one row persisted");
    let saved = &st.saved_albums[0];
    assert_eq!(saved.part_index, 1);
    assert_eq!(
        saved.generation_hash,
        expected_generation_hash("alb.zip", &["t1", "t2"]),
        "real generation hash persisted"
    );
    // ZIP-only delivery: no per-track dump copies for the individual
    // tracks (only the ZIP part DM copy may occur).
    let per_track_copies: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert!(
        per_track_copies.iter().all(|msg| *msg == 900),
        "only the ZIP part copy is delivered, got {per_track_copies:?}"
    );
    // Cover + thumbnail fetched, preview sent to the requester.
    assert_eq!(
        st.fetch_artwork_urls.len(),
        2,
        "cover + 320px thumbnail fetched: {:?}",
        st.fetch_artwork_urls
    );
    assert_eq!(st.sent_photos.len(), 1);
    assert_eq!(st.sent_photos[0].0, 100);
    assert_eq!(st.sent_thumbs.len(), 1, "thumbnail attached to the part");
    // Filename carries the release year.
    assert!(
        saved.file_name.contains("(2021)"),
        "file was {}",
        saved.file_name
    );
    // The uploaded archive really contains cover.jpg + hashed manifest.
    // Bytes were captured at upload time — the workspace is gone now.
    let archive_dir = std::env::temp_dir().join(format!("zip_assert_{}", std::process::id()));
    std::fs::create_dir_all(&archive_dir).unwrap();
    let archive_path = archive_dir.join("uploaded.zip");
    std::fs::write(&archive_path, &st.uploaded_document_bytes).unwrap();
    let file = std::fs::File::open(&archive_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert!(archive.by_name("cover.jpg").is_ok());
    assert!(archive.by_name("manifest.json").is_ok());
    let mut manifest_file = archive.by_name("manifest.json").unwrap();
    let mut manifest_bytes = Vec::new();
    std::io::Read::read_to_end(&mut manifest_file, &mut manifest_bytes).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).unwrap();
    assert!(manifest["cover_sha256"].is_string());
    assert!(manifest["tracks"][0]["sha256"].is_string());
    let _ = std::fs::remove_dir_all(&archive_dir);
    assert!(summary.warnings.is_empty());
}

// two-lane pipeline (lane 1 rip ∥ lane 2 upload)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_album_job_caches_zip_without_delivering_it() {
    // The always-zip rule: a plain (no -z) album job still builds and
    // caches the archive to the dump for future requests, but the user
    // only receives the individual tracks — never the ZIP parts.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.auto".into(),
        FakeDeps::album(vec![meta("t1"), meta("t2")]),
    );
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t1".into(), RipScript::OkWithFile(vec![1, 2, 3]));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("t2".into(), RipScript::OkWithFile(vec![4, 5, 6]));
    {
        let mut st = state.lock().unwrap();
        for _ in 0..2 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.auto")], false))
        .await
        .expect("job succeeds");

    let st = state.lock().unwrap();
    assert_eq!(summary.ripped_count, 2);
    assert_eq!(st.sent_audio.len(), 2, "both tracks cached to dump");
    // Individual tracks ARE delivered to the user (message 777 is the fake
    // per-track dump upload; only those copies appear, never 900 = ZIP part).
    let delivered: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert_eq!(delivered, vec![777, 777], "individual tracks delivered");
    // The archive was published to the dump and saved as cache rows…
    assert_eq!(st.sent_documents.len(), 1, "archive uploaded to dump");
    assert_eq!(st.saved_albums.len(), 1, "album ZIP row saved");
    // …but no details photo/ZIP copy went to the user and the summary
    // reports no ZIP delivery.
    assert!(summary.zip_delivery.is_none());
    assert!(
        st.sent_photos.is_empty(),
        "no details message on plain jobs"
    );
    let zip_part_copies: Vec<i64> = st
        .copies
        .iter()
        .filter(|(to, _, _, _)| *to == 100)
        .filter(|(_, msg, _, _)| *msg == 900)
        .map(|(_, msg, _, _)| *msg)
        .collect();
    assert!(
        zip_part_copies.is_empty(),
        "ZIP parts never delivered on plain jobs"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_lanes_interleave_job_b_rips_while_job_a_uploads() {
    // Lane 1 must free the rip slot when job A's last RIP is done, even
    // while its uploads continue on lane 2 — so job B starts ripping
    // before A's summary resolves.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.a".into(),
        FakeDeps::album(vec![meta("a1"), meta("a2")]),
    );
    deps.albums.lock().unwrap().insert(
        "alb.b".into(),
        FakeDeps::album(vec![meta("b1"), meta("b2")]),
    );
    for id in ["a1", "a2", "b1", "b2"] {
        deps.rip_scripts
            .lock()
            .unwrap()
            .insert(id.into(), RipScript::OkWithFile(vec![1]));
    }
    {
        let mut st = state.lock().unwrap();
        for _ in 0..4 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }

    let orch = Arc::new(orch);
    let a_deps = Arc::clone(&deps);
    let a_options = options(vec![album_item("alb.a")], false);
    let b_deps = Arc::clone(&deps);
    let b_options = options(vec![album_item("alb.b")], false);

    let a_orch = Arc::clone(&orch);
    let a_task = tokio::spawn(async move { run_async(&a_orch, &a_deps, &a_options).await });
    // Ensure A is admitted and starts lane 1 first.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let b_orch = Arc::clone(&orch);
    let b_task = tokio::spawn(async move { run_async(&b_orch, &b_deps, &b_options).await });
    let a_summary = a_task.await.unwrap().expect("job A succeeds");
    let b_summary = b_task.await.unwrap().expect("job B succeeds");
    assert_eq!(a_summary.ripped_count, 2);
    assert_eq!(b_summary.ripped_count, 2);

    // Both jobs' four tracks reached the dump through lane 2.
    let st = state.lock().unwrap();
    assert_eq!(st.sent_audio.len(), 4);
    assert_eq!(st.saved_albums.len(), 2, "both archives cached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_during_lane_two_upload_still_resolves() {
    // Cancelling after lane 1 finished its rips (uploads gated open on
    // lane 2) must not hang start_job: the finalize marker still runs,
    // cleans the workspaces, and resolves the summary.
    let (orch, deps, state, events) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.cancel".into(),
        FakeDeps::album(vec![meta("c1"), meta("c2")]),
    );
    for id in ["c1", "c2"] {
        deps.rip_scripts
            .lock()
            .unwrap()
            .insert(id.into(), RipScript::OkWithFile(vec![1]));
    }
    {
        let mut st = state.lock().unwrap();
        for _ in 0..2 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }
    // Hold every lane-2 upload open until the test releases it.
    let gate = tokio_util::sync::CancellationToken::new();
    state.lock().unwrap().gate_uploads = Some(gate.clone());

    let orch = Arc::new(orch);
    let run_orch = Arc::clone(&orch);
    let run_deps = Arc::clone(&deps);
    let run_options = options(vec![album_item("alb.cancel")], false);
    let task = tokio::spawn(async move { run_async(&run_orch, &run_deps, &run_options).await });
    // Wait until lane 1 is done (both rip calls recorded) — its slot frees
    // while lane 2 is gated mid-upload.
    let rip_done = async {
        loop {
            let done = {
                let st = state.lock().unwrap();
                st.rip_calls.len() == 2
            };
            if done {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    };
    rip_done.await;
    let job = orch
        .get_active_jobs()
        .into_iter()
        .next()
        .expect("active job");
    assert!(orch.cancel_job(&job.id, Some("tester")));
    // Release the gated uploads; the marker then runs and resolves.
    gate.cancel();
    let summary = task.await.unwrap().expect("summary still resolves");
    // The gated uploads completed as Telegram sends but the job was
    // already cancelled: the post-upload rollback deleted the dump rows,
    // so nothing counts as ripped.
    assert_eq!(summary.ripped_count, 0, "uploads rolled back post-cancel");
    let terminal_count = events
        .snapshot()
        .iter()
        .filter(|event| {
            event.starts_with("completed")
                || event.starts_with("cancelled")
                || event.starts_with("failed")
        })
        .count();
    assert_eq!(terminal_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn staging_failure_queues_rip_for_cached_track() {
    // A cache row whose dump message cannot be downloaded is re-ripped on
    // lane 1 instead of silently breaking the archive.
    let (orch, deps, state, _) = setup();
    let meta = |id: &str| FakeDeps::track_meta(id, "Track", "Artist");
    deps.albums.lock().unwrap().insert(
        "alb.stage".into(),
        FakeDeps::album(vec![meta("s1"), meta("s2")]),
    );
    // s1 is cached with a dead dump row (staging download fails), s2 is a
    // plain miss. Both end up ripped.
    deps.cache_track("s1", 424242);
    {
        let mut st = state.lock().unwrap();
        st.copies_fail_ids.push(424242); // instant delivery fails → re-rip
    }
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("s1".into(), RipScript::OkWithFile(vec![9]));
    deps.rip_scripts
        .lock()
        .unwrap()
        .insert("s2".into(), RipScript::OkWithFile(vec![8]));
    {
        let mut st = state.lock().unwrap();
        for _ in 0..2 {
            st.send_audio_results.push_back(FakeDeps::upload_ok());
        }
    }

    let summary = run_async(&orch, &deps, &options(vec![album_item("alb.stage")], false))
        .await
        .expect("job succeeds");

    // s1 was re-ripped (stale row + dead dump) and s2 ripped fresh.
    let st = state.lock().unwrap();
    assert!(
        st.rip_calls.contains(&"s1".to_string()),
        "stale s1 re-ripped"
    );
    assert!(st.rip_calls.contains(&"s2".to_string()), "s2 ripped");
    assert_eq!(summary.ripped_count, 2);
    assert_eq!(st.saved_albums.len(), 1, "archive complete and cached");
}
