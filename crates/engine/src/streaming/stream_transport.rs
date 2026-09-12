//! Audio stream connection with primary-mirror and wrapper failover.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    http::{ByteStream, StreamHttp, StreamHttpError},
    mirror_policy::{MirrorEndpoint, MirrorPolicy},
};
use crate::{
    limits::{MAX_AUDIO_BYTES, MAX_ERROR_BODY_BYTES},
    wrapper::CodecPreference,
};

pub struct FetchEndpointOptions {
    pub stream_url: String,
    pub api_key: Option<String>,
    pub source_name: String,
    pub signal: Option<CancellationToken>,
    pub timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("{0}")]
    Message(String),
    #[error(
        "incomplete audio body from {source_name}: expected {expected} bytes, received {received}"
    )]
    IncompleteBody {
        source_name: String,
        expected: u64,
        received: u64,
    },
}

impl StreamError {
    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    fn into_message(self) -> String {
        match self {
            Self::Message(message) => message,
            Self::IncompleteBody {
                source_name,
                expected,
                received,
            } => format!(
                "Incomplete audio body from {source_name}: expected {expected} bytes, received {received}"
            ),
        }
    }
}

pub struct AudioStreamSource {
    pub stream: ByteStream,
    pub source_name: String,
    pub codec: String,
    pub bit_depth: u32,
    pub sample_rate: u32,
    /// Response `content-length`; `None` or `Some(0)` mean "unknown" for
    /// progress totals.
    pub content_length: Option<u64>,
}

impl fmt::Debug for AudioStreamSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AudioStreamSource")
            .field("source_name", &self.source_name)
            .field("codec", &self.codec)
            .field("bit_depth", &self.bit_depth)
            .field("sample_rate", &self.sample_rate)
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}

pub type ProgressCallback = Arc<dyn Fn(&str) + Send + Sync>;

pub struct ConnectStreamOptions<'a> {
    pub track_id: String,
    pub primary_mirror: Option<MirrorEndpoint>,
    pub wrapper_url: Option<String>,
    pub wrapper_api_key: Option<String>,
    pub signal: Option<CancellationToken>,
    pub on_progress: Option<ProgressCallback>,
    pub mirror_policy: Option<&'a dyn MirrorPolicy>,
    pub codec_preference: CodecPreference,
}

/// Transport over an injectable streaming adapter.
pub struct StreamTransport<H: StreamHttp> {
    http: H,
    default_timeout: Duration,
}

impl<H: StreamHttp> StreamTransport<H> {
    pub fn new(http: H) -> Self {
        Self::with_timeout(http, Duration::from_secs(15))
    }

    pub fn with_timeout(http: H, default_timeout: Duration) -> Self {
        Self {
            http,
            default_timeout,
        }
    }

    pub fn http(&self) -> &H {
        &self.http
    }

    pub async fn fetch_endpoint(
        &self,
        options: FetchEndpointOptions,
    ) -> Result<AudioStreamSource, StreamError> {
        let FetchEndpointOptions {
            stream_url,
            api_key,
            source_name,
            signal,
            timeout,
        } = options;
        if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
            return Err(StreamError::message("Download was cancelled"));
        }
        let response = self
            .http
            .fetch(&stream_url, api_key.as_deref(), timeout, signal.as_ref())
            .await
            .map_err(|error| self.map_http_error(error, timeout, &source_name))?;

        if !(200..300).contains(&response.status) {
            let text = if let Some(mut body) = response.body {
                collect_body(&mut body).await
            } else {
                String::new()
            };
            return Err(StreamError::message(format!(
                "HTTP {} on {}: {}",
                response.status,
                source_name,
                text.chars().take(120).collect::<String>()
            )));
        }

