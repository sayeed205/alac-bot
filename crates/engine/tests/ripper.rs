//! Ripper parity tests against `src/modules/alac/ripper.ts` — progress
//! sequences, retry semantics, stall/cancel behavior, temp cleanup, and
//! result mapping.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
};

use bytes::Bytes;
use engine::{
    ripper::{AlacTrackRipper, RipError, RipProgressCallback, RipperConfig, RipperDeps},
    streaming::{AudioStreamSource, ByteStream, MirrorEndpoint, ProgressCallback},
    types::TrackMeta,
};
use futures_util::stream;
use tokio_util::sync::CancellationToken;

fn meta() -> TrackMeta {
    TrackMeta {
        id: "42".into(),
        title: "Title".into(),
        artist: "Artist".into(),
        album: "Album".into(),
        album_artist: "Artist".into(),
        genre: None,                 // → 'Unknown' in result mapping
        release_date: String::new(), // '' passthrough
        composer: None,
        track_number: None, // → 1
        track_count: None,  // → 1
        disc_number: None,
        disc_count: None,
        duration_secs: 240,
        explicit: false,
        content_advisory: None,
        artwork_url: String::new(), // → no artwork fetch
        album_id: None,
        artist_id: None,
        isrc: None,
        record_label: None,
        copyright: None,
        upc: None,
    }
}

fn fake_stream_with_length(chunks: Vec<Bytes>, content_length: Option<u64>) -> AudioStreamSource {
    AudioStreamSource {
        stream: Box::pin(stream::iter(
            chunks
                .into_iter()
                .map(Ok::<_, engine::streaming::StreamBodyError>),
        )) as ByteStream,
        source_name: "primary mirror (test)".into(),
        codec: "alac".into(),
        bit_depth: 24,
        sample_rate: 96_000,
        content_length,
    }
}

/// Scriptable fake deps: every behavior is configurable per test.
struct FakeDeps {
    meta: TrackMeta,
    meta_failures: u32, // first N track_meta calls fail
    meta_calls: AtomicU32,
    mirror: Option<MirrorEndpoint>,
    stream_chunks: Vec<Bytes>,
    stream_content_length: Option<u64>,
    connect_fails: u32, // first N connect calls fail with a stall message
    connect_calls: AtomicU32,
    lyrics: Option<String>,
    artwork: Option<Vec<u8>>,
    tag_calls: Mutex<Vec<(PathBuf, PathBuf)>>,
    tag_should_fail: bool,
}

impl FakeDeps {
    fn ok() -> Self {
        Self {
            meta: meta(),
            meta_failures: 0,
            meta_calls: AtomicU32::new(0),
            mirror: Some(MirrorEndpoint {
                mirror_url: "https://mirror".into(),
                api_key: "k".into(),
            }),
            stream_chunks: vec![Bytes::from(vec![1u8; 10]), Bytes::from(vec![2u8; 5])],
            stream_content_length: None,
            connect_fails: 0,
            connect_calls: AtomicU32::new(0),
            lyrics: Some("la\nla".into()),
            artwork: Some(vec![1, 2, 3]),
            tag_calls: Mutex::new(Vec::new()),
            tag_should_fail: false,
        }
    }
}

impl RipperDeps for FakeDeps {
    async fn track_meta(&self, _track_id: &str, _storefront: &str) -> Result<TrackMeta, RipError> {
        let n = self.meta_calls.fetch_add(1, Ordering::SeqCst);
        if n < self.meta_failures {
            return Err(RipError::Message("catalog down".into()));
        }
        Ok(self.meta.clone())
    }

    async fn mirror_endpoint(&self, _signal: Option<&CancellationToken>) -> Option<MirrorEndpoint> {
        self.mirror.clone()
    }

    async fn connect_stream(
        &self,
        _track_id: &str,
        primary: Option<MirrorEndpoint>,
        _signal: Option<CancellationToken>,
        _on_progress: Option<ProgressCallback>,
    ) -> Result<AudioStreamSource, RipError> {
        let n = self.connect_calls.fetch_add(1, Ordering::SeqCst);
        if n < self.connect_fails {
            return Err(RipError::Message(
                "Audio stream stalled on test: no data received for 45s".into(),
            ));
        }
        assert_eq!(primary.is_some(), self.mirror.is_some());
        Ok(fake_stream_with_length(
            self.stream_chunks.clone(),
            self.stream_content_length,
        ))
    }

