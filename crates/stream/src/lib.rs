//! Telegram MTProto streaming engine for alac-bot.
//!
//! Provides a dedicated auxiliary worker pool, circuit breaker with FloodWait quarantine,
//! in-memory LRU chunk cache with byte-weight accounting, and backpressure-regulated
//! byte-range stream pipes.

pub mod cache;
pub mod circuit_breaker;
pub mod engine;
pub mod pipe;
pub mod worker_pool;

pub use cache::{ChunkCache, ChunkKey, CHUNK_SIZE};
pub use circuit_breaker::CircuitBreaker;
pub use engine::{AudioStreamResponse, StreamEngine, TrackMediaMetadata};
pub use pipe::{create_stream_pipe, ByteRange, ChunkStream, LocationRefresher, StreamPipeParams};

pub use worker_pool::{hash_bot_token, StreamWorkerPool};

/// Streaming engine errors.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("File reference expired")]
    FileReferenceExpired,

    #[error("Flood wait: {0} seconds")]
    FloodWait(u64),

    #[error("All workers quarantined or unavailable")]
    AllWorkersUnavailable,

    #[error("Unsupported CDN redirect")]
    UnsupportedCdnRedirect,

    #[error("Telegram connect error: {0}")]
    Connect(#[from] ferogram::QuickConnectError),

    #[error("Telegram invocation error: {0}")]
    Telegram(#[from] ferogram::InvocationError),

    #[error("Database error: {0}")]
    Db(#[from] db::DbError),

    #[error("Track not found: {0}")]
    TrackNotFound(i32),

    #[error("No media document found for track {0}")]
    NoMediaDocument(i32),

    #[error("Invalid range: {0}")]
    InvalidRange(String),
}
