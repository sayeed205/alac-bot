//! Single-track ripper: metadata → stream → raw file → tagged M4A, with
//! retries and cancellation. Port of `src/modules/alac/ripper.ts`.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use futures_util::{FutureExt, StreamExt};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    catalog::{Catalog, CatalogError, ReqwestTransport},
    limits::MAX_AUDIO_BYTES,
    lyrics::{self, LyricsHttp, LyricsMeta},
    streaming::{
        AudioStreamSource, MirrorEndpoint, MirrorPolicyManager, ProgressCallback, StreamError,
        StreamTransport,
    },
    tagger,
    types::{TrackMeta, TrackRipResult},
};

/// All rip failures reduce to messages (TS `new Error(msg)` parity).
#[derive(Debug, thiserror::Error)]
pub enum RipError {
    #[error("{0}")]
    Message(String),
}

impl From<StreamError> for RipError {
    fn from(error: StreamError) -> Self {
        RipError::Message(error.to_string())
    }
}

impl From<CatalogError> for RipError {
    fn from(error: CatalogError) -> Self {
        RipError::Message(error.to_string())
    }
}

impl From<std::io::Error> for RipError {
    fn from(error: std::io::Error) -> Self {
        RipError::Message(error.to_string())
    }
}

/// `(status, downloaded_bytes, total_bytes)` — the latter two present only
/// for the byte-progress updates.
pub type RipProgressCallback = Arc<dyn Fn(&str, Option<u64>, Option<u64>) + Send + Sync>;

/// Configuration knobs (TS constructor parity: retries 3, base delay 2s).
#[derive(Debug, Clone)]
pub struct RipperConfig {
    pub default_output_dir: PathBuf,
    pub max_retries: u32,
    pub base_delay_ms: u64,
}

impl Default for RipperConfig {
    fn default() -> Self {
        Self {
            default_output_dir: std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("bot-data")
                .join("downloads"),
            max_retries: 3,
            base_delay_ms: 2000,
        }
    }
}

/// Everything a rip needs, behind one seam so tests can fake each step.
pub trait RipperDeps: Send + Sync {
    fn track_meta(
        &self,
        track_id: &str,
        storefront: &str,
    ) -> impl Future<Output = Result<TrackMeta, RipError>> + Send;
    /// `None` = mirror lookup failed (TS catches → wrapper fallback path).
    fn mirror_endpoint(
        &self,
        signal: Option<&CancellationToken>,
    ) -> impl Future<Output = Option<MirrorEndpoint>> + Send;
    fn connect_stream(
        &self,
        track_id: &str,
        primary: Option<MirrorEndpoint>,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
    ) -> impl Future<Output = Result<AudioStreamSource, RipError>> + Send;
    fn fetch_lyrics(
        &self,
        track_id: &str,
        meta: &LyricsMeta,
    ) -> impl Future<Output = Option<String>> + Send;
    fn fetch_artwork(&self, url: &str) -> impl Future<Output = Option<Vec<u8>>> + Send;
    fn tag_m4a(
        &self,
        raw_path: &Path,
        output_path: &Path,
        meta: &TrackMeta,
        cover: Option<&[u8]>,
        lyrics: Option<&str>,
    ) -> impl Future<Output = Result<(), RipError>> + Send;
}

/// Retrying wrapper around `rip_once` (TS `rip` parity).
pub struct AlacTrackRipper {
    config: RipperConfig,
}

