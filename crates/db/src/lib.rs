//! Database access for the bot.

mod auth;
mod migrations;
mod models;

pub use auth::{Auth, AuthedPeer};
pub use migrations::{initial_schema, migrate};
pub use models::User;

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
