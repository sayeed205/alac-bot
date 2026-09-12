use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use music::{Codec, Provider};

use crate::{
    models::{Album, NewAlbum},
    schema::albums,
    DbError, DbPool,
};

/// Database repository for cached album archive parts.
#[derive(Clone)]
pub struct AlbumsRepository {
    pool: DbPool,
}

impl AlbumsRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Finds all archive parts for a given album, ordered by part_index.
    pub async fn find_albums(
        &self,
        provider: Provider,
        album_id: &str,
        codec: Option<Codec>,
    ) -> Result<Vec<Album>, DbError> {
        let mut connection = self.pool.connection().await?;
        let mut query = albums::table
            .filter(albums::provider.eq(provider))
            .filter(albums::album_id.eq(album_id))
            .order(albums::part_index.asc())
            .into_boxed();
        if let Some(c) = codec {
            query = query.filter(albums::codec.eq(c));
        }
        let rows = query
            .select(Album::as_select())
            .load::<Album>(&mut *connection)
            .await?;
        Ok(rows)
    }

    /// Finds a single album archive record by Telegram file unique id.
    pub async fn find_by_file_unique_id(
        &self,
        file_unique_id: &str,
    ) -> Result<Option<Album>, DbError> {
        let mut connection = self.pool.connection().await?;
        Ok(albums::table
            .filter(albums::file_unique_id.eq(file_unique_id))
            .select(Album::as_select())
            .first::<Album>(&mut *connection)
            .await
            .optional()?)
    }

    /// Inserts or updates an album archive part record.
    pub async fn save_album(&self, input: &NewAlbum<'_>) -> Result<Album, DbError> {
        let mut connection = self.pool.connection().await?;
        diesel::insert_into(albums::table)
            .values(input)
            .on_conflict((
                albums::provider,
                albums::album_id,
                albums::codec,
                albums::part_index,
            ))
            .do_update()
            .set((
                albums::total_parts.eq(input.total_parts),
                albums::message_id.eq(input.message_id),
                albums::file_id.eq(input.file_id),
                albums::file_unique_id.eq(input.file_unique_id),
                albums::file_size.eq(input.file_size),
                albums::file_name.eq(input.file_name),
                albums::generation_hash.eq(input.generation_hash),
            ))
            .execute(&mut *connection)
            .await?;

        albums::table
            .filter(albums::provider.eq(input.provider))
            .filter(albums::album_id.eq(input.album_id))
            .filter(albums::codec.eq(input.codec))
            .filter(albums::part_index.eq(input.part_index))
            .select(Album::as_select())
            .first::<Album>(&mut *connection)
            .await
            .map_err(DbError::from)
    }

    /// Deletes all archive parts for an album, returning the deleted records.
    pub async fn delete_albums(
        &self,
        provider: Provider,
        album_id: &str,
        codec: Option<Codec>,
    ) -> Result<Vec<Album>, DbError> {
        let existing = self.find_albums(provider, album_id, codec).await?;
        if existing.is_empty() {
            return Ok(Vec::new());
        }
        let mut connection = self.pool.connection().await?;
        let mut query = diesel::delete(albums::table)
            .filter(albums::provider.eq(provider))
            .filter(albums::album_id.eq(album_id))
            .into_boxed();
        if let Some(c) = codec {
            query = query.filter(albums::codec.eq(c));
        }
        query.execute(&mut *connection).await?;
        Ok(existing)
    }

    /// Lists album archives for indexing/exporting.
    pub async fn list_albums(&self) -> Result<Vec<Album>, DbError> {
        let mut connection = self.pool.connection().await?;
        Ok(albums::table
            .order(albums::id.desc())
            .select(Album::as_select())
            .load::<Album>(&mut *connection)
            .await?)
    }
}
