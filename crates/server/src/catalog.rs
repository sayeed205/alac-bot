use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{error::ServerError, ServerState};

/// Summary information for a track in catalog listings.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackSummaryDto {
    /// Unique database track ID (0 for uncached live tracks).
    #[schema(example = 42)]
    pub id: i32,
    /// Music provider name (`apple`, `qobuz`).
    #[schema(example = "apple")]
    pub provider: String,
    /// Provider-native track identifier.
    #[schema(example = "1440857781")]
    pub track_id: String,
    /// Track title.
    #[schema(example = "Blank Space")]
    pub title: String,
    /// Primary artist name.
    #[schema(example = "Taylor Swift")]
    pub artist: String,
    /// Album name.
    #[schema(example = "1989")]
    pub album: String,
    /// Duration of the audio track in seconds.
    #[schema(example = 231)]
    pub duration: i32,
    /// Lossless or compressed audio codec.
    #[schema(example = "alac")]
    pub codec: String,
    /// Audio bit depth (e.g. 16 or 24).
    #[schema(example = 24)]
    pub bit_depth: Option<i32>,
    /// Audio sample rate in Hz (e.g. 44100, 96000).
    #[schema(example = 44100)]
    pub sample_rate: Option<i32>,
    /// Whether this track is cached in Telegram and playable instantly.
    #[schema(example = true)]
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

/// Comprehensive track metadata and technical specifications.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrackDetailDto {
    /// Unique database track ID.
    #[schema(example = 42)]
    pub id: i32,
    /// Music provider (`apple`, `qobuz`).
    #[schema(example = "apple")]
    pub provider: String,
    /// Provider-native track ID.
    #[schema(example = "1440857781")]
    pub track_id: String,
    /// Track title.
    #[schema(example = "Blank Space")]
    pub title: String,
    /// Primary artist name.
    #[schema(example = "Taylor Swift")]
    pub artist: String,
    /// Album name.
    #[schema(example = "1989")]
    pub album: String,
    /// Duration in seconds.
    #[schema(example = 231)]
    pub duration: i32,
    /// Codec identifier (`alac`, `flac`, `aac`).
    #[schema(example = "alac")]
    pub codec: String,
    /// Bit depth.
    #[schema(example = 24)]
    pub bit_depth: i32,
    /// Sample rate in Hz.
    #[schema(example = 44100)]
    pub sample_rate: i32,
    /// Musical genre.
    #[schema(example = "Pop")]
    pub genre: String,
    /// Release date string (YYYY-MM-DD).
    #[schema(example = "2014-10-27")]
    pub release_date: String,
    /// Track sequence number on disc.
    #[schema(example = 2)]
    pub track_number: i32,
    /// Total tracks on disc.
    #[schema(example = 13)]
    pub track_count: i32,
    /// True if already cached in Telegram dump channel.
    #[schema(example = true)]
    pub is_cached: bool,
    /// International Standard Recording Code.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "USCJY1431245")]
    pub isrc: Option<String>,
    /// Track composer.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Taylor Swift, Max Martin, Shellback")]
    pub composer: Option<String>,
    /// Disc number for multi-disc releases.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
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

/// Representation of an uncached live catalog track discovered via provider search.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UncachedTrackDto {
    /// Music provider (`apple`, `qobuz`).
    #[schema(example = "apple")]
    pub provider: String,
    /// Provider item or store identifier.
    #[schema(example = "1440857781")]
    pub item_id: String,
    /// Provider track identifier.
    #[schema(example = "1440857781")]
    pub track_id: String,
    /// Track title.
    #[schema(example = "Blank Space")]
    pub title: String,
    /// Artist name.
    #[schema(example = "Taylor Swift")]
    pub artist: String,
    /// Album name.
    #[schema(example = "1989")]
    pub album: String,
    /// Duration in seconds.
    #[schema(example = 231)]
    pub duration: i32,
    /// False for uncached live search results.
    #[schema(example = false)]
    pub is_cached: bool,
}

