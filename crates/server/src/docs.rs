use axum::{
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::auth::exchange,
        crate::auth::refresh,
        crate::auth::logout,
        crate::auth::me,
        crate::streaming::get_playback_info,
        crate::streaming::stream_handler,
        crate::catalog::search_catalog,
        crate::catalog::get_track,
        crate::catalog::list_albums,
        crate::catalog::get_album_tracks,
        crate::catalog::get_artist_tracks,
        crate::tasks::create_rip_task,
        crate::tasks::task_events,
        crate::assets::get_artwork,
        crate::assets::get_lyrics,
        crate::library::list_favorites,
        crate::library::add_favorite,
        crate::library::remove_favorite,
        crate::library::list_playlists,
        crate::library::create_playlist,
        crate::library::get_playlist,
        crate::library::update_playlist,
        crate::library::delete_playlist,
    ),
    components(
        schemas(
            crate::auth::ExchangeRequest,
            crate::auth::ExchangeResponse,
            crate::auth::RefreshRequest,
            crate::auth::RefreshResponse,
            crate::auth::LogoutRequest,
            crate::auth::UserDto,
            crate::auth::SessionDto,
            crate::auth::MeResponse,
            crate::streaming::PlaybackInfo,
            crate::catalog::TrackSummaryDto,
            crate::catalog::TrackDetailDto,
            crate::catalog::UncachedTrackDto,
            crate::catalog::SearchResponse,
            crate::catalog::AlbumSummaryDto,
            crate::catalog::AlbumDetailsDto,
            crate::tasks::RipTaskRequest,
            crate::tasks::RipTaskResponse,
            crate::tasks::TaskProgressEvent,
            crate::assets::LyricsResponse,
            crate::assets::LyricsLineDto,
            crate::assets::LyricsWordDto,
            crate::library::FavoritesResponse,
            crate::library::PlaylistSummaryDto,
            crate::library::CreatePlaylistRequest,
            crate::library::PlaylistWithTracksDto,
            crate::library::UpdatePlaylistRequest,
        )
    ),
    tags(
        (name = "auth", description = "Telegram-gated authentication"),
        (name = "stream", description = "Lossless audio streaming"),
        (name = "catalog", description = "Music catalog search and discovery"),
        (name = "tasks", description = "On-demand rip tasks and SSE progress"),
        (name = "assets", description = "Artwork and lyrics"),
        (name = "library", description = "User favorites and playlists"),
    )
)]
pub struct ApiDoc;

const SCALAR_HTML: &str = r#"<!doctype html>
<html>
  <head>
    <title>ALAC Streaming Server API Reference</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
      body {
        margin: 0;
      }
    </style>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="/api/v1/docs.json"
      data-proxy-url=""
    ></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>
"#;

pub async fn scalar_html() -> Html<&'static str> {
    Html(SCALAR_HTML)
}

pub async fn openapi_json() -> Response {
    let doc = ApiDoc::openapi();
    match doc.to_json() {
        Ok(json) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize OpenAPI json: {e}"),
        )
            .into_response(),
    }
}

pub async fn openapi_yaml() -> Response {
    let doc = ApiDoc::openapi();
    match doc.to_yaml() {
        Ok(yaml) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/yaml")],
            yaml,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize OpenAPI yaml: {e}"),
        )
            .into_response(),
    }
}
