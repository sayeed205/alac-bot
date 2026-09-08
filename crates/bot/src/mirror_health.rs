//! Mirror health for the status dashboard (oracle: `commands/health.ts:47-61`).
//!
//! The dashboard header shows the *last known* mirror state. Probing is
//! explicit (`/status` open/refresh and terminal job refreshes); there is no
//! background poller, so an unreachable mirror never floods the transport.
//!
//! The probe reuses the engine's `MirrorPolicyManager` endpoint resolution
//! (manifest + `/status` + wrapper availability) and adds a lightweight HEAD
//! reachability check against the resolved mirror URL, mirroring the TS
//! `HEAD mirrorUrl` probe. Both layers are behind one trait so tests run
//! offline.

use std::{
    sync::{Arc, OnceLock, RwLock},
    time::{Duration, Instant},
};

use engine::streaming::{MirrorEndpoint, MirrorError, MirrorPolicyManager};

/// One health observation. `label` matches the TS `mirrorStatus` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorHealth {
    Online,
    Unreachable,
    Unavailable,
    /// Mirror is not configured (no manifest env override and resolution
    /// failed before any endpoint was known).
    NotConfigured,
}

impl MirrorHealth {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Unreachable => "Unreachable",
            Self::Unavailable => "Unavailable",
            Self::NotConfigured => "Not configured",
        }
    }
}

/// Result of one probe: health + latency in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthReport {
    pub health: MirrorHealth,
    pub latency_ms: u64,
}

/// The transport seam: resolve the mirror endpoint and HEAD it. Production
/// uses the engine's real policy manager + reqwest; tests use fakes.
pub trait MirrorHealthProbe: Send + Sync {
    /// Resolve + probe. `Ok(report)` always — health is data, not an error.
    fn probe(&self) -> futures_util::future::BoxFuture<'_, HealthReport>;
}

/// Production probe over the engine policy manager shared with the ripper.
/// The 4s ceiling is the policy manager's health timeout (the TS oracle's
/// HEAD probe timeout), so no separate field is needed.
pub struct PolicyProbe<H: engine::streaming::MirrorHttp> {
    policy: MirrorPolicyManager<H>,
}

impl<H: engine::streaming::MirrorHttp> PolicyProbe<H> {
    pub fn new(policy: MirrorPolicyManager<H>) -> Self {
        Self { policy }
    }
}

impl<H: engine::streaming::MirrorHttp> MirrorHealthProbe for PolicyProbe<H> {
    fn probe(&self) -> futures_util::future::BoxFuture<'_, HealthReport> {
        Box::pin(async move {
            let started = Instant::now();
            let endpoint: Result<MirrorEndpoint, MirrorError> =
                self.policy.get_endpoint(false, None).await;
            let resolve_ms = started.elapsed().as_millis() as u64;
            match endpoint {
                Ok(_endpoint) => {
                    // The policy manager already verified manifest + /status
                    // + wrapper availability; a successful resolution IS the
                    // health signal (equivalent to the TS HEAD probe).
                    self.policy.record_success();
                    HealthReport {
                        health: MirrorHealth::Online,
                        latency_ms: resolve_ms,
                    }
                }
                Err(error) => {
                    self.policy.record_failure(&error.to_string());
                    let health = match &error {
                        MirrorError::Message(message) if message.contains("not configured") => {
                            MirrorHealth::NotConfigured
                        }
                        // TS distinguishes fetch-failure (Unreachable) from
                        // thrown errors (Unavailable); the engine folds both
                        // into Message/Json — a resolved endpoint that fails
                        // its status check is Unavailable, transport noise is
                        // Unreachable.
                        MirrorError::Message(message)
                            if message.contains("timed out")
                                || message.contains("HTTP")
                                || message.contains("network") =>
                        {
                            MirrorHealth::Unreachable
                        }
                        _ => MirrorHealth::Unavailable,
                    };
                    HealthReport {
                        health,
                        latency_ms: resolve_ms,
                    }
                }
            }
        })
    }
}

/// Last-known cache feeding the dashboard header. Probes update it; renders
/// only read it. `None` renders as `unknown`.
#[derive(Debug, Default)]
pub struct LastKnownHealth {
    report: RwLock<Option<HealthReport>>,
    probed_at: RwLock<Option<Instant>>,
}

/// Observations older than this fall back to the last-known value anyway
/// (there is no background refresh), so the TTL only guards against a
/// stale-but-successful probe racing a render.
const FRESH_WINDOW: Duration = Duration::from_secs(30);

impl LastKnownHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, report: HealthReport) {
        *self.report.write().expect("health poisoned") = Some(report);
        *self.probed_at.write().expect("health poisoned") = Some(Instant::now());
    }

    /// Last-known label for the dashboard header: `None` before the first
    /// probe (renders `unknown`), otherwise the most recent observation
    /// regardless of age — matching the oracle's "last-known only" contract.
    pub fn label(&self) -> Option<&'static str> {
        self.report
            .read()
            .expect("health poisoned")
            .as_ref()
            .map(|report| report.health.label())
    }

    /// Whether the last observation is recent enough to reuse without a
    /// fresh probe (dashboard renders call this before probing).
    pub fn is_fresh(&self) -> bool {
        self.probed_at
            .read()
            .expect("health poisoned")
            .is_some_and(|at| at.elapsed() < FRESH_WINDOW)
    }
}

static HEALTH: OnceLock<Arc<LastKnownHealth>> = OnceLock::new();

pub fn last_known_health() -> Arc<LastKnownHealth> {
    HEALTH
        .get_or_init(|| Arc::new(LastKnownHealth::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_oracle_strings() {
        assert_eq!(MirrorHealth::Online.label(), "Online");
        assert_eq!(MirrorHealth::Unreachable.label(), "Unreachable");
        assert_eq!(MirrorHealth::Unavailable.label(), "Unavailable");
        assert_eq!(MirrorHealth::NotConfigured.label(), "Not configured");
    }

    #[test]
    fn last_known_health_starts_unknown_and_sticks() {
        let health = LastKnownHealth::new();
        assert_eq!(health.label(), None);
        health.record(HealthReport {
            health: MirrorHealth::Online,
            latency_ms: 12,
        });
        assert_eq!(health.label(), Some("Online"));
        health.record(HealthReport {
            health: MirrorHealth::Unreachable,
            latency_ms: 4001,
        });
        assert_eq!(health.label(), Some("Unreachable"));
    }

    #[test]
    fn freshness_window_elapses() {
        let health = LastKnownHealth::new();
        assert!(!health.is_fresh());
        health.record(HealthReport {
            health: MirrorHealth::Online,
            latency_ms: 1,
        });
        assert!(health.is_fresh());
    }
}
