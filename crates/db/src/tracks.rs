use std::{borrow::Borrow, collections::HashMap};

use chrono::{DateTime, Utc};
use engine::orchestrator::deps::{CachedTrack, SaveTrackInput};
use welds::connections::{Client, Param};

use crate::{DbError, Track};

fn row_value<
    T: Send + 'static + for<'a> sqlx::Decode<'a, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
>(
    row: &welds::connections::Row,
    column: &str,
) -> Result<T, DbError> {
    row.get(column)
        .map_err(|error| DbError::Row(error.to_string()))
}

fn track_from_row(row: &welds::connections::Row) -> Result<Track, DbError> {
    Ok(Track {
        id: row_value(row, "id")?,
        apple_track_id: row_value(row, "apple_track_id")?,
        message_id: row_value(row, "message_id")?,
        file_id: row_value(row, "file_id")?,
        file_unique_id: row_value(row, "file_unique_id")?,
        title: row_value(row, "title")?,
        artist: row_value(row, "artist")?,
        album: row_value(row, "album")?,
        duration: row_value(row, "duration")?,
        bit_depth: row_value(row, "bit_depth")?,
        sample_rate: row_value(row, "sample_rate")?,
        genre: row_value(row, "genre")?,
        release_date: row_value(row, "release_date")?,
        track_number: row_value(row, "track_number")?,
        track_count: row_value(row, "track_count")?,
        created_at: row_value(row, "created_at")?,
        updated_at: row_value(row, "updated_at")?,
    })
}

fn cached_track(track: Track) -> CachedTrack {
    CachedTrack {
        apple_track_id: track.apple_track_id,
        message_id: i64::from(track.message_id),
        file_id: track.file_id,
        file_unique_id: track.file_unique_id,
        title: track.title,
        artist: track.artist,
        album: track.album,
    }
}

/// Database repository for the Telegram audio cache.
#[derive(Clone)]
pub struct TracksRepository {
    client: welds::connections::postgres::PostgresClient,
}

impl TracksRepository {
    pub fn new(client: welds::connections::postgres::PostgresClient) -> Self {
        Self { client }
    }

    pub async fn find_cached_tracks(
        &self,
        apple_track_ids: &[String],
    ) -> Result<HashMap<String, CachedTrack>, DbError> {
        let unique_ids: Vec<String> = apple_track_ids
            .iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        if unique_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows = self
            .client
            .fetch_rows(
                "SELECT * FROM tracks WHERE apple_track_id = ANY($1)",
                &[&unique_ids as &(dyn Param + Sync)],
            )
            .await?;
        let mut result = HashMap::with_capacity(rows.len());
        for row in rows {
            let track = track_from_row(&row)?;
            result.insert(track.apple_track_id.clone(), cached_track(track));
        }
        Ok(result)
    }

    pub async fn find_track_by_file_unique_id(
        &self,
        file_unique_id: &str,
    ) -> Result<Option<Track>, DbError> {
        let file_unique_id = file_unique_id.to_owned();
        let rows = self
            .client
            .fetch_rows(
                "SELECT * FROM tracks WHERE file_unique_id = $1 LIMIT 1",
                &[&file_unique_id as &(dyn Param + Sync)],
            )
            .await?;
        rows.first().map(track_from_row).transpose()
    }

    pub async fn save_track<I>(&self, input: I) -> Result<Track, DbError>
    where
        I: Borrow<SaveTrackInput>,
    {
        let input = input.borrow();
        let message_id = i32::try_from(input.message_id)
            .map_err(|error| DbError::Row(format!("message_id out of range: {error}")))?;
        let duration = i32::try_from(input.duration)
            .map_err(|error| DbError::Row(format!("duration out of range: {error}")))?;
        let bit_depth = i32::try_from(input.bit_depth)
            .map_err(|error| DbError::Row(format!("bit_depth out of range: {error}")))?;
        let sample_rate = i32::try_from(input.sample_rate)
            .map_err(|error| DbError::Row(format!("sample_rate out of range: {error}")))?;
        let track_number = i32::try_from(input.track_number)
            .map_err(|error| DbError::Row(format!("track_number out of range: {error}")))?;
        let track_count = i32::try_from(input.track_count)
            .map_err(|error| DbError::Row(format!("track_count out of range: {error}")))?;
        let params: [&(dyn Param + Sync); 14] = [
            &input.apple_track_id,
            &message_id,
            &input.file_id,
            &input.file_unique_id,
            &input.title,
            &input.artist,
            &input.album,
            &duration,
            &bit_depth,
            &sample_rate,
            &input.genre,
            &input.release_date,
            &track_number,
            &track_count,
        ];
        let rows = self
            .client
            .fetch_rows(
                "INSERT INTO tracks (apple_track_id, message_id, file_id, file_unique_id, title, artist, album, duration, bit_depth, sample_rate, genre, release_date, track_number, track_count) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14) RETURNING *",
                &params,
            )
            .await?;
        rows.first()
            .ok_or_else(|| DbError::Row("track insert returned no row".to_owned()))
            .and_then(track_from_row)
    }

    pub async fn delete_track(&self, apple_track_id: &str) -> Result<bool, DbError> {
        let apple_track_id = apple_track_id.to_owned();
        let result = self
            .client
            .execute(
                "DELETE FROM tracks WHERE apple_track_id = $1",
                &[&apple_track_id as &(dyn Param + Sync)],
            )
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn search_cached_tracks(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Track>, DbError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = format!("%{trimmed}%");
        let trimmed = trimmed.to_owned();
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = self
            .client
            .fetch_rows(
                "SELECT * FROM tracks WHERE apple_track_id = $1 OR title ILIKE $2 OR artist ILIKE $2 OR album ILIKE $2 ORDER BY CASE WHEN apple_track_id = $1 THEN 3 WHEN (title ILIKE $2 OR artist ILIKE $2 OR album ILIKE $2) THEN 2 ELSE 1 END DESC LIMIT $3",
                &[
                    &trimmed as &(dyn Param + Sync),
                    &pattern as &(dyn Param + Sync),
                    &limit as &(dyn Param + Sync),
                ],
            )
            .await?;
        rows.iter().map(track_from_row).collect()
    }

    pub async fn get_all_track_ids(&self) -> Result<Vec<String>, DbError> {
        let rows = self
            .client
            .fetch_rows("SELECT apple_track_id FROM tracks", &[])
            .await?;
        rows.iter()
            .map(|row| row_value(row, "apple_track_id"))
            .collect()
    }

    pub async fn delete_tracks_not_in(&self, valid_track_ids: &[String]) -> Result<u64, DbError> {
        let result = if valid_track_ids.is_empty() {
            self.client.execute("DELETE FROM tracks", &[]).await?
        } else {
            let valid_track_ids = valid_track_ids.to_vec();
            self.client
                .execute(
                    "DELETE FROM tracks WHERE NOT (apple_track_id = ANY($1))",
                    &[&valid_track_ids as &(dyn Param + Sync)],
                )
                .await?
        };
        Ok(result.rows_affected())
    }
}

// Keep these imports in this module's type-check surface: the model's timestamp
// fields intentionally mirror the schema's timestamptz columns.
#[allow(dead_code)]
fn _timestamp_type_check(_: DateTime<Utc>) {}