impl AlacTrackRipper {
    pub fn new(config: RipperConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &RipperConfig {
        &self.config
    }

    pub async fn rip<D: RipperDeps>(
        &self,
        deps: &D,
        track_id: &str,
        on_progress: Option<&RipProgressCallback>,
        storefront: &str,
        signal: Option<CancellationToken>,
        output_dir: Option<&Path>,
    ) -> Result<TrackRipResult, RipError> {
        let mut attempt: u32 = 0;

        loop {
            if signal.as_ref().is_some_and(|t| t.is_cancelled()) {
                return Err(cancelled());
            }

            match self
                .rip_once(
                    deps,
                    track_id,
                    on_progress,
                    storefront,
                    signal.as_ref(),
                    output_dir,
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(err) => {
                    let message = err.to_string();
                    if signal.as_ref().is_some_and(|t| t.is_cancelled())
                        || message == "Download was cancelled"
                    {
                        return Err(err);
                    }
                    if attempt >= self.config.max_retries {
                        return Err(err);
                    }
                    attempt += 1;

                    let raw_delay = self.config.base_delay_ms * 2u64.pow(attempt - 1);
                    let jitter = 0.8 + jitter_fraction() * 0.4;
                    let delay_ms = (raw_delay as f64 * jitter).round() as u64;

                    let wait_sec = delay_ms as f64 / 1000.0;
                    emit_progress(
                        on_progress,
                        &format!(
                            "⚠️ Rip failed, retrying (attempt {attempt}/{}) in {wait_sec:.1}s: {message}",
                            self.config.max_retries
                        ),
                        None,
                        None,
                    );
                    warn!(
                        track_id,
                        attempt,
                        max_retries = self.config.max_retries,
                        delay_ms,
                        error = %message,
                        "Track rip failed, retrying"
                    );

                    // Abortable sleep: cancellation rejects immediately.
                    if let Some(token) = &signal {
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
                            _ = token.cancelled() => return Err(cancelled()),
                        }
                    } else {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
    }

    async fn rip_once<D: RipperDeps>(
        &self,
        deps: &D,
        track_id: &str,
        on_progress: Option<&RipProgressCallback>,
        storefront: &str,
        signal: Option<&CancellationToken>,
        output_dir: Option<&Path>,
    ) -> Result<TrackRipResult, RipError> {
        let rip_start = std::time::Instant::now();

        if signal.is_some_and(|t| t.is_cancelled()) {
            return Err(cancelled());
        }

        let target_dir: &Path = output_dir.unwrap_or(&self.config.default_output_dir);
        tokio::fs::create_dir_all(target_dir).await?;
        let track_dir = target_dir.join(format!(".track_{}", unique_temp_suffix()));
        tokio::fs::create_dir_all(&track_dir).await?;

        emit_progress(on_progress, "Fetching track metadata...", None, None);
        let meta = deps.track_meta(track_id, storefront).await?;

        if signal.is_some_and(|t| t.is_cancelled()) {
            return Err(cancelled());
        }

        emit_progress(
            on_progress,
            &format!("Connecting stream for {} - {}...", meta.title, meta.artist),
            None,
            None,
        );

        // Concurrent prefetch: lyrics + artwork run while the audio streams.
        let lyrics_meta = LyricsMeta {
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            album: Some(meta.album.clone()).filter(|a| !a.is_empty()),
            duration: Some(meta.duration_secs).filter(|d| *d != 0),
        };
        let lyrics_task = {
            let meta = lyrics_meta.clone();
            let track_id = track_id.to_owned();
            async move {
                match deps.fetch_lyrics(&track_id, &meta).await {
                    Some(l) => {
                        debug!(track_id, found = true, "Lyrics prefetch completed");
                        Some(l)
                    }
                    None => {
                        debug!(track_id, found = false, "Lyrics prefetch completed");
                        None
                    }
                }
            }
        };
        let artwork_task = {
            let track_id = track_id.to_owned();
            let artwork_url = meta.artwork_url.clone();
            async move {
                if artwork_url.is_empty() {
                    return None;
                }
                let artwork = deps.fetch_artwork(&artwork_url).await;
                debug!(
                    track_id,
                    size_bytes = artwork.as_ref().map_or(0, Vec::len),
                    "Artwork prefetch completed"
                );
                artwork
            }
        };

        // 1. Primary mirror (failure is not fatal — wrapper fallback).
        let primary = deps.mirror_endpoint(signal).await;
        if primary.is_none() {
            debug!(
                track_id,
                "Primary mirror manifest/status lookup failed, will attempt fallback"
            );
        }

        // 2. Connect the audio stream.
        let stream_start = std::time::Instant::now();
        let stream_progress: Option<ProgressCallback> = on_progress
            .cloned()
            .map(|cb| Arc::new(move |status: &str| cb(status, None, None)) as Arc<_>);
        let mut stream = deps
            .connect_stream(track_id, primary, signal.cloned(), stream_progress)
            .await?;

        debug!(
            track_id,
            source = %stream.source_name,
            codec = %stream.codec,
            bit_depth = stream.bit_depth,
            sample_rate = stream.sample_rate,
            "Stream audio specs received"
        );

        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let safe_track_id: String = track_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        // Keep the human-readable track id for diagnostics, but always add a
        // monotonic/time component. Track ids are external input and must not
        // be allowed to select a path or collide within one job.
        let temp_raw_path = track_dir.join(format!(
            "stream_{safe_track_id}_{unix_ms}_{}.raw",
            unique_temp_suffix()
        ));
        let final_path = track_dir.join(tagger::build_track_filename(&meta));

        // Detached-into-the-loop prefetch: lyrics + artwork download while
        // the audio streams (polled in the same select! as the stream so
        // failures never wait on them).
        let lyrics_fut = std::pin::pin!(lyrics_task);
        let artwork_fut = std::pin::pin!(artwork_task);
        let mut lyrics_result: Option<Option<String>> = None;
        let mut artwork_result: Option<Option<Vec<u8>>> = None;
        let mut lyrics_fut = lyrics_fut.fuse();
        let mut artwork_fut = artwork_fut.fuse();

        const CHUNK_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(45);

        let result: Result<TrackRipResult, RipError> = async {
            let total = stream.content_length.filter(|len| *len > 0);
            if total.is_some_and(|length| length > MAX_AUDIO_BYTES) {
                return Err(RipError::Message(format!(
                    "Audio stream exceeds the {} MiB limit",
                    MAX_AUDIO_BYTES / (1024 * 1024)
                )));
            }
            let mut downloaded_bytes = 0u64;
            // TS starts at 0 → the first chunk always emits a progress event.
            let mut last_progress_update = std::time::Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now);
            let mut file = tokio::fs::File::create(&temp_raw_path).await?;

            loop {
                if signal.is_some_and(|t| t.is_cancelled()) {
                    return Err(cancelled());
                }

                let chunk: bytes::Bytes = tokio::select! {
                    lyrics = &mut lyrics_fut => {
                        lyrics_result = Some(lyrics);
                        continue;
                    }
                    artwork = &mut artwork_fut => {
                        artwork_result = Some(artwork);
                        continue;
                    }
                    _ = tokio::time::sleep(CHUNK_INACTIVITY_TIMEOUT) => {
                        return Err(RipError::Message(format!(
                            "Audio stream stalled on {}: no data received for {}s",
                            stream.source_name,
                            CHUNK_INACTIVITY_TIMEOUT.as_secs()
                        )));
                    }
                    chunk = stream.stream.next() => match chunk {
                        Some(Ok(bytes)) => bytes,
                        Some(Err(error)) => {
                            return Err(RipError::Message(error.to_string()));
                        }
                        None => break,
                    },
                };

                if downloaded_bytes.saturating_add(chunk.len() as u64) > MAX_AUDIO_BYTES {
                    return Err(RipError::Message(format!(
                        "Audio stream exceeds the {} MiB limit",
                        MAX_AUDIO_BYTES / (1024 * 1024)
                    )));
                }

                file.write_all(&chunk).await?;
                downloaded_bytes += chunk.len() as u64;

                if last_progress_update.elapsed() > Duration::from_secs(1) {
                    last_progress_update = std::time::Instant::now();
                    let total_for_bar = total.unwrap_or(0);
                    let progress_str =
                        crate::progress::format_byte_progress(downloaded_bytes, total_for_bar, 12);
                    emit_progress(
                        on_progress,
                        &format!("Downloading lossless audio: {progress_str}"),
                        Some(downloaded_bytes),
                        total,
                    );
                }
            }

            if let Some(expected) = total {
                if downloaded_bytes != expected {
                    return Err(RipError::Message(format!(
                        "Incomplete audio body from {}: expected {expected} bytes, received {downloaded_bytes}",
                        stream.source_name
                    )));
                }
            }

            file.flush().await?;

            if signal.is_some_and(|t| t.is_cancelled()) {
                return Err(cancelled());
            }

            debug!(
                track_id,
                source = %stream.source_name,
                downloaded_bytes,
                stream_duration_ms = stream_start.elapsed().as_millis() as u64,
                "Stream download completed"
            );

            emit_progress(
                on_progress,
                "Tagging and embedding lossless artwork...",
                None,
                None,
            );
            let tag_start = std::time::Instant::now();

            // Reap any prefetches that outlived the stream.
            if lyrics_result.is_none() {
                lyrics_result = Some((&mut lyrics_fut).await);
            }
            if artwork_result.is_none() {
                artwork_result = Some((&mut artwork_fut).await);
            }
            let lyrics = lyrics_result.flatten();
            let cover = artwork_result.flatten();

            if signal.is_some_and(|t| t.is_cancelled()) {
                return Err(cancelled());
            }

            deps.tag_m4a(
                &temp_raw_path,
                &final_path,
                &meta,
                cover.as_deref(),
                lyrics.as_deref(),
            )
            .await?;

            debug!(
                track_id,
                has_lyrics = lyrics.is_some(),
                has_cover = cover.is_some(),
                tag_duration_ms = tag_start.elapsed().as_millis() as u64,
                total_duration_ms = rip_start.elapsed().as_millis() as u64,
                "Tagging finished"
            );

            Ok(TrackRipResult {
                file_path: final_path.to_string_lossy().into_owned(),
                title: meta.title.clone(),
                artist: meta.artist.clone(),
                album: meta.album.clone(),
                duration: meta.duration_secs,
                bit_depth: stream.bit_depth,
                sample_rate: stream.sample_rate,
                codec: stream.codec.clone(),
                genre: meta
                    .genre
                    .clone()
                    .filter(|g| !g.is_empty())
                    .unwrap_or_else(|| "Unknown".to_owned()),
                release_date: meta.release_date.clone(),
                track_number: meta.track_number.unwrap_or(1),
                track_count: meta.track_count.unwrap_or(1),
            })
        }
        .await;

        // TS finally: temp raw removed in all paths; prefetch tasks reaped.
        let _ = tokio::fs::remove_file(&temp_raw_path).await;
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&track_dir).await;
        }
        result
    }
}

fn cancelled() -> RipError {
    RipError::Message("Download was cancelled".to_owned())
}

fn emit_progress(
    on_progress: Option<&RipProgressCallback>,
    status: &str,
    downloaded: Option<u64>,
    total: Option<u64>,
) {
    if let Some(callback) = on_progress {
        callback(status, downloaded, total);
    }
}

/// Jitter fraction in `0.0..1.0` from a cheap time-seeded xorshift (TS uses
/// Math.random; exact values are never asserted).
fn jitter_fraction() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            | 1;
    }
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    STATE.store(state, Ordering::Relaxed);
    (state >> 11) as f64 / (1u64 << 53) as f64
}

