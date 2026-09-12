use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use apple::{
    AppleAcquisitionConfig, AppleStreamAcquisition, MirrorHttp, MirrorHttpError,
    MirrorPolicyManager, MANIFEST_URL,
};
use bytes::Bytes;
use engine::streaming::{
    ByteStream, ProgressCallback, StreamHttp, StreamHttpError, StreamHttpResponse, StreamTransport,
};
use futures_util::stream;
use tokio_util::sync::CancellationToken;

struct FakeMirror {
    available: bool,
}

impl MirrorHttp for FakeMirror {
    async fn get(
        &self,
        url: &str,
        headers: &[(&str, String)],
        timeout: Duration,
        signal: Option<&CancellationToken>,
    ) -> Result<String, MirrorHttpError> {
        let _ = (headers, timeout, signal);
        if !self.available {
            return Err(MirrorHttpError::Status(503));
        }
        if url == MANIFEST_URL {
            Ok(r#"{"source":{"apple":"https://mirror"},"key":"key"}"#.to_owned())
        } else {
            Ok(r#"{"wrapper_instances":[1]}"#.to_owned())
        }
    }
}

struct FakeStream {
    calls: Arc<Mutex<Vec<String>>>,
    failures: Mutex<Vec<String>>,
    cancel_on_fetch: bool,
}

impl FakeStream {
    fn new(failures: &[&str], calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            calls,
            failures: Mutex::new(failures.iter().map(|url| (*url).to_owned()).collect()),
            cancel_on_fetch: false,
        }
    }
}

impl StreamHttp for FakeStream {
    async fn fetch(
        &self,
        url: &str,
        api_key: Option<&str>,
        timeout: Duration,
        signal: Option<&CancellationToken>,
    ) -> Result<StreamHttpResponse, StreamHttpError> {
        let _ = (api_key, timeout, signal);
        self.calls.lock().unwrap().push(url.to_owned());
        if self.cancel_on_fetch {
            if let Some(token) = signal {
                token.cancel();
            }
            return Err(StreamHttpError::Cancelled);
        }
        let should_fail = self
            .failures
            .lock()
            .unwrap()
            .iter()
            .position(|failure| url.contains(failure));
        if let Some(index) = should_fail {
            self.failures.lock().unwrap().remove(index);
            return Err(StreamHttpError::Network("transient".to_owned()));
        }
        let body: ByteStream = Box::pin(stream::once(
            async move { Ok(Bytes::from_static(b"audio")) },
        ));
        Ok(StreamHttpResponse {
            status: 200,
            codec: Some("alac".to_owned()),
            bit_depth: Some("24".to_owned()),
            sample_rate: Some("96000".to_owned()),
            content_length: Some(5),
            body: Some(body),
        })
    }
}

fn progress_log() -> (ProgressCallback, Arc<Mutex<Vec<String>>>) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&messages);
    let callback: ProgressCallback = Arc::new(move |message| {
        captured.lock().unwrap().push(message.to_owned());
    });
    (callback, messages)
}

fn acquisition(
    failures: &[&str],
    mirror: FakeMirror,
    wrapper_url: Option<&str>,
    rounds: u32,
) -> (
    AppleStreamAcquisition<FakeStream, FakeMirror>,
    Arc<Mutex<Vec<String>>>,
) {
    acquisition_with_retry_config(failures, mirror, wrapper_url, rounds, 0)
}

fn acquisition_with_retry_config(
    failures: &[&str],
    mirror: FakeMirror,
    wrapper_url: Option<&str>,
    rounds: u32,
    retry_base_delay_ms: u64,
) -> (
    AppleStreamAcquisition<FakeStream, FakeMirror>,
    Arc<Mutex<Vec<String>>>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let policy = MirrorPolicyManager::new(mirror, None);
    let stream = FakeStream::new(failures, Arc::clone(&calls));
    (
        AppleStreamAcquisition::with_config(
            StreamTransport::new(stream),
            policy,
            wrapper_url.map(str::to_owned),
            None,
            AppleAcquisitionConfig {
                retry_rounds: rounds,
                retry_base_delay_ms,
            },
        ),
        calls,
    )
}

#[tokio::test]
async fn transient_mirror_failure_reuses_resolved_endpoint_on_retry_round() {
    let (acquisition, calls) = acquisition(
        &["mirror/api/stream/42"],
        FakeMirror { available: true },
        None,
        2,
    );
    let source = acquisition
        .connect_stream("42", None, None, apple::CodecPreference::HighestQuality)
        .await
        .unwrap();
    assert_eq!(source.source_name, "primary mirror (mirror)");
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn wrapper_candidates_are_tried_in_order_after_mirror_failure() {
    let (acquisition, calls) = acquisition(
        &["/api/stream/42"],
        FakeMirror { available: false },
        Some("https://wrapper///"),
        1,
    );
    acquisition
        .connect_stream("42", None, None, apple::CodecPreference::HighestQuality)
        .await
        .unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "https://wrapper/api/stream/42".to_owned(),
            "https://wrapper/stream/42".to_owned(),
        ]
    );
}

