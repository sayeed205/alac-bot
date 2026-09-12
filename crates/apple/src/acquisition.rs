//! Apple Music audio acquisition and source failover.
//!
//! This is the provider boundary used by the generic engine ripper. It owns
//! the Apple mirror policy, wrapper selection, retry ordering, and the native
//! wrapper path; the engine only sees one `connect_stream` operation.

use std::time::Duration;

use engine::streaming::{
    AudioStreamSource, FetchEndpointOptions, ProgressCallback, StreamError, StreamHttp,
    StreamTransport,
};
use lyrics::{LyricsFuture, LyricsHttp, LyricsLookup, LyricsRegistry};
use music::CodecPreference;
use tokio_util::sync::CancellationToken;

use crate::{
    mirror_http::MirrorHttp,
    mirror_policy::{MirrorEndpoint, MirrorPolicyManager},
    wrapper::WrapperEngine,
    ReqwestMirrorHttp,
};

pub struct AppleStreamAcquisition<S: StreamHttp, M: MirrorHttp> {
    stream_transport: StreamTransport<S>,
    mirror_policy: MirrorPolicyManager<M>,
    wrapper_url: Option<String>,
    wrapper_api_key: Option<String>,
    retry_config: AppleAcquisitionConfig,
}

/// Retry settings for one Apple stream acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppleAcquisitionConfig {
    /// Total mirror/wrapper rounds, including the initial round.
    pub retry_rounds: u32,
    /// Delay before the second round; later delays double up to 30 seconds.
    pub retry_base_delay_ms: u64,
}

impl Default for AppleAcquisitionConfig {
    fn default() -> Self {
        Self {
            retry_rounds: 3,
            retry_base_delay_ms: 2_000,
        }
    }
}

impl AppleAcquisitionConfig {
    fn from_environment() -> Self {
        Self {
            retry_rounds: std::env::var("ALAC_STREAM_RETRIES")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|rounds| *rounds > 0)
                .unwrap_or(3),
            retry_base_delay_ms: std::env::var("ALAC_STREAM_RETRY_BASE_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|delay| *delay > 0)
                .unwrap_or(2_000)
                .min(30_000),
        }
    }
}

impl<S: StreamHttp, M: MirrorHttp> AppleStreamAcquisition<S, M> {
    pub fn new(
        stream_transport: StreamTransport<S>,
        mirror_policy: MirrorPolicyManager<M>,
        wrapper_url: Option<String>,
        wrapper_api_key: Option<String>,
    ) -> Self {
        Self::with_config(
            stream_transport,
            mirror_policy,
            wrapper_url,
            wrapper_api_key,
            AppleAcquisitionConfig::default(),
        )
    }

    pub fn with_config(
        stream_transport: StreamTransport<S>,
        mirror_policy: MirrorPolicyManager<M>,
        wrapper_url: Option<String>,
        wrapper_api_key: Option<String>,
        retry_config: AppleAcquisitionConfig,
    ) -> Self {
        Self {
            stream_transport,
            mirror_policy,
            wrapper_url,
            wrapper_api_key,
            retry_config: AppleAcquisitionConfig {
                retry_rounds: retry_config.retry_rounds.max(1),
                retry_base_delay_ms: retry_config.retry_base_delay_ms.min(30_000),
            },
        }
    }

    pub fn mirror_policy(&self) -> &MirrorPolicyManager<M> {
        &self.mirror_policy
    }

    pub async fn connect_stream(
        &self,
        track_id: &str,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
        codec_preference: CodecPreference,
    ) -> Result<AudioStreamSource, StreamError> {
        let rounds = self.retry_config.retry_rounds;
        let base_delay = self.retry_config.retry_base_delay_ms;
        let mut all_errors = Vec::new();
        // Resolve at most once for this acquisition. A failed stream is a
        // transient request failure, not a reason to re-run endpoint health
        // discovery and let its circuit cooldown hide the same endpoint from
        // the next retry round.
        let primary = self
            .mirror_policy
            .get_endpoint(false, signal.clone())
            .await
            .ok();

        for round in 0..rounds {
            if round > 0 {
                if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    break;
                }
                let delay = base_delay * 2u64.pow(round - 1);
                if let Some(callback) = on_progress.as_ref() {
                    callback(&format!(
                        "All sources failed; retrying (round {round}/{rounds}) in {:.1}s...",
                        delay as f32 / 1000.0
                    ));
                }
                if let Some(token) = signal.as_ref() {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                        _ = token.cancelled() => break,
                    }
                } else {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
            }

            let mut round_errors = Vec::new();
            if let Some(stream) = self
                .connect_once(
                    track_id,
                    signal.clone(),
                    on_progress.clone(),
                    codec_preference,
                    primary.as_ref(),
                    &mut round_errors,
                )
                .await
            {
                return Ok(stream);
            }
            for message in round_errors {
                if !all_errors.contains(&message) {
                    all_errors.push(message);
                }
            }
            if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                break;
            }