fn unique_temp_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Production dependency bundle wiring catalog, mirror policy, stream
/// transport, lyrics, artwork, and native media finalization.
pub struct EngineRipperDeps {
    catalog: Catalog<ReqwestTransport>,
    mirror_policy: MirrorPolicyManager<crate::streaming::ReqwestHttp>,
    stream_transport: StreamTransport<crate::streaming::ReqwestHttp>,
    wrapper_url: Option<String>,
    wrapper_api_key: Option<String>,
    artwork_client: reqwest::Client,
    lyrics_client: reqwest::Client,
    media: media::MediaProcessor,
}

impl EngineRipperDeps {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        catalog: Catalog<ReqwestTransport>,
        mirror_policy: MirrorPolicyManager<crate::streaming::ReqwestHttp>,
        stream_transport: StreamTransport<crate::streaming::ReqwestHttp>,
        wrapper_url: Option<String>,
        wrapper_api_key: Option<String>,
    ) -> Self {
        Self {
            catalog,
            mirror_policy,
            stream_transport,
            wrapper_url,
            wrapper_api_key,
            artwork_client: reqwest::Client::new(),
            lyrics_client: reqwest::Client::new(),
            media: media::MediaProcessor::new(),
        }
    }
}

/// reqwest adapter for the lyrics seam.
struct ReqwestLyricsHttp {
    client: reqwest::Client,
}

