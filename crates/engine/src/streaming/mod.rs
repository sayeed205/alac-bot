//! Provider-neutral HTTP and audio streaming primitives.

mod http;
mod stream_transport;

pub use http::{
    ByteStream, ReqwestHttp, StreamBodyError, StreamHttp, StreamHttpError, StreamHttpResponse,
    CHROME_USER_AGENT,
};
pub use stream_transport::{
    AudioStreamSource, FetchEndpointOptions, ProgressCallback, StreamError, StreamTransport,
};
