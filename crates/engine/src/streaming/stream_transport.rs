//! Audio stream connection with primary-mirror and wrapper failover.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::{
    http::{ByteStream, StreamHttp, StreamHttpError},
    mirror_policy::{MirrorEndpoint, MirrorPolicy},
};
use crate::limits::{MAX_AUDIO_BYTES, MAX_ERROR_BODY_BYTES};

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
    /// progress totals (TS: `Number(header || 0)`, `> 0 ? : 0`).
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
        // TS parseInt produces NaN for malformed values. Rust uses the
        // documented defaults instead, a harmless deviation for bad mirrors.
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
        } = options;
        let errors = &mut Vec::new();
        let policy = mirror_policy;

        if let Some(primary) = primary_mirror {
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
                    return Ok(stream);
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

        let clean_wrapper = wrapper_url
            .as_deref()
            .map(str::trim)
            .map(|url| url.trim_end_matches('/'))
            .filter(|url| !url.is_empty());
        let Some(clean_wrapper) = clean_wrapper else {
            return Err(StreamError::message(format!(
                "Audio streaming failed and no wrapper URL is configured. Errors: {}",
                errors.join("; ")
            )));
        };
        if let Some(on_progress) = on_progress {
            on_progress("Primary mirror unavailable. Connecting to fallback wrapper...");
        }

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
                Ok(stream) => return Ok(stream),
                Err(error) => errors.push(format!(
                    "Wrapper candidate ({endpoint}) failed: {}",
                    error.into_message()
                )),
            }
        }

        Err(StreamError::message(format!(
            "Failed to stream audio from all sources. All streaming endpoints failed for track {track_id}. Errors: {}",
            errors.join("; ")
        )))
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
