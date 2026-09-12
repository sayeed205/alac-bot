//! Audio stream endpoint validation and body handling.

use std::{fmt, sync::Arc, time::Duration};

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use super::http::{ByteStream, StreamHttp, StreamHttpError};
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

/// Transport over an injectable streaming adapter.
pub struct StreamTransport<H: StreamHttp> {
    http: H,
}

impl<H: StreamHttp> StreamTransport<H> {
    pub fn new(http: H) -> Self {
        Self { http }
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
