use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{error::ServerError, ServerState};

#[derive(Debug, Serialize, ToSchema)]
pub struct TrackSummaryDto {
    pub id: i32,
    pub provider: String,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i32,
    pub codec: String,
    pub bit_depth: Option<i32>,
    pub sample_rate: Option<i32>,
    pub is_cached: bool,
}

impl From<db::Track> for TrackSummaryDto {
    fn from(t: db::Track) -> Self {
        Self {
            id: t.id,
            provider: t.provider.as_str().to_string(),
            track_id: t.track_id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration,
            codec: t.codec.as_str().to_string(),
            bit_depth: Some(t.bit_depth),
            sample_rate: Some(t.sample_rate),
            is_cached: true,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TrackDetailDto {
    pub id: i32,
    pub provider: String,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i32,
    pub codec: String,
    pub bit_depth: i32,
    pub sample_rate: i32,
    pub genre: String,
    pub release_date: String,
    pub track_number: i32,
    pub track_count: i32,
    pub is_cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc_number: Option<i32>,
}

impl From<db::Track> for TrackDetailDto {
    fn from(t: db::Track) -> Self {
        Self {
            id: t.id,
            provider: t.provider.as_str().to_string(),
            track_id: t.track_id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            duration: t.duration,
            codec: t.codec.as_str().to_string(),
            bit_depth: t.bit_depth,
            sample_rate: t.sample_rate,
            genre: t.genre,
            release_date: t.release_date,
            track_number: t.track_number,
            track_count: t.track_count,
            is_cached: true,
            isrc: None,
            composer: None,
            disc_number: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UncachedTrackDto {
    pub provider: String,
    pub item_id: String,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: i32,
    pub is_cached: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    pub q: String,
    pub provider: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub cached: Vec<TrackSummaryDto>,
    pub live: Vec<UncachedTrackDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/search",
    params(
        SearchQuery
    ),
    responses(
        (status = 200, description = "Search cached and live catalog", body = SearchResponse)
    )
)]
pub async fn search_catalog(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ServerError> {
    let limit = query.limit.unwrap_or(20).clamp(1, 100) as usize;
    let page = query.page.unwrap_or(1).max(1);
    let offset = ((page - 1) * (limit as i64)) as usize;

    let cached_tracks = state
        .tracks_repo
        .search_cached_tracks(&query.q, offset + limit)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let cached: Vec<TrackSummaryDto> = cached_tracks
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(Into::into)
        .collect();

    // Query live catalog from catalog_service if available
    let mut live = Vec::new();
    if let Some(ref catalog) = state.catalog_service {
        let provider = query.provider.as_deref().unwrap_or("apple");
        if provider.eq_ignore_ascii_case("apple") {
            if let Ok(results) = catalog.search_catalog(&query.q, 10, "us").await {
                let cached_track_ids: std::collections::HashSet<&str> =
                    cached.iter().map(|c| c.track_id.as_str()).collect();
                live = results
                    .into_iter()
                    .filter(|item| !cached_track_ids.contains(item.id.as_str()))
                    .map(|item| UncachedTrackDto {
                        provider: "apple".to_string(),
                        item_id: item.id.clone(),
                        track_id: item.id,
                        title: item.title,
                        artist: item.artist,
                        album: item.album,
                        duration: item.duration_secs as i32,
                        is_cached: false,
                    })
                    .collect();
            }
        }
    }

    Ok(Json(SearchResponse { cached, live }))
}

#[utoipa::path(
    get,
    path = "/api/v1/tracks/{id}",
    params(
        ("id" = i32, Path, description = "Track ID")
    ),
    responses(
        (status = 200, description = "Detailed track metadata", body = TrackDetailDto),
        (status = 404, description = "Track not found")
    )
)]
pub async fn get_track(
    State(state): State<Arc<ServerState>>,
    Path(track_id): Path<i32>,
) -> Result<Json<TrackDetailDto>, ServerError> {
    let track = state
        .tracks_repo
        .find_track_by_id(track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .ok_or_else(|| ServerError::NotFound(format!("Track {track_id} not found")))?;

    Ok(Json(track.into()))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumSummaryDto {
    pub album: String,
    pub artist: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PaginationQuery {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/albums",
    params(
        PaginationQuery
    ),
    responses(
        (status = 200, description = "Paginated list of albums", body = Vec<AlbumSummaryDto>)
    )
)]
pub async fn list_albums(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<Vec<AlbumSummaryDto>>, ServerError> {
    let limit = query.limit.unwrap_or(30).clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

    let rows = state
        .tracks_repo
        .list_distinct_albums(limit, offset)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let albums = rows
        .into_iter()
        .map(|row| AlbumSummaryDto {
            album: row.album,
            artist: row.artist,
        })
        .collect();

    Ok(Json(albums))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumDetailsDto {
    pub album: String,
    pub artist: String,
    pub track_count: usize,
    pub tracks: Vec<TrackSummaryDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/albums/{id}",
    params(
        ("id" = String, Path, description = "Album Name or Track/Collection ID")
    ),
    responses(
        (status = 200, description = "Tracks in the specified album", body = AlbumDetailsDto),
        (status = 404, description = "Album not found")
    )
)]
pub async fn get_album_tracks(
    State(state): State<Arc<ServerState>>,
    Path(id_or_name): Path<String>,
) -> Result<Json<AlbumDetailsDto>, ServerError> {
    let mut tracks = state
        .tracks_repo
        .find_tracks_by_album(&id_or_name)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    if tracks.is_empty() {
        if let Ok(track_id) = id_or_name.parse::<i32>() {
            if let Ok(Some(track)) = state.tracks_repo.find_track_by_id(track_id).await {
                tracks = state
                    .tracks_repo
                    .find_tracks_by_album(&track.album)
                    .await
                    .map_err(|e| ServerError::Internal(e.to_string()))?;
            }
        }
    }

    if tracks.is_empty() {
        if let Some(ref catalog) = state.catalog_service {
            if let Ok(album_res) = catalog.fetch_album_tracks(&id_or_name, "us").await {
                let album = album_res.album.album.clone();
                let artist = album_res.album.artist.clone();
                let track_count = album_res.tracks.len();
                let dtos = album_res
                    .tracks
                    .into_iter()
                    .map(|t| TrackSummaryDto {
                        id: 0,
                        provider: "apple".to_string(),
                        track_id: t.id,
                        title: t.title,
                        artist: t.artist,
                        album: album.clone(),
                        duration: t.duration_secs as i32,
                        codec: "alac".to_string(),
                        bit_depth: None,
                        sample_rate: None,
                        is_cached: false,
                    })
                    .collect();
                return Ok(Json(AlbumDetailsDto {
                    album,
                    artist,
                    track_count,
                    tracks: dtos,
                }));
            }
        }
        return Err(ServerError::NotFound(format!("Album '{id_or_name}' not found")));
    }



    let album = tracks[0].album.clone();
    let artist = tracks[0].artist.clone();
    let track_count = tracks.len();
    let track_dtos = tracks.into_iter().map(Into::into).collect();

    Ok(Json(AlbumDetailsDto {
        album,
        artist,
        track_count,
        tracks: track_dtos,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/artists/{name}/tracks",
    params(
        ("name" = String, Path, description = "Artist Name")
    ),
    responses(
        (status = 200, description = "Tracks by artist", body = Vec<TrackSummaryDto>)
    )
)]
pub async fn get_artist_tracks(
    State(state): State<Arc<ServerState>>,
    Path(artist_name): Path<String>,
) -> Result<Json<Vec<TrackSummaryDto>>, ServerError> {
    let tracks = state
        .tracks_repo
        .find_tracks_by_artist(&artist_name, 50)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(Json(tracks.into_iter().map(Into::into).collect()))
}