impl LyricsHttp for ReqwestLyricsHttp {
    async fn get_json(&self, url: &str) -> Option<String> {
        let response = self
            .client
            .get(url)
            .header("User-Agent", "AlacBot/1.0")
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.text().await.ok()
    }
}

impl RipperDeps for EngineRipperDeps {
    async fn track_meta(&self, track_id: &str, storefront: &str) -> Result<TrackMeta, RipError> {
        self.catalog
            .fetch_track_meta(track_id, storefront)
            .await
            .map_err(RipError::from)
    }

    async fn mirror_endpoint(&self, signal: Option<&CancellationToken>) -> Option<MirrorEndpoint> {
        self.mirror_policy
            .get_endpoint(false, signal.cloned())
            .await
            .ok()
    }

    async fn connect_stream(
        &self,
        track_id: &str,
        primary: Option<MirrorEndpoint>,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
    ) -> Result<AudioStreamSource, RipError> {
        self.stream_transport
            .connect_audio_stream(crate::streaming::ConnectStreamOptions {
                track_id: track_id.to_owned(),
                primary_mirror: primary,
                wrapper_url: self.wrapper_url.clone(),
                wrapper_api_key: self.wrapper_api_key.clone(),
                signal,
                on_progress,
                mirror_policy: Some(&self.mirror_policy),
            })
            .await
            .map_err(RipError::from)
    }

