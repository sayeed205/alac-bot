use std::{sync::Mutex, time::Duration};

use bytes::Bytes;
use engine::streaming::{
    ByteStream, FetchEndpointOptions, StreamHttp, StreamHttpError, StreamHttpResponse,
    StreamTransport,
};
use futures_util::stream;
use tokio_util::sync::CancellationToken;

struct FakeHttp {
    response: Mutex<Option<StreamHttpResponse>>,
}

impl StreamHttp for FakeHttp {
    async fn fetch(
        &self,
        url: &str,
        api_key: Option<&str>,
        timeout: Duration,
        signal: Option<&CancellationToken>,
    ) -> Result<StreamHttpResponse, StreamHttpError> {
        let _ = (url, api_key, timeout, signal);
        self.response
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| StreamHttpError::Network("response already consumed".into()))
    }
}

fn response(status: u16, body: Option<&str>) -> StreamHttpResponse {
    StreamHttpResponse {
        status,
        codec: Some("alac".into()),
        bit_depth: Some("24".into()),
        sample_rate: Some("96000".into()),
        content_length: body.map(|value| value.len() as u64),
        body: body.map(|value| {
            let value = value.to_owned();
            Box::pin(stream::once(async move { Ok(Bytes::from(value)) })) as ByteStream
        }),
    }
}

#[tokio::test]
async fn fetch_endpoint_validates_and_preserves_stream_metadata() {
    let transport = StreamTransport::new(FakeHttp {
        response: Mutex::new(Some(response(200, Some("audio")))),
    });
    let source = transport
        .fetch_endpoint(FetchEndpointOptions {
            stream_url: "https://example.test/audio".into(),
            api_key: None,
            source_name: "test source".into(),
            signal: None,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(source.codec, "alac");
    assert_eq!(source.bit_depth, 24);
    assert_eq!(source.sample_rate, 96_000);
    assert_eq!(source.content_length, Some(5));
}

#[tokio::test]
async fn fetch_endpoint_rejects_cancelled_requests() {
    let signal = CancellationToken::new();
    signal.cancel();
    let transport = StreamTransport::new(FakeHttp {
        response: Mutex::new(Some(response(200, Some("audio")))),
    });
    let error = transport
        .fetch_endpoint(FetchEndpointOptions {
            stream_url: "https://example.test/audio".into(),
            api_key: None,
            source_name: "test source".into(),
            signal: Some(signal),
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Download was cancelled");
}