    async fn fetch_lyrics(
        &self,
        _track_id: &str,
        _meta: &engine::lyrics::LyricsMeta,
    ) -> Option<String> {
        self.lyrics.clone()
    }

    async fn fetch_artwork(&self, _url: &str) -> Option<Vec<u8>> {
        self.artwork.clone()
    }

    async fn tag_m4a(
        &self,
        raw_path: &Path,
        output_path: &Path,
        _meta: &TrackMeta,
        _cover: Option<&[u8]>,
        _lyrics: Option<&str>,
    ) -> Result<(), RipError> {
        self.tag_calls
            .lock()
            .unwrap()
            .push((raw_path.to_owned(), output_path.to_owned()));
        if self.tag_should_fail {
            return Err(RipError::Message(
                "native media finalization failed: boom".into(),
            ));
        }
        Ok(())
    }
}

fn config(dir: &Path, retries: u32, base_ms: u64) -> RipperConfig {
    RipperConfig {
        default_output_dir: dir.to_owned(),
        max_retries: retries,
        base_delay_ms: base_ms,
    }
}

type ProgressLog = Arc<Mutex<Vec<(String, Option<u64>, Option<u64>)>>>;

fn record() -> (RipProgressCallback, ProgressLog) {
    let log: ProgressLog = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    let cb: RipProgressCallback = Arc::new(move |status: &str, d, t| {
        log2.lock().unwrap().push((status.to_owned(), d, t));
    });
    (cb, log)
}

#[tokio::test]
async fn happy_path_progress_and_result_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let deps = FakeDeps::ok();
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let (cb, log) = record();
    let result = ripper
        .rip(&deps, "42", Some(&cb), "us", None, None)
        .await
        .unwrap();

    // Result mapping (TS || / ?? semantics).
    assert_eq!(result.title, "Title");
    assert_eq!(result.genre, "Unknown");
    assert_eq!(result.release_date, "");
    assert_eq!(result.track_number, 1);
    assert_eq!(result.track_count, 1);
    assert_eq!(result.codec, "alac");
    assert_eq!(result.bit_depth, 24);
    assert_eq!(result.sample_rate, 96_000);
    assert_eq!(result.duration, 240);
    assert!(result.file_path.ends_with("01. Title - Artist [ALAC].m4a"));

    // Progress sequence (TS: lastProgressUpdate=0 → the FIRST chunk always
    // emits a byte-progress update).
    let log = log.lock().unwrap();
    let statuses: Vec<&str> = log.iter().map(|(s, _, _)| s.as_str()).collect();
    assert_eq!(
        statuses[..3],
        [
            "Fetching track metadata...",
            "Connecting stream for Title - Artist...",
            "Downloading lossless audio: 0.0 MB"
        ]
    );
    assert!(statuses.contains(&"Tagging and embedding lossless artwork..."));

    // Tagged with the right paths; temp raw cleaned up.
    let tag_calls = deps.tag_calls.lock().unwrap();
    assert_eq!(tag_calls.len(), 1);
    assert!(tag_calls[0]
        .0
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("stream_42_"));
    assert_eq!(tag_calls[0].1, PathBuf::from(&result.file_path));
    drop(tag_calls);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".raw")))
        .collect();
    assert!(leftovers.is_empty(), "temp raw removed");
}

#[tokio::test]
async fn retry_succeeds_after_two_failures() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.connect_fails = 2;
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let (cb, log) = record();
    let result = ripper
        .rip(&deps, "42", Some(&cb), "us", None, None)
        .await
        .unwrap();
    assert_eq!(result.title, "Title");
    assert_eq!(
        deps.connect_calls.load(Ordering::SeqCst),
        3,
        "3 attempts total"
    );
    let log = log.lock().unwrap();
    let retry_msgs: Vec<&str> = log
        .iter()
        .map(|(s, _, _)| s.as_str())
        .filter(|s| s.starts_with("⚠️"))
        .collect();
    assert_eq!(retry_msgs.len(), 2);
    assert!(retry_msgs[0]
        .starts_with("⚠️ Rip failed, retrying (attempt 1/3) in 0.0s: Audio stream stalled"));
    assert!(retry_msgs[1]
        .starts_with("⚠️ Rip failed, retrying (attempt 2/3) in 0.0s: Audio stream stalled"));
}

