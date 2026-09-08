//! Database access for the bot.

mod auth;
mod migrations;
mod models;
mod requests;
mod settings;
mod tracks;

pub use auth::{Auth, AuthedPeer};
pub use engine::orchestrator::deps::{CachedTrack, RequestLog, SaveTrackInput};
pub use migrations::{initial_schema, migrate};
pub use models::{Request, Setting, Track, User};
pub use requests::RequestLogRepository;
pub use settings::SettingsStore;
pub use tracks::TracksRepository;

/// Errors returned by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Database(#[from] welds::connections::Error),
    #[error("migration error: {0}")]
    Migration(#[from] welds::WeldsError),
    #[error("database row error: {0}")]
    Row(String),
}

impl From<Box<dyn std::error::Error + Send + Sync>> for DbError {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Row(error.to_string())
    }
}
