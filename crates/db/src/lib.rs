//! Database access for the bot.

use std::sync::Arc;

use diesel_async::{pooled_connection::bb8::Pool, AsyncPgConnection};

mod auth;
mod migrations;
mod models;
mod requests;
mod schema;
mod settings;
mod tracks;

pub mod dump;
mod stats;

pub use auth::{Auth, AuthedPeer};
pub use dump::{DbDumpService, DumpStats, RestoreStats};
pub use engine::{
    orchestrator::deps::{CachedTrack, RequestLog, SaveTrackInput},
    Provider, TrackKey,
};
pub use migrations::migrate;
pub use models::{Request, SettingsRow, Track, User};
pub use requests::RequestLogRepository;
pub use settings::SettingsStore;
pub use stats::{AlacStats, StatsRepository, TopTrackStat};
pub use tracks::TracksRepository;

type DieselManager =
    diesel_async::pooled_connection::AsyncDieselConnectionManager<AsyncPgConnection>;
type DieselPool = Pool<AsyncPgConnection>;

/// A cloneable async PostgreSQL pool. The URL is retained solely so Diesel's
/// migration harness can run in its required blocking wrapper.
#[derive(Clone)]
pub struct DbPool {
    pool: DieselPool,
    database_url: Arc<str>,
}

impl DbPool {
    pub async fn connection(
        &self,
    ) -> Result<
        diesel_async::pooled_connection::bb8::PooledConnection<'_, AsyncPgConnection>,
        DbError,
    > {
        self.pool
            .get()
            .await
            .map_err(|error| DbError::Pool(error.to_string()))
    }

    pub(crate) fn database_url(&self) -> &str {
        &self.database_url
    }
}

/// Establish the shared Diesel async pool.
pub async fn connect(database_url: &str) -> Result<DbPool, DbError> {
    let manager = DieselManager::new(database_url);
    let pool = Pool::builder()
        .build(manager)
        .await
        .map_err(|error| DbError::Pool(error.to_string()))?;
    let database = DbPool {
        pool,
        database_url: Arc::from(database_url.to_owned()),
    };
    // Fail during application bootstrap, not on the first repository call.
    // The pool still performs normal health checks for subsequent requests.
    {
        let _connection = database.connection().await?;
    }
    Ok(database)
}

/// Errors returned by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("database pool error: {0}")]
    Pool(String),
    #[error("migration error: {0}")]
    Migration(String),
    #[error("database row error: {0}")]
    Row(String),
}

impl From<Box<dyn std::error::Error + Send + Sync>> for DbError {
    fn from(error: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self::Row(error.to_string())
    }
}