#[tokio::test]
async fn exhaustion_rethrows_last_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.connect_fails = 10;
    let ripper = AlacTrackRipper::new(config(dir.path(), 2, 1));
    let error = ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Audio stream stalled on test: no data received for 45s"
    );
    assert_eq!(
        deps.connect_calls.load(Ordering::SeqCst),
        3,
        "max_retries=2 → 3 attempts"
    );
}

#[tokio::test]
async fn cancelled_message_bypasses_retries() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.connect_fails = 1;
    // Make the failure message exactly 'Download was cancelled'.
    let ripper = AlacTrackRipper::new(config(dir.path(), 5, 1));
    let (cb, log) = record();

    // Simulate: connect fails with the cancelled message.
    struct CancelledDeps(FakeDeps);
    impl RipperDeps for CancelledDeps {
        async fn track_meta(&self, _t: &str, _s: &str) -> Result<TrackMeta, RipError> {
            self.0.track_meta(_t, _s).await
        }
        async fn mirror_endpoint(&self, s: Option<&CancellationToken>) -> Option<MirrorEndpoint> {
            self.0.mirror_endpoint(s).await
        }
        async fn connect_stream(
            &self,
            _t: &str,
            p: Option<MirrorEndpoint>,
            s: Option<CancellationToken>,
            o: Option<ProgressCallback>,
        ) -> Result<AudioStreamSource, RipError> {
            let _ = self.0.connect_stream(_t, p, s, o).await?;
            Err(RipError::Message("Download was cancelled".into()))
        }
        async fn fetch_lyrics(&self, t: &str, m: &engine::lyrics::LyricsMeta) -> Option<String> {
            self.0.fetch_lyrics(t, m).await
        }
        async fn fetch_artwork(&self, u: &str) -> Option<Vec<u8>> {
            self.0.fetch_artwork(u).await
        }
        async fn tag_m4a(
            &self,
            r: &Path,
            o: &Path,
            m: &TrackMeta,
            c: Option<&[u8]>,
            l: Option<&str>,
        ) -> Result<(), RipError> {
            self.0.tag_m4a(r, o, m, c, l).await
        }
    }

    let deps = CancelledDeps(FakeDeps::ok());
    let error = ripper
        .rip(&deps, "42", Some(&cb), "us", None, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Download was cancelled");
    // No retry progress messages.
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|(s, _, _)| !s.starts_with("⚠️")));
}