        let body = response.body.ok_or_else(|| {
            StreamError::message(format!("Empty body returned from {source_name}"))
        })?;
        if response
            .content_length
            .is_some_and(|length| length > MAX_AUDIO_BYTES)
        {
            return Err(StreamError::message(format!(
                "Audio stream exceeds the {} MiB limit",
                MAX_AUDIO_BYTES / (1024 * 1024)
            )));
        }
        // Malformed values parse as documented defaults instead of
        // failing, which only ever happens with misbehaving mirrors.
        let bit_depth = response
            .bit_depth
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(24);
        let sample_rate = response
            .sample_rate
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(96_000);
        Ok(AudioStreamSource {
            stream: body,
            source_name,
            codec: response.codec.unwrap_or_else(|| "alac".to_owned()),
            bit_depth,
            sample_rate,
            content_length: response.content_length,
        })
    }

    pub async fn connect_audio_stream(
        &self,
        options: ConnectStreamOptions<'_>,
    ) -> Result<AudioStreamSource, StreamError> {
        let ConnectStreamOptions {
            track_id,
            primary_mirror,
            wrapper_url,
            wrapper_api_key,
            signal,
            on_progress,
            mirror_policy,
            codec_preference,
        } = options;
        let policy = mirror_policy;

        let rounds = stream_retry_rounds();
        let base_delay = stream_retry_base_delay();
        // Unique failure messages in first-seen order: retry rounds revisit
        // the same sources and must not spam the aggregated error.
        let mut all_errors: Vec<String> = Vec::new();

        for round in 0..rounds {
            if round > 0 {
                if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                    break;
                }
                let delay = base_delay * 2u64.pow(round - 1);
                if let Some(on_progress) = on_progress.as_ref() {
                    on_progress(&format!(
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
            let round_errors = &mut Vec::new();
            if let Some(stream) = self
                .connect_once(
                    &track_id,
                    primary_mirror.clone(),
                    wrapper_url.as_deref(),
                    wrapper_api_key.clone(),
                    signal.clone(),
                    on_progress.clone(),
                    policy,
                    codec_preference,
                    round_errors,
                )
                .await
            {
                return Ok(stream);
            }
            for message in round_errors.drain(..) {
                if !all_errors.contains(&message) {
                    all_errors.push(message);
                }
            }
            // A cancellation surfaced as an error string, not a token
            // state; stop retrying silently.
            if signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                break;
            }

            // Fast exit on non-retryable errors:
            // If the wrapper is configured and reported a 404/unavailable error,
            // or if no wrapper is configured and all mirror sources reported 404,
            // subsequent retry rounds will never succeed.
            let wrapper_configured = wrapper_url
                .as_deref()
                .map(|url| !url.trim().trim_end_matches('/').is_empty())
                .unwrap_or(false);
            let permanent_failure = if wrapper_configured {
                all_errors
                    .iter()
                    .any(|e| (e.contains("wrapper") || e.contains("Wrapper")) && is_non_retryable_error(e))
            } else {
                !all_errors.is_empty() && all_errors.iter().all(|e| is_non_retryable_error(e))
            };
            if permanent_failure {
                break;
            }
        }

        let wrapper_missing = wrapper_url
            .as_deref()
            .map(|url| url.trim().trim_end_matches('/').is_empty())
            .unwrap_or(true);
        if wrapper_missing {
            return Err(StreamError::message(format!(
                "Audio streaming failed and no wrapper URL is configured. Errors: {}",
                all_errors.join("; ")
            )));
        }
        Err(StreamError::message(format!(
            "Failed to stream audio from all sources. All streaming endpoints failed for track {track_id}. Errors: {}",
            all_errors.join("; ")
        )))
    }

    /// One attempt through mirror then wrapper; pushes one message per
    /// failed source into `errors`.
    #[allow(clippy::too_many_arguments)]
    async fn connect_once(
        &self,
        track_id: &str,
        primary_mirror: Option<MirrorEndpoint>,
        wrapper_url: Option<&str>,
        wrapper_api_key: Option<String>,
        signal: Option<CancellationToken>,
        on_progress: Option<ProgressCallback>,
        policy: Option<&dyn MirrorPolicy>,
        codec_preference: CodecPreference,
        errors: &mut Vec<String>,
    ) -> Option<AudioStreamSource> {
        if let Some(primary) = primary_mirror {
            // The mirror only serves ALAC rips; an Atmos request would be a
            // guaranteed dead end there.
            if codec_preference == CodecPreference::Atmos {
                errors.push("Skipping primary mirror: Atmos requested".to_owned());
            } else {
                let mirror_url = primary.mirror_url.trim_end_matches('/').to_owned();
                let source_name = format!("primary mirror ({})", hostname(&primary.mirror_url));
                let result = self
                    .fetch_endpoint(FetchEndpointOptions {
                        stream_url: format!("{mirror_url}/api/stream/{track_id}"),
                        api_key: Some(primary.api_key),
                        source_name,
                        signal: signal.clone(),
                        timeout: self.default_timeout,
                    })
                    .await;
                match result {
                    Ok(stream) => {
                        if let Some(policy) = policy {
                            policy.record_success();
                        }
                        return Some(stream);
                    }
                    Err(error) => {
                        let message = error.into_message();
                        errors.push(format!("Primary mirror failed: {message}"));
                        if !signal.as_ref().is_some_and(CancellationToken::is_cancelled) {
                            if let Some(policy) = policy {
                                policy.record_failure(&message);
                            }
                        }
                    }
                }
            }
        }

        let clean_wrapper = wrapper_url
            .map(str::trim)
            .map(|url| url.trim_end_matches('/'))
            .filter(|url| !url.is_empty());
        let Some(clean_wrapper) = clean_wrapper else {
            // The caller owns the no-wrapper message; there is nothing left
            // to try this round.
            return None;
        };
        if let Some(on_progress) = on_progress.as_ref() {
            on_progress("Primary mirror unavailable. Connecting to fallback wrapper...");
        }

        // A wrapper-lite instance (default port 12340 or "lite" in the URL)
        // is served by the native Rust engine with Temari FairPlay
        // decryption.
        let is_wrapper_lite = clean_wrapper.contains("12340")
            || clean_wrapper.ends_with("/lite")
            || clean_wrapper.contains("wrapper-lite");
        if is_wrapper_lite {
            let wrapper_engine =
                crate::wrapper::WrapperEngine::new(clean_wrapper, wrapper_api_key.as_deref());
            match wrapper_engine
                .rip_track(
                    track_id,
                    signal.clone(),
                    on_progress.clone(),
                    codec_preference,
                )
                .await
            {
                Ok(source) => return Some(source),
                Err(err) => {
                    let msg = err.into_message();
                    errors.push(format!("Native wrapper engine failed: {msg}"));
                }
            }
        } else {
            // Standard HTTP stream proxies: try the known candidate paths.
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
                    .fetch_endpoint(FetchEndpointOptions {
                        stream_url: endpoint.clone(),
                        api_key: wrapper_api_key.clone(),
                        source_name: format!("wrapper ({clean_wrapper})"),
                        signal: signal.clone(),
                        timeout: self.default_timeout,
                    })
                    .await
                {
                    Ok(stream) => return Some(stream),
                    Err(error) => errors.push(format!(
                        "Wrapper candidate ({endpoint}) failed: {}",
                        error.into_message()
                    )),
                }
            }
        }
        None
    }

    fn map_http_error(
        &self,
        error: StreamHttpError,
        timeout: Duration,
        source_name: &str,
    ) -> StreamError {
        match error {
            StreamHttpError::Timeout { .. } => StreamError::message(format!(
                "Stream handshake timed out after {}s on {source_name}",
                timeout.as_millis() / 1000
            )),
            StreamHttpError::Cancelled => StreamError::message("Download was cancelled"),
            StreamHttpError::Network(message) => StreamError::message(message),
        }
    }
}

async fn collect_body(body: &mut ByteStream) -> String {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        if let Ok(chunk) = chunk {
            let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if bytes.len() >= MAX_ERROR_BODY_BYTES {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
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

/// Retry rounds for one stream connection (mirror + wrapper per round).
/// `1` = no retries, today's behavior. Default 3.
fn stream_retry_rounds() -> u32 {
    std::env::var("ALAC_STREAM_RETRIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|rounds| *rounds > 0)
        .unwrap_or(3)
}

/// Base backoff between stream retry rounds; doubles per round, capped at
/// 30s. Default 2s (matching the upload-retry base).
fn stream_retry_base_delay() -> u64 {
    std::env::var("ALAC_STREAM_RETRY_BASE_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|delay| *delay > 0)
        .unwrap_or(2000)
        .min(30_000)
}

/// Returns true if an error message indicates the track is permanently
/// unavailable / 404 and should not be retried.
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
