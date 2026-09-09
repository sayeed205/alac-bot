use diesel::Connection;
use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::{DbError, DbPool};

/// Embedded Diesel migrations for this crate.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// Apply the embedded canonical schema migrations.
pub async fn migrate(pool: &DbPool) -> Result<(), DbError> {
    let database_url = pool.database_url().to_owned();
    tokio::task::spawn_blocking(move || {
        let mut connection =
            AsyncConnectionWrapper::<diesel_async::AsyncPgConnection>::establish(&database_url)
                .map_err(|error| DbError::Migration(error.to_string()))?;
        connection
            .run_pending_migrations(MIGRATIONS)
            .map(|_| ())
            .map_err(|error| DbError::Migration(error.to_string()))
    })
    .await
    .map_err(|error| DbError::Migration(error.to_string()))?
}
