use std::sync::{Arc, Mutex};

use bytes::Bytes;
use engine::streaming::{
    ConnectStreamOptions, MirrorEndpoint, MirrorPolicy, StreamHttp, StreamHttpError,
    StreamHttpResponse, StreamTransport,
};
use futures_util::stream;
use tokio_util::sync::CancellationToken;

struct Route {
    status: u16,
    body: Option<String>,
    codec: Option<String>,
    bit_depth: Option<String>,
    sample_rate: Option<String>,
    content_length: Option<u64>,
    error: Option<StreamHttpError>,
    /// Fail the first N calls to this route (per URL occurrence), then
    /// succeed — used to exercise stream retry rounds.
    fail_times: usize,
}

struct FakeHttp {
    routes: Mutex<Vec<(String, Route)>>,
    calls: Mutex<Vec<String>>,
    /// How many times each URL has been fetched, keyed by the fetched URL.
    url_counts: Mutex<std::collections::HashMap<String, usize>>,
}

impl FakeHttp {
    fn new() -> Self {
        Self {
            routes: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            url_counts: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn route(&self, needle: &str, route: Route) {
        self.routes.lock().unwrap().push((needle.into(), route));
    }
}

impl StreamHttp for FakeHttp {
    async fn fetch(
        &self,
        url: &str,
        api_key: Option<&str>,
        timeout: std::time::Duration,
        signal: Option<&CancellationToken>,
    ) -> Result<StreamHttpResponse, StreamHttpError> {
        let _ = (api_key, timeout, signal);
        self.calls.lock().unwrap().push(url.into());
        let seen = self
            .url_counts
            .lock()
            .unwrap()
            .get(url)
            .copied()
            .unwrap_or(0);
        self.url_counts
            .lock()
            .unwrap()
            .insert(url.to_owned(), seen + 1);
        let routes = self.routes.lock().unwrap();
        let route = routes
            .iter()
            .find(|(needle, _)| url.contains(needle))
            .map(|(_, route)| route)
            .ok_or_else(|| StreamHttpError::Network("missing route".into()))?;
        if route.fail_times > seen {
            return Err(StreamHttpError::Network("transient".into()));
        }
        if let Some(error) = &route.error {
            return Err(error.clone());
        }
        let body = route.body.as_ref().map(|body| {
            Box::pin(stream::iter(vec![Ok(Bytes::from(body.clone()))]))
                as engine::streaming::ByteStream
        });
        Ok(StreamHttpResponse {
            status: route.status,
            codec: route.codec.clone(),
            bit_depth: route.bit_depth.clone(),
            sample_rate: route.sample_rate.clone(),
            content_length: route.content_length,
            body,
        })
    }
}

fn ok() -> Route {
    Route {
        status: 200,
        body: Some("audio".into()),
        codec: None,
        bit_depth: None,
        sample_rate: None,
        content_length: None,
        error: None,
        fail_times: 0,
    }
}

#[derive(Default)]
struct FakePolicy {
    successes: Mutex<usize>,
    failures: Mutex<Vec<String>>,
}

impl MirrorPolicy for FakePolicy {
    fn record_failure(&self, error: &str) {
        self.failures.lock().unwrap().push(error.into());
    }
    fn record_success(&self) {
        *self.successes.lock().unwrap() += 1;
    }
}

fn primary() -> MirrorEndpoint {
    MirrorEndpoint {
        mirror_url: "https://primary.example:123///".into(),
        api_key: "pkey".into(),
    }
}

#[tokio::test]
async fn primary_success_uses_exact_url_and_hostname_and_records_success() {
    let http = FakeHttp::new();
    http.route("primary.example:123/api", ok());
    let transport = StreamTransport::new(http);
    let policy = FakePolicy::default();
    let source = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: Some(primary()),
            wrapper_url: None,
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: Some(&policy),
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap();
    assert_eq!(source.source_name, "primary mirror (primary.example)");
    assert_eq!(*policy.successes.lock().unwrap(), 1);
    assert_eq!(
        transport.http().calls.lock().unwrap().as_slice(),
        ["https://primary.example:123/api/stream/42"]
    );
    assert_eq!(source.content_length, None, "absent header → None");
}

#[tokio::test]
async fn primary_success_carries_content_length() {
    let http = FakeHttp::new();
    let mut route = ok();
    route.content_length = Some(29_000_000);
    http.route("primary.example:123/api", route);
    let transport = StreamTransport::new(http);
    let policy = FakePolicy::default();
    let source = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: Some(primary()),
            wrapper_url: None,
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: Some(&policy),
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap();
    assert_eq!(source.content_length, Some(29_000_000));
    assert_eq!(source.sample_rate, 96_000);
    assert_eq!(source.bit_depth, 24);
    assert_eq!(source.codec, "alac");
}

#[tokio::test]
async fn primary_failure_falls_back_to_first_wrapper_and_reports_progress_once() {
    let http = FakeHttp::new();
    http.route(
        "primary.example:123/api",
        Route {
            error: Some(StreamHttpError::Network("primary down".into())),
            ..ok()
        },
    );
    http.route("wrapper/api/stream/42", ok());
    let transport = StreamTransport::new(http);
    let progress = Arc::new(Mutex::new(Vec::<String>::new()));
    let policy = FakePolicy::default();
    transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: Some(primary()),
            wrapper_url: Some(" https://wrapper/// ".into()),
            wrapper_api_key: Some("wkey".into()),
            signal: None,
            on_progress: Some({
                let progress = progress.clone();
                Arc::new(move |message| progress.lock().unwrap().push(message.into()))
            }),
            mirror_policy: Some(&policy),
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap();
    assert_eq!(
        &*progress.lock().unwrap(),
        &["Primary mirror unavailable. Connecting to fallback wrapper..."]
    );
    assert_eq!(
        transport.http().calls.lock().unwrap()[1],
        "https://wrapper/api/stream/42"
    );
}

#[tokio::test]
async fn both_wrapper_candidates_are_aggregated_in_order() {
    let http = FakeHttp::new();
    http.route(
        "wrapper/api",
        Route {
            error: Some(StreamHttpError::Network("one".into())),
            ..ok()
        },
    );
    http.route(
        "wrapper/stream",
        Route {
            error: Some(StreamHttpError::Network("two".into())),
            ..ok()
        },
    );
    let transport = StreamTransport::with_retry_config(
        http,
        engine::streaming::StreamRetryConfig::new(1, 2_000),
    );
    let error = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: None,
            wrapper_url: Some("https://wrapper".into()),
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: None,
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Failed to stream audio from all sources. All streaming endpoints failed for track 42. Errors: Wrapper candidate (https://wrapper/api/stream/42) failed: one; Wrapper candidate (https://wrapper/stream/42) failed: two");
}

#[tokio::test]
async fn primary_failure_without_wrapper_has_exact_message() {
    let http = FakeHttp::new();
    http.route(
        "primary.example",
        Route {
            error: Some(StreamHttpError::Network("no primary".into())),
            ..ok()
        },
    );
    let transport = StreamTransport::with_retry_config(
        http,
        engine::streaming::StreamRetryConfig::new(1, 2_000),
    );
    let error = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "x".into(),
            primary_mirror: Some(primary()),
            wrapper_url: Some("  ".into()),
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: None,
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Audio streaming failed and no wrapper URL is configured. Errors: Primary mirror failed: no primary");
}

#[tokio::test]
async fn cancelled_signal_is_aggregated_as_download_cancelled() {
    let http = FakeHttp::new();
    http.route("wrapper", ok());
    let signal = CancellationToken::new();
    signal.cancel();
    let transport = StreamTransport::new(http);
    let error = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "x".into(),
            primary_mirror: None,
            wrapper_url: Some("https://wrapper".into()),
            wrapper_api_key: None,
            signal: Some(signal),
            on_progress: None,
            mirror_policy: None,
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Download was cancelled"));
}

#[tokio::test]
async fn primary_failure_records_unless_cancelled() {
    let http = FakeHttp::new();
    http.route(
        "primary",
        Route {
            error: Some(StreamHttpError::Network("bad".into())),
            ..ok()
        },
    );
    let policy = FakePolicy::default();
    let transport = StreamTransport::with_retry_config(
        http,
        engine::streaming::StreamRetryConfig::new(1, 2_000),
    );
    let _ = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "x".into(),
            primary_mirror: Some(primary()),
            wrapper_url: None,
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: Some(&policy),
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await;
    assert_eq!(&*policy.failures.lock().unwrap(), &["bad"]);

    let http = FakeHttp::new();
    http.route(
        "primary",
        Route {
            error: Some(StreamHttpError::Cancelled),
            ..ok()
        },
    );
    let signal = CancellationToken::new();
    signal.cancel();
    let policy = FakePolicy::default();
    let transport = StreamTransport::with_retry_config(
        http,
        engine::streaming::StreamRetryConfig::new(1, 2_000),
    );
    let _ = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "x".into(),
            primary_mirror: Some(primary()),
            wrapper_url: None,
            wrapper_api_key: None,
            signal: Some(signal),
            on_progress: None,
            mirror_policy: Some(&policy),
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await;
    assert!(policy.failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn headers_defaults_and_http_body_limit_and_empty_body_match() {
    let http = FakeHttp::new();
    let mut route = ok();
    route.codec = Some("flac".into());
    route.bit_depth = Some("32".into());
    route.sample_rate = Some("44100".into());
    http.route("custom", route);
    let transport = StreamTransport::new(http);
    let source = transport
        .fetch_endpoint(engine::streaming::FetchEndpointOptions {
            stream_url: "https://custom".into(),
            api_key: None,
            source_name: "custom".into(),
            signal: None,
            timeout: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(
        (source.codec, source.bit_depth, source.sample_rate),
        ("flac".into(), 32, 44100)
    );

    let http = FakeHttp::new();
    http.route(
        "bad",
        Route {
            status: 500,
            body: Some("x".repeat(200)),
            ..ok()
        },
    );
    let transport = StreamTransport::new(http);
    let error = transport
        .fetch_endpoint(engine::streaming::FetchEndpointOptions {
            stream_url: "https://bad".into(),
            api_key: None,
            source_name: "bad".into(),
            signal: None,
            timeout: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string().len(), "HTTP 500 on bad: ".len() + 120);

    let http = FakeHttp::new();
    http.route("empty", Route { body: None, ..ok() });
    let transport = StreamTransport::new(http);
    let error = transport
        .fetch_endpoint(engine::streaming::FetchEndpointOptions {
            stream_url: "https://empty".into(),
            api_key: None,
            source_name: "empty".into(),
            signal: None,
            timeout: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Empty body returned from empty");
}

#[tokio::test]
async fn handshake_timeout_uses_integer_seconds() {
    let http = FakeHttp::new();
    http.route(
        "timeout",
        Route {
            error: Some(StreamHttpError::Timeout { elapsed_ms: 1 }),
            ..ok()
        },
    );
    let transport = StreamTransport::with_timeout(http, std::time::Duration::from_millis(1_999));
    let error = transport
        .fetch_endpoint(engine::streaming::FetchEndpointOptions {
            stream_url: "https://timeout".into(),
            api_key: None,
            source_name: "mirror".into(),
            signal: None,
            timeout: std::time::Duration::from_millis(1_999),
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Stream handshake timed out after 1s on mirror"
    );
}

/// A mirror that recovers after one failed round is retried and succeeds
/// on round 2; the retry backoff honors the configured base delay.
#[tokio::test(start_paused = true)]
async fn transient_mirror_failure_is_retried_next_round() {
    let http = FakeHttp::new();
    http.route(
        "primary.example",
        Route {
            fail_times: 1,
            ..ok()
        },
    );
    let transport = StreamTransport::with_retry_config(
        http,
        engine::streaming::StreamRetryConfig::new(2, 2_000),
    );
    let start = tokio::time::Instant::now();
    let source = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: Some(primary()),
            wrapper_url: None,
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: None,
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap();
    assert_eq!(source.source_name, "primary mirror (primary.example)");
    // Round 2 waits one base delay before the second attempt.
    assert_eq!(start.elapsed(), std::time::Duration::from_millis(2_000));
}

/// With retries exhausted the aggregated error lists each failing source
/// once, regardless of how many rounds ran.
#[tokio::test(start_paused = true)]
async fn exhausted_retries_aggregate_each_source_once() {
    let http = FakeHttp::new();
    http.route(
        "wrapper/api",
        Route {
            error: Some(StreamHttpError::Network("one".into())),
            ..ok()
        },
    );
    http.route(
        "wrapper/stream",
        Route {
            error: Some(StreamHttpError::Network("two".into())),
            ..ok()
        },
    );
    let transport =
        StreamTransport::with_retry_config(http, engine::streaming::StreamRetryConfig::new(2, 100));
    let error = transport
        .connect_audio_stream(ConnectStreamOptions {
            track_id: "42".into(),
            primary_mirror: None,
            wrapper_url: Some("https://wrapper".into()),
            wrapper_api_key: None,
            signal: None,
            on_progress: None,
            mirror_policy: None,
            codec_preference: engine::wrapper::CodecPreference::HighestQuality,
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Failed to stream audio from all sources. All streaming endpoints failed for track 42. Errors: Wrapper candidate (https://wrapper/api/stream/42) failed: one; Wrapper candidate (https://wrapper/stream/42) failed: two");
}