/// Search query parameters.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchQuery {
    /// Search query string (song title, artist, or album).
    #[param(example = "Taylor Swift Blank Space")]
    pub q: String,
    /// Target provider filter (`apple` or `qobuz`).
    #[param(example = "apple")]
    pub provider: Option<String>,
    /// Page number (1-based pagination).
    #[param(example = 1)]
    pub page: Option<i64>,
    /// Results per page (default: 20, max: 100).
    #[param(example = 20)]
    pub limit: Option<i64>,
}

/// Unified search response containing both cached and live catalog results.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    /// Cached tracks playable instantly without ripping.
    pub cached: Vec<TrackSummaryDto>,
    /// Live provider catalog tracks that can be ripped on-demand.
    pub live: Vec<UncachedTrackDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/search",
    tag = "catalog",
    summary = "Search Cached & Live Music Catalog",
    description = "Searches tracks with pagination (`page`, `limit`). Returns both instantly playable cached tracks and live provider catalog results.",
    params(
        SearchQuery
    ),
    responses(
        (status = 200, description = "Deduplicated cached and live search results", body = SearchResponse)
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
    tag = "catalog",
    summary = "Get Track Metadata & Audio Specs",
    description = "Retrieves full track details including codec, bit depth, sample rate, ISRC, composer, and cached status.",
    params(
        ("id" = i32, Path, description = "Unique database track ID", example = 42)
    ),
    responses(
        (status = 200, description = "Comprehensive track metadata", body = TrackDetailDto),
        (status = 404, description = "Track not found in database cache")
    ),
    security(
        ("bearer_auth" = [])
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

/// Summary of a distinct cached album.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumSummaryDto {
    /// Album title.
    #[schema(example = "1989")]
    pub album: String,
    /// Primary album artist.
    #[schema(example = "Taylor Swift")]
    pub artist: String,
}

/// Standard pagination query parameters.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PaginationQuery {
    /// Page number (1-based index).
    #[param(example = 1)]
    pub page: Option<i64>,
    /// Number of items per page (default: 30, max: 100).
    #[param(example = 30)]
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/albums",
    tag = "catalog",
    summary = "List Cached Albums",
    description = "Returns paginated list of distinct albums and artists cached in the database.",
    params(
        PaginationQuery
    ),
    responses(
        (status = 200, description = "Paginated list of distinct cached albums", body = Vec<AlbumSummaryDto>)
    ),
    security(
        ("bearer_auth" = [])
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

/// Album details with complete tracklist.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumDetailsDto {
    /// Album title.
    #[schema(example = "1989")]
    pub album: String,
    /// Album artist.
    #[schema(example = "Taylor Swift")]
    pub artist: String,
    /// Total count of tracks in this album.
    #[schema(example = 13)]
    pub track_count: usize,
    /// Ordered list of tracks in the album.
    pub tracks: Vec<TrackSummaryDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/albums/{id}",
    tag = "catalog",
    summary = "Get Album Tracklist",
    description = "Returns the complete ordered tracklist for an album, checking the database cache first and falling back to live Apple Music catalog if uncached.",
    params(
        ("id" = String, Path, description = "Album title, collection ID, or track ID", example = "1989")
    ),
    responses(
        (status = 200, description = "Album metadata and ordered tracklist", body = AlbumDetailsDto),
        (status = 404, description = "Album not found in cache or live catalog")
    ),
    security(
        ("bearer_auth" = [])
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
        return Err(ServerError::NotFound(format!(
            "Album '{id_or_name}' not found"
        )));
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
    tag = "catalog",
    summary = "Get Artist Tracks",
    description = "Returns all cached tracks for a specified artist.",
    params(
        ("name" = String, Path, description = "Artist name", example = "Taylor Swift")
    ),
    responses(
        (status = 200, description = "List of cached tracks by artist", body = Vec<TrackSummaryDto>)
    ),
    security(
        ("bearer_auth" = [])
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
