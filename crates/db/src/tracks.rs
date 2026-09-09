use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
};

use diesel::{
    dsl::now,
    prelude::*,
    sql_query,
    sql_types::{Integer, Text},
};
use diesel_async::RunQueryDsl;
use engine::orchestrator::deps::{CachedTrack, SaveTrackInput};

use crate::{models::NewTrack, schema::tracks, DbError, DbPool, Track};

fn cached_track(track: Track) -> CachedTrack {
    CachedTrack {
        track_key: engine::TrackKey::new(track.provider, track.track_id),
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
    pool: DbPool,
}

impl TracksRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn find_cached_tracks(
        &self,
        track_keys: &[engine::TrackKey],
    ) -> Result<HashMap<engine::TrackKey, CachedTrack>, DbError> {
        let unique_keys: Vec<engine::TrackKey> = track_keys
            .iter()
            .filter(|key| !key.track_id.is_empty())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if unique_keys.is_empty() {
            return Ok(HashMap::new());
        }
        let mut connection = self.pool.connection().await?;
        let mut query = tracks::table.into_boxed();
        for key in &unique_keys {
            query = query.or_filter(
                tracks::provider
                    .eq(key.provider)
                    .and(tracks::track_id.eq(&key.track_id)),
            );
        }
        let rows = query
            .select(Track::as_select())
            .load::<Track>(&mut *connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|track| {
                let key = engine::TrackKey::new(track.provider, track.track_id.clone());
                (key, cached_track(track))
            })
            .collect())
    }

    pub async fn find_track_by_file_unique_id(
        &self,
        file_unique_id: &str,
    ) -> Result<Option<Track>, DbError> {
        let mut connection = self.pool.connection().await?;
        Ok(tracks::table
            .filter(tracks::file_unique_id.eq(file_unique_id))
            .select(Track::as_select())
            .first::<Track>(&mut *connection)
            .await
            .optional()?)
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
        let mut connection = self.pool.connection().await?;
        let new_track = NewTrack {
            provider: input.track_key.provider,
            track_id: &input.track_key.track_id,
            message_id,
            file_id: &input.file_id,
            file_unique_id: &input.file_unique_id,
            title: &input.title,
            artist: &input.artist,
            album: &input.album,
            duration,
            bit_depth,
            sample_rate,
            genre: &input.genre,
            release_date: &input.release_date,
            track_number,
            track_count,
        };
        diesel::insert_into(tracks::table)
            .values(new_track)
            .on_conflict((tracks::provider, tracks::track_id))
            .do_update()
            .set((
                tracks::message_id.eq(message_id),
                tracks::file_id.eq(&input.file_id),
                tracks::file_unique_id.eq(&input.file_unique_id),
                tracks::title.eq(&input.title),
                tracks::artist.eq(&input.artist),
                tracks::album.eq(&input.album),
                tracks::duration.eq(duration),
                tracks::bit_depth.eq(bit_depth),
                tracks::sample_rate.eq(sample_rate),
                tracks::genre.eq(&input.genre),
                tracks::release_date.eq(&input.release_date),
                tracks::track_number.eq(track_number),
                tracks::track_count.eq(track_count),
                tracks::updated_at.eq(now),
            ))
            .execute(&mut *connection)
            .await?;
        tracks::table
            .filter(tracks::provider.eq(input.track_key.provider))
            .filter(tracks::track_id.eq(&input.track_key.track_id))
            .select(Track::as_select())
            .first::<Track>(&mut *connection)
            .await
            .map_err(DbError::from)
    }

    pub async fn delete_track(&self, track_key: &engine::TrackKey) -> Result<bool, DbError> {
        let mut connection = self.pool.connection().await?;
        Ok(diesel::delete(
            tracks::table
                .filter(tracks::provider.eq(track_key.provider))
                .filter(tracks::track_id.eq(&track_key.track_id)),
        )
        .execute(&mut *connection)
        .await?
            > 0)
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
        let limit = i32::try_from(limit).unwrap_or(i32::MAX);
        let mut connection = self.pool.connection().await?;
        Ok(sql_query("SELECT * FROM tracks WHERE (provider = 'apple' AND track_id = $1) OR title ILIKE $2 OR artist ILIKE $2 OR album ILIKE $2 OR word_similarity($1, title || ' ' || artist || ' ' || album) >= 0.35 ORDER BY CASE WHEN provider = 'apple' AND track_id = $1 THEN 3 WHEN (title ILIKE $2 OR artist ILIKE $2 OR album ILIKE $2) THEN 2 ELSE 1 END DESC, word_similarity($1, title || ' ' || artist || ' ' || album) DESC LIMIT $3")
            .bind::<Text, _>(trimmed)
            .bind::<Text, _>(&pattern)
            .bind::<Integer, _>(limit)
            .load::<Track>(&mut *connection)
            .await?)
    }

    pub async fn get_all_track_ids(&self) -> Result<Vec<engine::TrackKey>, DbError> {
        let mut connection = self.pool.connection().await?;
        let rows = tracks::table
            .select((tracks::provider, tracks::track_id))
            .load::<(engine::Provider, String)>(&mut *connection)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(provider, track_id)| engine::TrackKey::new(provider, track_id))
            .collect())
    }

    pub async fn delete_tracks_not_in(
        &self,
        valid_track_keys: &[engine::TrackKey],
    ) -> Result<u64, DbError> {
        let mut connection = self.pool.connection().await?;
        let valid: HashSet<_> = valid_track_keys.iter().cloned().collect();
        connection
            .build_transaction()
            .run(async |transaction| -> Result<u64, diesel::result::Error> {
                let rows = tracks::table
                    .select((tracks::id, tracks::provider, tracks::track_id))
                    .load::<(i32, engine::Provider, String)>(&mut *transaction)
                    .await?;
                let stale_ids: Vec<i32> = rows
                    .into_iter()
                    .filter_map(|(id, provider, track_id)| {
                        (!valid.contains(&engine::TrackKey::new(provider, track_id))).then_some(id)
                    })
                    .collect();
                if stale_ids.is_empty() {
                    return Ok(0_u64);
                }
                let deleted = diesel::delete(tracks::table.filter(tracks::id.eq_any(stale_ids)))
                    .execute(&mut *transaction)
                    .await?;
                Ok(deleted as u64)
            })
            .await
            .map_err(DbError::from)
    }
}