            let wrapper_configured = self
                .wrapper_url
                .as_deref()
                .is_some_and(|url| !url.trim().trim_end_matches('/').is_empty());
            let permanent_failure = if wrapper_configured {
                all_errors.iter().any(|error| {
                    (error.contains("wrapper") || error.contains("Wrapper"))
                        && is_non_retryable_error(error)
                })
            } else {
                !all_errors.is_empty()
                    && all_errors.iter().all(|error| is_non_retryable_error(error))
            };
            if permanent_failure {
                break;
            }
        }

        let wrapper_missing = self
            .wrapper_url
            .as_deref()
            .is_none_or(|url| url.trim().trim_end_matches('/').is_empty());
        if wrapper_missing {
            return Err(StreamError::Message(format!(
                "Audio streaming failed and no wrapper URL is configured. Errors: {}",
                all_errors.join("; ")
            )));
        }
        Err(StreamError::Message(format!(
            "Failed to stream audio from all sources. All streaming endpoints failed for track {track_id}. Errors: {}",
            all_errors.join("; ")
        )))
    }

    async fn connect_once(
        &self,
        track_id: &str,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
        codec_preference: CodecPreference,
        primary: Option<&MirrorEndpoint>,
        errors: &mut Vec<String>,
    ) -> Option<AudioStreamSource> {
        if let Some(primary) = primary {
            if codec_preference == CodecPreference::Atmos {
                errors.push("Skipping primary mirror: Atmos requested".to_owned());
            } else {
                let mirror_url = primary.mirror_url.trim_end_matches('/');
                let result = self
                    .stream_transport
                    .fetch_endpoint(FetchEndpointOptions {
                        stream_url: format!("{mirror_url}/api/stream/{track_id}"),
                        api_key: Some(primary.api_key.clone()),
                        source_name: format!("primary mirror ({})", hostname(&primary.mirror_url)),
                        signal: signal.clone(),
                        timeout: Duration::from_secs(15),
                    })
                    .await;
                match result {
                    Ok(stream) => {
                        self.mirror_policy.record_success();
                        return Some(stream);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        errors.push(format!("Primary mirror failed: {message}"));
                        if !signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                            self.mirror_policy.record_failure(&message);
                        }
                    }
                }
            }
        } else {
            tracing::debug!(
                track_id,
                "Primary mirror manifest/status lookup failed, will attempt fallback"
            );
        }

        let clean_wrapper = self
            .wrapper_url
            .as_deref()
            .map(str::trim)
            .map(|url| url.trim_end_matches('/'))
            .filter(|url| !url.is_empty())?;
        if let Some(callback) = on_progress.as_ref() {
            callback("Primary mirror unavailable. Connecting to fallback wrapper...");
        }

        let is_wrapper_lite = clean_wrapper.contains("12340")
            || clean_wrapper.ends_with("/lite")
            || clean_wrapper.contains("wrapper-lite");
        if is_wrapper_lite {
            let wrapper_engine = WrapperEngine::new(clean_wrapper, self.wrapper_api_key.as_deref());
            match wrapper_engine
                .rip_track(track_id, signal, on_progress, codec_preference)
                .await
            {
                Ok(source) => Some(source),
                Err(error) => {
                    errors.push(format!("Native wrapper engine failed: {error}"));
                    None
                }
            }
        } else {
            for endpoint in [
                format!("{clean_wrapper}/api/stream/{track_id}"),
                format!("{clean_wrapper}/stream/{track_id}"),
            ] {
                if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    errors.push(format!(
                        "Wrapper candidate ({endpoint}) failed: Download was cancelled"
                    ));
                    continue;
                }
                match self
                    .stream_transport
                    .fetch_endpoint(FetchEndpointOptions {
                        stream_url: endpoint.clone(),
                        api_key: self.wrapper_api_key.clone(),
                        source_name: format!("wrapper ({clean_wrapper})"),
                        signal: signal.clone(),
                        timeout: Duration::from_secs(15),
                    })
                    .await
                {
                    Ok(stream) => return Some(stream),
                    Err(error) => {
                        errors.push(format!("Wrapper candidate ({endpoint}) failed: {error}"))
                    }
                }
            }
            None
        }
    }
}