    async fn fetch_lyrics(&self, track_id: &str, meta: &LyricsMeta) -> Option<String> {
        let http = ReqwestLyricsHttp {
            client: self.lyrics_client.clone(),
        };
        lyrics::fetch_lyrics(&http, track_id, meta).await
    }

    async fn fetch_artwork(&self, url: &str) -> Option<Vec<u8>> {
        let response = self
            .artwork_client
            .get(url)
            .header("User-Agent", "AlacBot/1.0")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        Some(bytes.to_vec())
    }

    async fn tag_m4a(
        &self,
        raw_path: &Path,
        output_path: &Path,
        meta: &TrackMeta,
        cover: Option<&[u8]>,
        lyrics: Option<&str>,
    ) -> Result<(), RipError> {
        let tags = media::TrackTags {
            title: (!meta.title.is_empty()).then(|| meta.title.clone()),
            title_sort: (!meta.title.is_empty()).then(|| meta.title.clone()),
            artist: (!meta.artist.is_empty()).then(|| meta.artist.clone()),
            artist_sort: (!meta.artist.is_empty()).then(|| meta.artist.clone()),
            album: (!meta.album.is_empty()).then(|| meta.album.clone()),
            album_sort: (!meta.album.is_empty()).then(|| meta.album.clone()),
            album_artist: (!meta.album_artist.is_empty()).then(|| meta.album_artist.clone()),
            album_artist_sort: (!meta.album_artist.is_empty()).then(|| meta.album_artist.clone()),
            release_date: (!meta.release_date.is_empty()).then(|| meta.release_date.clone()),
            genre: meta.genre.clone().filter(|value| !value.is_empty()),
            composer: meta.composer.clone().filter(|value| !value.is_empty()),
            composer_sort: meta.composer.clone().filter(|value| !value.is_empty()),
            track_number: meta
                .track_number
                .filter(|value| *value != 0)
                .and_then(|value| u16::try_from(value).ok()),
            track_count: meta
                .track_count
                .filter(|value| *value != 0)
                .and_then(|value| u16::try_from(value).ok()),
            disc_number: meta
                .disc_number
                .filter(|value| *value != 0)
                .and_then(|value| u16::try_from(value).ok()),
            disc_count: meta
                .disc_count
                .filter(|value| *value != 0)
                .and_then(|value| u16::try_from(value).ok()),
            lyrics: lyrics.map(str::to_owned).filter(|value| !value.is_empty()),
            artwork_jpeg: cover
                .filter(|value| !value.is_empty())
                .map(|value| value.to_vec()),
            isrc: meta.isrc.clone().filter(|value| !value.is_empty()),
            label: meta.record_label.clone().filter(|value| !value.is_empty()),
            copyright: meta.copyright.clone().filter(|value| !value.is_empty()),
            publisher: meta.record_label.clone().filter(|value| !value.is_empty()),
            performer: (!meta.artist.is_empty()).then(|| meta.artist.clone()),
            release_time: (!meta.release_date.is_empty()).then(|| meta.release_date.clone()),
            upc: meta.upc.clone().filter(|value| !value.is_empty()),
            song_id: meta.id.parse::<u64>().ok(),
            album_id: meta
                .album_id
                .as_deref()
                .and_then(|value| value.parse().ok()),
            artist_id: meta
                .artist_id
                .as_deref()
                .and_then(|value| value.parse().ok()),
            explicit: Some(meta.explicit),
            advisory: Some(match meta.content_advisory.as_deref() {
                Some(value) if value.eq_ignore_ascii_case("explicit") => {
                    media::AdvisoryKind::Explicit
                }
                Some(value) if value.eq_ignore_ascii_case("clean") => media::AdvisoryKind::Clean,
                Some(_) => media::AdvisoryKind::Inoffensive,
                None if meta.explicit => media::AdvisoryKind::Explicit,
                None => media::AdvisoryKind::Inoffensive,
            }),
            media_kind: Some(media::MediaKind::Music),
        };
        let cancellation = CancellationToken::new();
        self.media
            .finalize_alac(raw_path, output_path, &tags, &cancellation)
            .await
            .map(|_| ())
            .map_err(|error| RipError::Message(error.to_string()))
    }
}
