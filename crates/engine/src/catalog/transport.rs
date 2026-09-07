//! HTTP transport seam for the catalog module.
//!
//! Two adapters justify this seam: `ReqwestTransport` for production and a
//! fake JSON-serving transport for offline tests. User-agent and timeout are
//! per-call decisions made by the catalog (TS parity: the TS service sets
//! them per fetch), so the transport stays a dumb GET.

use std::{
    future::Future,
    time::{Duration, Instant},
};

/// Failures of a single HTTP GET.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The request threw — timeout OR network error. The TS oracle catches
    /// every `fetch()` throw and lumps both into one "timed out after Xms"
    /// message, so parity keeps them together.
    #[error("fetch failed after {elapsed_ms}ms: {source}")]
    Fetch {
        elapsed_ms: u64,
        source: reqwest::Error,
    },
    #[error("HTTP {status}")]
    Status { status: u16 },
}

/// The seam every catalog fetch crosses.
pub trait Transport: Send + Sync {
    fn get(
        &self,
        url: &str,
        user_agent: &str,
        timeout: Duration,
    ) -> impl Future<Output = Result<String, TransportError>> + Send;
}

/// User-agent for every iTunes endpoint (TS parity).
pub const ITUNES_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/145.0.0.0";
/// Different, simpler UA the charts RSS endpoint gets (TS parity).
pub const CHARTS_USER_AGENT: &str = "Mozilla/5.0";

/// Production adapter over `reqwest`.
#[derive(Clone, Default)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Transport for ReqwestTransport {
    async fn get(
        &self,
        url: &str,
        user_agent: &str,
        timeout: Duration,
    ) -> Result<String, TransportError> {
        let start = Instant::now();
        let resp = self
            .client
            .get(url)
            .header("User-Agent", user_agent)
            .timeout(timeout)
            .send()
            .await
            .map_err(|source| TransportError::Fetch {
                elapsed_ms: start.elapsed().as_millis() as u64,
                source,
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(TransportError::Status {
                status: status.as_u16(),
            });
        }
        resp.text().await.map_err(|source| TransportError::Fetch {
            elapsed_ms: start.elapsed().as_millis() as u64,
            source,
        })
    }
}