#[tokio::test]
async fn wrapper_failures_are_aggregated_in_candidate_order() {
    let (acquisition, _) = acquisition(
        &["/api/stream/42", "/stream/42"],
        FakeMirror { available: false },
        Some("https://wrapper"),
        1,
    );
    let error = acquisition
        .connect_stream("42", None, None, apple::CodecPreference::HighestQuality)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to stream audio from all sources. All streaming endpoints failed for track 42. Errors: Wrapper candidate (https://wrapper/api/stream/42) failed: transient; Wrapper candidate (https://wrapper/stream/42) failed: transient"
    );
}

#[tokio::test]
async fn cancellation_does_not_retry_or_open_primary_circuit() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let policy = MirrorPolicyManager::new(FakeMirror { available: true }, None);
    let stream = FakeStream {
        calls: Arc::clone(&calls),
        failures: Mutex::new(Vec::new()),
        cancel_on_fetch: true,
    };
    let acquisition = AppleStreamAcquisition::with_config(
        StreamTransport::new(stream),
        policy,
        None,
        None,
        AppleAcquisitionConfig {
            retry_rounds: 4,
            retry_base_delay_ms: 0,
        },
    );
    let signal = CancellationToken::new();
    let error = acquisition
        .connect_stream(
            "42",
            Some(signal),
            None,
            apple::CodecPreference::HighestQuality,
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("Download was cancelled"));
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "the canceled primary is not retried"
    );
    assert!(
        !acquisition.mirror_policy().is_circuit_open(),
        "cancellation is not a mirror failure"
    );
}

#[tokio::test]
async fn primary_failure_opens_circuit_and_primary_success_clears_it() {
    let (failed_acquisition, _) = acquisition(
        &["mirror/api/stream/42"],
        FakeMirror { available: true },
        None,
        1,
    );
    failed_acquisition
        .connect_stream("42", None, None, apple::CodecPreference::HighestQuality)
        .await
        .unwrap_err();
    assert!(failed_acquisition.mirror_policy().is_circuit_open());

    let calls = Arc::new(Mutex::new(Vec::new()));
    let policy = MirrorPolicyManager::new(
        FakeMirror { available: true },
        Some(("https://mirror/".to_owned(), "key".to_owned())),
    );
    policy.record_failure("previous primary failure");
    let successful_acquisition = AppleStreamAcquisition::with_config(
        StreamTransport::new(FakeStream::new(&[], Arc::clone(&calls))),
        policy,
        None,
        None,
        AppleAcquisitionConfig {
            retry_rounds: 1,
            retry_base_delay_ms: 0,
        },
    );
    successful_acquisition
        .connect_stream("42", None, None, apple::CodecPreference::HighestQuality)
        .await
        .unwrap();
    assert!(!successful_acquisition.mirror_policy().is_circuit_open());
}

#[tokio::test]
async fn fallback_reports_progress_when_primary_is_unavailable() {
    let (acquisition, _) = acquisition(
        &[],
        FakeMirror { available: false },
        Some("https://wrapper"),
        1,
    );
    let (callback, messages) = progress_log();
    acquisition
        .connect_stream(
            "42",
            None,
            Some(callback),
            apple::CodecPreference::HighestQuality,
        )
        .await
        .unwrap();

    assert_eq!(
        *messages.lock().unwrap(),
        vec!["Primary mirror unavailable. Connecting to fallback wrapper..."]
    );
}

#[tokio::test]
async fn injected_retry_config_controls_exponential_backoff_and_rounds() {
    tokio::time::pause();
    let (acquisition, calls) = acquisition_with_retry_config(
        &["mirror/api/stream/42", "mirror/api/stream/42"],
        FakeMirror { available: true },
        None,
        3,
        100,
    );
    let (callback, messages) = progress_log();
    acquisition
        .connect_stream(
            "42",
            None,
            Some(callback),
            apple::CodecPreference::HighestQuality,
        )
        .await
        .unwrap();

    assert_eq!(calls.lock().unwrap().len(), 3);
    assert_eq!(
        *messages.lock().unwrap(),
        vec![
            "All sources failed; retrying (round 1/3) in 0.1s...",
            "All sources failed; retrying (round 2/3) in 0.2s...",
        ]
    );
}
