//! Mirror-policy discovery and failover audio streaming.

mod http;
mod mirror_policy;
mod stream_transport;

pub use http::{
    ByteStream, MirrorHttp, MirrorHttpError, ReqwestHttp, StreamBodyError, StreamHttp,
    StreamHttpError, StreamHttpResponse, CHROME_USER_AGENT,
};
pub use mirror_policy::{
    MirrorEndpoint, MirrorError, MirrorPolicy, MirrorPolicyManager, MANIFEST_URL,
};
pub use stream_transport::{
    AudioStreamSource, ConnectStreamOptions, FetchEndpointOptions, ProgressCallback, StreamError,
    StreamTransport,
};