fn hostname(url: &str) -> String {
    let without_scheme = url
        .split_once("://")
        .map_or(url, |(_, remainder)| remainder);
    let authority = without_scheme
        .find(['/', ':', '?'])
        .map_or(without_scheme, |index| &without_scheme[..index]);
    authority.to_owned()
}

pub fn is_non_retryable_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("code 404")
        || lower.contains("code: 404")
        || lower.contains("http 404")
        || lower.contains("status 404")
        || lower.contains("status: 404")
        || lower.contains("404 not found")
        || lower.contains("failed to get m3u8")
        || lower.contains("song is currently unavailable")
        || lower.contains("track is currently unavailable")
        || lower.contains("track not found in itunes")
        || lower.contains("not available in your region")
        || lower.contains("not available in this country")
        || lower.contains("not available in the current storefront")
}

/// Production dependencies for the generic engine ripper.
pub struct AppleRipperDeps {
    catalog: crate::catalog::Catalog<crate::catalog::ReqwestTransport>,
    acquisition: AppleStreamAcquisition<engine::streaming::ReqwestHttp, ReqwestMirrorHttp>,
    artwork_client: reqwest::Client,
    lyrics_client: reqwest::Client,
    lyrics_registry: LyricsRegistry,
    media: media::MediaProcessor,
}

impl AppleRipperDeps {
    pub fn new(
        catalog: crate::catalog::Catalog<crate::catalog::ReqwestTransport>,
        acquisition: AppleStreamAcquisition<engine::streaming::ReqwestHttp, ReqwestMirrorHttp>,
    ) -> Self {
        Self {
            catalog,
            acquisition,
            artwork_client: reqwest::Client::new(),
            lyrics_client: reqwest::Client::new(),
            lyrics_registry: LyricsRegistry::default(),
            media: media::MediaProcessor::new(),
        }
    }

    pub async fn fetch_artwork_bytes(&self, url: &str) -> Option<Vec<u8>> {
        engine::ripper::RipperDeps::fetch_artwork(self, url).await
    }

    pub fn mirror_policy(&self) -> &MirrorPolicyManager<ReqwestMirrorHttp> {
        self.acquisition.mirror_policy()
    }

    pub fn with_lyrics_registry(mut self, registry: LyricsRegistry) -> Self {
        self.lyrics_registry = registry;
        self
    }

    pub fn catalog(&self) -> &crate::catalog::Catalog<crate::catalog::ReqwestTransport> {
        &self.catalog
    }
}

/// Production settings for the complete Apple provider composition.
#[derive(Debug, Clone)]
pub struct AppleProductionConfig {
    pub wrapper_url: Option<String>,
    pub wrapper_api_key: Option<String>,
    pub mirror_override: Option<(String, String)>,
    pub acquisition: AppleAcquisitionConfig,
}

impl Default for AppleProductionConfig {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl AppleProductionConfig {
    /// Load Apple-specific production settings from the process environment.
    pub fn from_environment() -> Self {
        Self {
            wrapper_url: Some(
                env_option("ALAC_WRAPPER_URL")
                    .unwrap_or_else(|| "http://127.0.0.1:12340".to_owned()),
            ),
            wrapper_api_key: env_option("ALAC_WRAPPER_API_KEY"),
            mirror_override: env_option("ALAC_MIRROR_URL").zip(env_option("ALAC_API_KEY")),
            acquisition: AppleAcquisitionConfig::from_environment(),
        }
    }
}

/// Fully wired Apple catalog, playlist, mirror, and stream acquisition.
///
/// The mirror manager retained by this composition is the owner of the shared
/// policy state. Clones returned by `mirror_policy()` and the acquisition use
/// that same state, so dashboard probes and rips observe one circuit/cache.
pub struct AppleProduction {
    playlist: crate::playlist::PlaylistClient<crate::playlist::ReqwestPlaylistHttp>,
    ripper_deps: AppleRipperDeps,
    mirror_policy: MirrorPolicyManager<ReqwestMirrorHttp>,
}

/// Apple-owned presentation data used by the provider-neutral orchestrator.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApplePresentation;

impl engine::orchestrator::deps::ProviderPresentation for ApplePresentation {
    fn default_job_header(&self) -> &str {
        "Apple Music Lossless Rip"
    }

    fn album_url(&self, album_id: &str, storefront: &str) -> Option<String> {
        (!album_id.is_empty()).then(|| {
            let storefront = if storefront.is_empty() {
                "us"
            } else {
                storefront
            };
            format!("https://music.apple.com/{storefront}/album/{album_id}")
        })
    }