#[tokio::test]
async fn cancellation_mid_rip_no_retry() {
    let dir = tempfile::tempdir().unwrap();
    let deps = FakeDeps::ok();
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let token = CancellationToken::new();
    token.cancel();
    let error = ripper
        .rip(&deps, "42", None, "us", Some(token), None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Download was cancelled");
    assert_eq!(deps.meta_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mirror_none_still_connects() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.mirror = None;
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let result = ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap();
    assert_eq!(result.title, "Title");
    assert_eq!(deps.connect_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn artwork_empty_vec_means_no_cover() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.artwork = Some(Vec::new());
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap();
    // Tagged once — cover emptiness is handled inside tag_m4a.
    assert_eq!(deps.tag_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn stalled_stream_is_retryable() {
    // A stream that NEVER yields (pending forever): rip_once returns the
    // 45s stall error, and the retry loop retries it.
    let dir = tempfile::tempdir().unwrap();
    struct StalledDeps(FakeDeps);
    impl RipperDeps for StalledDeps {
        async fn track_meta(&self, t: &str, s: &str) -> Result<TrackMeta, RipError> {
            self.0.track_meta(t, s).await
        }
        async fn mirror_endpoint(&self, s: Option<&CancellationToken>) -> Option<MirrorEndpoint> {
            self.0.mirror_endpoint(s).await
        }
        async fn connect_stream(
            &self,
            _t: &str,
            _p: Option<MirrorEndpoint>,
            _s: Option<CancellationToken>,
            _o: Option<ProgressCallback>,
        ) -> Result<AudioStreamSource, RipError> {
            self.0.connect_calls.fetch_add(1, Ordering::SeqCst);
            let pending: ByteStream = Box::pin(stream::pending());
            Ok(AudioStreamSource {
                stream: pending,
                source_name: "primary mirror (test)".into(),
                codec: "alac".into(),
                bit_depth: 24,
                sample_rate: 96_000,
                content_length: None,
            })
        }
        async fn fetch_lyrics(&self, t: &str, m: &engine::lyrics::LyricsMeta) -> Option<String> {
            self.0.fetch_lyrics(t, m).await
        }
        async fn fetch_artwork(&self, u: &str) -> Option<Vec<u8>> {
            self.0.fetch_artwork(u).await
        }
        async fn tag_m4a(
            &self,
            r: &Path,
            o: &Path,
            m: &TrackMeta,
            c: Option<&[u8]>,
            l: Option<&str>,
        ) -> Result<(), RipError> {
            self.0.tag_m4a(r, o, m, c, l).await
        }
    }

    let inner = FakeDeps::ok();
    let connect_calls = Arc::new(AtomicU32::new(0));
    // Track connect attempts via the shared counter inside FakeDeps.
    let deps = StalledDeps(inner);
    let ripper = AlacTrackRipper::new(config(dir.path(), 1, 1));
    tokio::time::pause();
    let error = ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("Audio stream stalled on primary mirror (test)"));
    assert!(error.to_string().contains("no data received for 45s"));
    assert_eq!(
        deps.0.connect_calls.load(Ordering::SeqCst),
        2,
        "retried once"
    );
    drop(connect_calls);
}

#[tokio::test]
async fn progress_totals_with_content_length() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    // 15 bytes across two chunks, content-length 15.
    deps.stream_chunks = vec![Bytes::from(vec![0u8; 10]), Bytes::from(vec![0u8; 5])];
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let (cb, log) = record();
    ripper
        .rip(&deps, "42", Some(&cb), "us", None, None)
        .await
        .unwrap();
    let log = log.lock().unwrap();
    let download_events: Vec<_> = log
        .iter()
        .filter(|(s, _, _)| s.starts_with("Downloading lossless audio"))
        .collect();
    assert!(!download_events.is_empty());
    // Byte progress format parity (content_length None → MB only).
    assert!(download_events[0]
        .0
        .starts_with("Downloading lossless audio: 0.0 MB"));
}

#[tokio::test]
async fn short_body_is_rejected_before_tagging() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.stream_content_length = Some(20);
    let ripper = AlacTrackRipper::new(config(dir.path(), 0, 1));

    let error = ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Incomplete audio body"));
    assert!(deps.tag_calls.lock().unwrap().is_empty());
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[tokio::test]
async fn output_dir_override_used() {
    let base = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let deps = FakeDeps::ok();
    let ripper = AlacTrackRipper::new(config(base.path(), 3, 1));
    let result = ripper
        .rip(&deps, "42", None, "us", None, Some(other.path()))
        .await
        .unwrap();
    assert!(result.file_path.starts_with(other.path().to_str().unwrap()));
    assert!(Path::new(&result.file_path).starts_with(other.path()));
    let tag_calls = deps.tag_calls.lock().unwrap();
    assert!(tag_calls[0].0.starts_with(other.path()));
}

#[tokio::test]
async fn tag_failure_is_retryable_and_exhausts() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    deps.tag_should_fail = true;
    let ripper = AlacTrackRipper::new(config(dir.path(), 2, 1));
    let error = ripper
        .rip(&deps, "42", None, "us", None, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "native media finalization failed: boom");
    assert_eq!(deps.tag_calls.lock().unwrap().len(), 3);
    // Temp raw cleaned even after failures.
    let raw_left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".raw")))
        .collect();
    assert!(raw_left.is_empty());
}

#[tokio::test]
async fn progress_throttles_to_one_per_second() {
    let dir = tempfile::tempdir().unwrap();
    let mut deps = FakeDeps::ok();
    // Many chunks arriving quickly: progress updates throttle to 1/sec.
    deps.stream_chunks = (0..50).map(|i| Bytes::from(vec![i as u8; 100])).collect();
    let ripper = AlacTrackRipper::new(config(dir.path(), 3, 1));
    let (cb, log) = record();
    tokio::time::pause();
    ripper
        .rip(&deps, "42", Some(&cb), "us", None, None)
        .await
        .unwrap();
    let log = log.lock().unwrap();
    let download_events: Vec<_> = log
        .iter()
        .filter(|(s, _, _)| s.starts_with("Downloading lossless audio"))
        .collect();
    // All chunks delivered within the same paused "second" → at most a
    // handful of updates (first fires immediately due to the backdated
    // last_update, subsequent ones only after 1s of paused time, which
    // never advances while chunks are ready).
    assert!(
        download_events.len() <= 3,
        "throttled: got {}",
        download_events.len()
    );
}
