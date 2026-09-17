use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{auth::AuthedUser, catalog::TrackSummaryDto, error::ServerError, ServerState};

#[derive(Debug, Serialize, ToSchema)]
pub struct FavoritesResponse {
    pub track_ids: Vec<i32>,
}

#[utoipa::path(
    get,
    path = "/api/v1/me/favorites",
    responses(
        (status = 200, description = "List of user favorited track IDs", body = FavoritesResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_favorites(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
) -> Result<Json<FavoritesResponse>, ServerError> {
    let ids = state
        .library_mgr
        .list_favorite_ids(user.telegram_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(Json(FavoritesResponse { track_ids: ids }))
}

#[utoipa::path(
    post,
    path = "/api/v1/me/favorites/{track_id}",
    params(
        ("track_id" = i32, Path, description = "Track ID to bookmark")
    ),
    responses(
        (status = 200, description = "Track added to favorites"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn add_favorite(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Path(track_id): Path<i32>,
) -> Result<Json<serde_json::Value>, ServerError> {
    let added = state
        .library_mgr
        .add_favorite(user.telegram_id, track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "added": added })))
}

#[utoipa::path(
    delete,
    path = "/api/v1/me/favorites/{track_id}",
    params(
        ("track_id" = i32, Path, description = "Track ID to remove from bookmarks")
    ),
    responses(
        (status = 200, description = "Track removed from favorites"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn remove_favorite(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Path(track_id): Path<i32>,
) -> Result<Json<serde_json::Value>, ServerError> {
    let removed = state
        .library_mgr
        .remove_favorite(user.telegram_id, track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlaylistSummaryDto {
    pub id: i32,
    pub name: String,
    pub track_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<db::UserPlaylistSummary> for PlaylistSummaryDto {
    fn from(p: db::UserPlaylistSummary) -> Self {
        Self {
            id: p.id,
            name: p.name,
            track_count: p.track_count,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlaylistWithTracksDto {
    pub id: i32,
    pub name: String,
    pub tracks: Vec<TrackSummaryDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub track_ids: Vec<i32>,
}

#[utoipa::path(
    get,
    path = "/api/v1/me/playlists",
    responses(
        (status = 200, description = "List of user playlists", body = Vec<PlaylistSummaryDto>),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn list_playlists(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
) -> Result<Json<Vec<PlaylistSummaryDto>>, ServerError> {
    let playlists = state
        .library_mgr
        .list_playlists(user.telegram_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    Ok(Json(playlists.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/me/playlists",
    request_body = CreatePlaylistRequest,
    responses(
        (status = 200, description = "Playlist created", body = PlaylistSummaryDto),
        (status = 400, description = "Invalid name"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_playlist(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Json(payload): Json<CreatePlaylistRequest>,
) -> Result<Json<PlaylistSummaryDto>, ServerError> {
    let p = state
        .library_mgr
        .create_playlist(user.telegram_id, &payload.name)
        .await?;

    Ok(Json(PlaylistSummaryDto {
        id: p.id,
        name: p.name,
        track_count: 0,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/me/playlists/{id}",
    params(
        ("id" = i32, Path, description = "Playlist ID")
    ),
    responses(
        (status = 200, description = "Playlist details with tracks", body = PlaylistWithTracksDto),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Playlist not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_playlist(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Path(playlist_id): Path<i32>,
) -> Result<Json<PlaylistWithTracksDto>, ServerError> {
    let details = state
        .library_mgr
        .get_playlist(user.telegram_id, playlist_id)
        .await?;

    let tracks = details.tracks.into_iter().map(Into::into).collect();

    Ok(Json(PlaylistWithTracksDto {
        id: details.playlist.id,
        name: details.playlist.name,
        tracks,
        created_at: details.playlist.created_at,
        updated_at: details.playlist.updated_at,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/me/playlists/{id}",
    params(
        ("id" = i32, Path, description = "Playlist ID")
    ),
    request_body = UpdatePlaylistRequest,
    responses(
        (status = 200, description = "Playlist reordered"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Playlist not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn update_playlist(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Path(playlist_id): Path<i32>,
    Json(payload): Json<UpdatePlaylistRequest>,
) -> Result<Json<serde_json::Value>, ServerError> {
    state
        .library_mgr
        .reorder_playlist(user.telegram_id, playlist_id, &payload.track_ids)
        .await?;

    Ok(Json(serde_json::json!({ "updated": true })))
}

#[utoipa::path(
    delete,
    path = "/api/v1/me/playlists/{id}",
    params(
        ("id" = i32, Path, description = "Playlist ID")
    ),
    responses(
        (status = 200, description = "Playlist deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Playlist not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn delete_playlist(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Path(playlist_id): Path<i32>,
) -> Result<Json<serde_json::Value>, ServerError> {
    let deleted = state
        .library_mgr
        .delete_playlist(user.telegram_id, playlist_id)
        .await?;

    Ok(Json(serde_json::json!({ "deleted": deleted })))
}