    fn unavailable_track_message(&self) -> &str {
        "Unavailable on Apple Music (not streamable)"
    }

    fn unavailable_track_log_message(&self) -> &str {
        "Track is not streamable in Apple Music catalog, skipping rip"
    }
}

impl AppleProduction {
    pub fn new(config: AppleProductionConfig) -> Self {
        let catalog = crate::catalog::Catalog::new(crate::catalog::ReqwestTransport::new());
        let mirror_policy =
            MirrorPolicyManager::new(ReqwestMirrorHttp::new(), config.mirror_override);
        let stream_transport =
            engine::streaming::StreamTransport::new(engine::streaming::ReqwestHttp::new());
        let acquisition = AppleStreamAcquisition::with_config(
            stream_transport,
            mirror_policy.shared(),
            config.wrapper_url,
            config.wrapper_api_key,
            config.acquisition,
        );
        let ripper_deps = AppleRipperDeps::new(catalog, acquisition);
        Self {
            playlist: crate::playlist::PlaylistClient::new(
                crate::playlist::ReqwestPlaylistHttp::new(),
            ),
            ripper_deps,
            mirror_policy,
        }
    }

    pub fn with_lyrics_registry(mut self, registry: LyricsRegistry) -> Self {
        self.ripper_deps = self.ripper_deps.with_lyrics_registry(registry);
        self
    }

    pub fn catalog(&self) -> &crate::catalog::Catalog<crate::catalog::ReqwestTransport> {
        self.ripper_deps.catalog()
    }

    pub fn playlist(
        &self,
    ) -> &crate::playlist::PlaylistClient<crate::playlist::ReqwestPlaylistHttp> {
        &self.playlist
    }

    pub fn ripper_deps(&self) -> &AppleRipperDeps {
        &self.ripper_deps
    }

    pub fn mirror_policy(&self) -> &MirrorPolicyManager<ReqwestMirrorHttp> {
        &self.mirror_policy
    }
}

fn env_option(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

struct ReqwestLyricsHttp {
    client: reqwest::Client,
}

impl LyricsHttp for ReqwestLyricsHttp {
    fn get_json<'a>(&'a self, url: &'a str) -> LyricsFuture<'a, Option<String>> {
        Box::pin(async move {
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
        })
    }
}

impl engine::ripper::RipperDeps for AppleRipperDeps {
    fn is_non_retryable_error(&self, message: &str) -> bool {
        is_non_retryable_error(message)
    }

    async fn track_meta(
        &self,
        track_id: &str,
        storefront: &str,
    ) -> Result<music::TrackMeta, engine::ripper::RipError> {
        self.catalog
            .fetch_track_meta(track_id, storefront)
            .await
            .map_err(|error| engine::ripper::RipError::Message(error.to_string()))
    }

    async fn connect_stream(
        &self,
        track_id: &str,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
        codec_preference: CodecPreference,
    ) -> Result<AudioStreamSource, engine::ripper::RipError> {
        self.acquisition
            .connect_stream(track_id, signal, on_progress, codec_preference)
            .await
            .map_err(engine::ripper::RipError::from)
    }

    async fn fetch_lyrics(&self, lookup: &LyricsLookup) -> Option<String> {
        let http = ReqwestLyricsHttp {
            client: self.lyrics_client.clone(),
        };
        lyrics::lookup(&http, &self.lyrics_registry, lookup).await
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
        Some(response.bytes().await.ok()?.to_vec())
    }

    async fn tag_m4a(
        &self,
        raw_path: &std::path::Path,
        output_path: &std::path::Path,
        meta: &music::TrackMeta,
        cover: Option<&[u8]>,
        lyrics: Option<&str>,
    ) -> Result<(), engine::ripper::RipError> {
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
            artwork_jpeg: cover.filter(|value| !value.is_empty()).map(<[u8]>::to_vec),
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
            compilation: Some(
                meta.album_artist.eq_ignore_ascii_case("Various Artists")
                    || meta.artist.eq_ignore_ascii_case("Various Artists"),
            ),
            gapless: Some(true),
            genre_id: None,
            storefront_id: None,
            encoder: Some("alac-bot".to_owned()),
            comment: None,
            description: None,
        };
        let cancellation = CancellationToken::new();
        self.media
            .finalize_m4a(raw_path, output_path, &tags, &cancellation)
            .await
            .map(|_| ())
            .map_err(|error| engine::ripper::RipError::Message(error.to_string()))
    }
}
