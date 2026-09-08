//! Database dump export/import (oracle: `src/db/dump.ts`).
//!
//! Export: SELECT all rows → build transactional upsert SQL statements →
//! gzip → `.sql.gz` document. Import: gunzip → execute every statement in
//! one transaction (rollback on failure). Statement counting mirrors the
//! oracle's "merged" numbers.

use std::time::Instant;

use flate2::{write::GzEncoder, Compression};
use welds::connections::{transaction::Transaction, Client, TransactStart};

use crate::DbError;

/// Oracle DumpStats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpStats {
    pub users_count: i64,
    pub tracks_count: i64,
    pub requests_count: i64,
    pub bytes: usize,
}

/// Oracle RestoreStats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreStats {
    pub users_merged: u64,
    pub tracks_merged: u64,
    pub requests_merged: u64,
    pub duration_ms: u128,
}

pub struct DbDumpService {
    client: welds::connections::postgres::PostgresClient,
}

impl DbDumpService {
    pub fn new(client: welds::connections::postgres::PostgresClient) -> Self {
        Self { client }
    }

    /// Export the users/tracks/requests tables as a gzipped SQL dump.
    pub async fn export_dump(&self) -> Result<(Vec<u8>, DumpStats, String), DbError> {
        let started = Instant::now();
        let mut lines: Vec<String> = Vec::new();
        // Oracle header lines.
        lines.push("-- ALAC Telegram Bot Database Dump".to_owned());
        lines.push(format!(
            "-- Generated: {}",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        ));
        lines.push("-- Format: SQL-GZ Transactional Upsert Dump".to_owned());
        lines.push(String::new());

        let users = self
            .client
            .fetch_rows("SELECT telegram_id, name, created_at FROM users", &[])
            .await?;
        let users_count = users.len() as i64;
        for row in users {
            let telegram_id: i64 = row
                .get("telegram_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let name: Option<String> = row
                .get("name")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let created_at: chrono::NaiveDateTime = row
                .get("created_at")
                .map_err(|error| DbError::Row(error.to_string()))?;
            lines.push(format!(
                "INSERT INTO users (telegram_id, name, created_at) VALUES ({}, {}, '{}') ON CONFLICT (telegram_id) DO UPDATE SET name = EXCLUDED.name, created_at = LEAST(users.created_at, EXCLUDED.created_at);",
                telegram_id,
                escape_sql_string(name.as_deref().unwrap_or("")),
                created_at
            ));
        }

        let tracks = self.client.fetch_rows("SELECT * FROM tracks", &[]).await?;
        let tracks_count = tracks.len() as i64;
        for row in tracks {
            let apple_track_id: String = row
                .get("apple_track_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let message_id: i32 = row
                .get("message_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let file_id: String = row
                .get("file_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let file_unique_id: String = row
                .get("file_unique_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let title: String = row
                .get("title")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let artist: String = row
                .get("artist")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let album: String = row
                .get("album")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let duration: i32 = row
                .get("duration")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let bit_depth: i32 = row
                .get("bit_depth")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let sample_rate: i32 = row
                .get("sample_rate")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let genre: String = row
                .get("genre")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let release_date: String = row
                .get("release_date")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let track_number: i32 = row
                .get("track_number")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let track_count: i32 = row
                .get("track_count")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let created_at: chrono::NaiveDateTime = row
                .get("created_at")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let updated_at: chrono::NaiveDateTime = row
                .get("updated_at")
                .map_err(|error| DbError::Row(error.to_string()))?;
            lines.push(format!(
                "INSERT INTO tracks (apple_track_id, message_id, file_id, file_unique_id, title, artist, album, duration, bit_depth, sample_rate, genre, release_date, track_number, track_count, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, '{}', '{}') ON CONFLICT (apple_track_id) DO UPDATE SET message_id = EXCLUDED.message_id, file_id = EXCLUDED.file_id, file_unique_id = EXCLUDED.file_unique_id, title = EXCLUDED.title, artist = EXCLUDED.artist, album = EXCLUDED.album, duration = EXCLUDED.duration, bit_depth = EXCLUDED.bit_depth, sample_rate = EXCLUDED.sample_rate, genre = EXCLUDED.genre, release_date = EXCLUDED.release_date, track_number = EXCLUDED.track_number, track_count = EXCLUDED.track_count, updated_at = GREATEST(tracks.updated_at, EXCLUDED.updated_at);",
                escape_sql_string(&apple_track_id),
                message_id,
                escape_sql_string(&file_id),
                escape_sql_string(&file_unique_id),
                escape_sql_string(&title),
                escape_sql_string(&artist),
                escape_sql_string(&album),
                duration,
                bit_depth,
                sample_rate,
                escape_sql_string(&genre),
                escape_sql_string(&release_date),
                track_number,
                track_count,
                created_at,
                updated_at
            ));
        }

        let requests = self
            .client
            .fetch_rows("SELECT * FROM requests", &[])
            .await?;
        let requests_count = requests.len() as i64;
        for row in requests {
            let telegram_id: i64 = row
                .get("telegram_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let chat_id: i64 = row
                .get("chat_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let apple_track_id: String = row
                .get("apple_track_id")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let is_cache_hit: bool = row
                .get("is_cache_hit")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let duration_ms: Option<i32> = row
                .get("duration_ms")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let status: String = row
                .get("status")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let error_reason: Option<String> = row
                .get("error_reason")
                .map_err(|error| DbError::Row(error.to_string()))?;
            let created_at: chrono::NaiveDateTime = row
                .get("created_at")
                .map_err(|error| DbError::Row(error.to_string()))?;
            lines.push(format!(
                "INSERT INTO requests (telegram_id, chat_id, apple_track_id, is_cache_hit, duration_ms, status, error_reason, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, '{}');",
                telegram_id,
                chat_id,
                escape_sql_string(&apple_track_id),
                if is_cache_hit { "TRUE" } else { "FALSE" },
                duration_ms.map(|v| v.to_string()).unwrap_or_else(|| "NULL".to_owned()),
                escape_sql_string(&status),
                escape_sql_string(error_reason.as_deref().unwrap_or("")),
                created_at
            ));
        }

        lines.push(
            "SELECT setval(pg_get_serial_sequence('tracks', 'id'), COALESCE((SELECT MAX(id) FROM tracks), 1), (SELECT MAX(id) IS NOT NULL FROM tracks));"
                .to_owned(),
        );
        lines.push(
            "SELECT setval(pg_get_serial_sequence('requests', 'id'), COALESCE((SELECT MAX(id) FROM requests), 1), (SELECT MAX(id) IS NOT NULL FROM requests));"
                .to_owned(),
        );

        let sql_content = lines.join("\n");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, sql_content.as_bytes())
            .map_err(|error| DbError::Row(format!("gzip encode failed: {error}")))?;
        let compressed = encoder
            .finish()
            .map_err(|error| DbError::Row(format!("gzip finish failed: {error}")))?;

        // Oracle: ISO timestamp with : and . replaced by -, first 19 chars.
        let bytes = compressed.len();
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let timestamp_str: String = timestamp
            .chars()
            .map(|c| if c == ':' || c == '.' { '-' } else { c })
            .take(19)
            .collect();
        let filename = format!("alac_dump_{timestamp_str}.sql.gz");

        tracing::info!(
            users = users_count,
            tracks = tracks_count,
            requests = requests_count,
            elapsed_ms = started.elapsed().as_millis(),
            "database dump exported"
        );

        Ok((
            compressed,
            DumpStats {
                users_count,
                tracks_count,
                requests_count,
                bytes,
            },
            filename,
        ))
    }

    /// Restore a gzipped SQL dump inside one transaction.
    pub async fn import_dump(&self, gzip_bytes: &[u8]) -> Result<RestoreStats, DbError> {
        let started = Instant::now();
        let decompressed = gunzip(gzip_bytes)?;
        let sql_text = String::from_utf8(decompressed)
            .map_err(|error| DbError::Row(format!("dump is not valid UTF-8: {error}")))?;

        let statements: Vec<&str> = sql_text
            .split('\n')
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
            .collect();

        let mut users_merged = 0u64;
        let mut tracks_merged = 0u64;
        let mut requests_merged = 0u64;
        for stmt in &statements {
            if stmt.starts_with("INSERT INTO users") {
                users_merged += 1;
            } else if stmt.starts_with("INSERT INTO tracks") {
                tracks_merged += 1;
            } else if stmt.starts_with("INSERT INTO requests") {
                requests_merged += 1;
            }
        }

        let tx: Transaction<'_> = self.client.begin().await?;
        for stmt in &statements {
            if let Err(error) = tx.execute(stmt, &[]).await {
                tx.rollback().await.ok();
                return Err(DbError::Database(error));
            }
        }
        tx.commit().await?;

        let stats = RestoreStats {
            users_merged,
            tracks_merged,
            requests_merged,
            duration_ms: started.elapsed().as_millis(),
        };
        tracing::info!(?stats, "database dump imported");
        Ok(stats)
    }
}

/// Oracle escapeSqlString: wrap in single quotes, doubling embedded quotes.
fn escape_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, DbError> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|error| DbError::Row(format!("gunzip failed: {error}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sql_string_doubles_quotes() {
        assert_eq!(escape_sql_string("plain"), "'plain'");
        assert_eq!(escape_sql_string("it's"), "'it''s'");
        assert_eq!(escape_sql_string(""), "''");
    }

    #[test]
    fn gzip_round_trip() {
        use std::io::Write;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"hello dump").unwrap();
        let compressed = encoder.finish().unwrap();
        assert_eq!(gunzip(&compressed).unwrap(), b"hello dump");
    }

    #[test]
    fn dump_filename_format_matches_oracle() {
        let timestamp = "2026-09-08T12:34:56.789Z";
        let mapped: String = timestamp
            .chars()
            .map(|c| if c == ':' || c == '.' { '-' } else { c })
            .take(19)
            .collect();
        assert_eq!(mapped, "2026-09-08T12-34-56");
    }
}
