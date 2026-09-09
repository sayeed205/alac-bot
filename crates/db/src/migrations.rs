use welds::{errors::Result, migrations::prelude::*};

/// The complete schema represented by the three Drizzle migrations.
///
/// Every statement is idempotent (`IF NOT EXISTS`) so the first Rust start
/// against a database already migrated by the TS bot (drizzle journal in
/// `drizzle.__drizzle_migrations`) is a no-op: welds keeps its own
/// bookkeeping (`_welds_migrations`), so without this the initial step would
/// attempt to re-create existing tables and abort startup.
pub const SQL: &str = r#"
CREATE TABLE IF NOT EXISTS "tracks" (
    "id" serial PRIMARY KEY NOT NULL,
    "apple_track_id" text NOT NULL,
    "message_id" integer NOT NULL,
    "file_id" text NOT NULL,
    "file_unique_id" text NOT NULL,
    "title" text NOT NULL,
    "artist" text NOT NULL,
    "album" text NOT NULL,
    "duration" integer NOT NULL,
    "bit_depth" integer NOT NULL,
    "sample_rate" integer NOT NULL,
    "genre" text NOT NULL,
    "release_date" text NOT NULL,
    "track_number" integer NOT NULL,
    "track_count" integer NOT NULL,
    "created_at" timestamp with time zone DEFAULT now() NOT NULL,
    "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT "tracks_apple_track_id_unique" UNIQUE("apple_track_id")
);
CREATE TABLE IF NOT EXISTS "users" (
    "telegram_id" bigint PRIMARY KEY NOT NULL,
    "name" text,
    "created_at" timestamp with time zone DEFAULT now() NOT NULL
);
CREATE INDEX IF NOT EXISTS "tracks_title_idx" ON "tracks" USING btree ("title");
CREATE INDEX IF NOT EXISTS "tracks_artist_idx" ON "tracks" USING btree ("artist");
CREATE TABLE IF NOT EXISTS "requests" (
    "id" serial PRIMARY KEY NOT NULL,
    "telegram_id" bigint NOT NULL,
    "chat_id" bigint NOT NULL,
    "apple_track_id" text NOT NULL,
    "is_cache_hit" boolean NOT NULL,
    "duration_ms" integer,
    "status" text NOT NULL,
    "error_reason" text,
    "created_at" timestamp with time zone DEFAULT now() NOT NULL
);
CREATE INDEX IF NOT EXISTS "requests_apple_track_id_idx" ON "requests" USING btree ("apple_track_id");
CREATE INDEX IF NOT EXISTS "requests_telegram_id_idx" ON "requests" USING btree ("telegram_id");
CREATE INDEX IF NOT EXISTS "requests_created_at_idx" ON "requests" USING btree ("created_at");
CREATE TABLE IF NOT EXISTS "settings" (
    "key" text PRIMARY KEY NOT NULL,
    "value" jsonb NOT NULL,
    "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
"#;

/// Drizzle migration 0001: pg_trgm extension + the trigram search index.
pub const TRGM_SQL: &str = r#"
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX IF NOT EXISTS "tracks_search_trgm_idx" ON "tracks" USING gin (("title" || ' ' || "artist" || ' ' || "album") gin_trgm_ops);
"#;

/// Construct the single initial migration.
pub fn initial_schema(_state: &TableState) -> Result<MigrationStep> {
    Ok(MigrationStep::new(
        "initial_schema",
        Manual::up(SQL).down("DROP TABLE users, tracks, requests, settings CASCADE;"),
    ))
}

/// Drizzle 0001 as its own step so pre-trgm databases upgrade cleanly.
pub fn trgm_extension(_state: &TableState) -> Result<MigrationStep> {
    Ok(MigrationStep::new(
        "trgm_extension",
        Manual::up(TRGM_SQL).down("DROP INDEX IF EXISTS tracks_search_trgm_idx;"),
    ))
}

/// Run all database migrations. Welds records completed migrations and makes
/// repeated calls a no-op.
pub async fn migrate(
    client: &welds::connections::postgres::PostgresClient,
) -> std::result::Result<(), welds::WeldsError> {
    welds::migrations::up(client, &[initial_schema, trgm_extension]).await
}
