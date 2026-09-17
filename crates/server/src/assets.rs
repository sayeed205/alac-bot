use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{error::ServerError, ServerState};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ArtworkQuery {
    pub size: Option<u16>,
}

#[utoipa::path(
    get,
    path = "/api/v1/assets/tracks/{id}/artwork",
    params(
        ("id" = i32, Path, description = "Track ID"),
        ArtworkQuery
    ),
    responses(
        (status = 307, description = "Redirect to high-resolution album artwork"),
        (status = 404, description = "Track or artwork not found")
    )
)]
pub async fn get_artwork(
    State(state): State<Arc<ServerState>>,
    Path(track_id): Path<i32>,
    Query(query): Query<ArtworkQuery>,
) -> Result<Response, ServerError> {
    let track = state
        .tracks_repo
        .find_track_by_id(track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .ok_or_else(|| ServerError::NotFound(format!("Track {track_id} not found")))?;

    let size = query.size.unwrap_or(600).clamp(100, 3000);

    // If track is from Apple or Qobuz, try resolving CDN artwork
    // Apple Music artwork URLs follow standard format or catalog lookup
    let artwork_url = match track.provider {
        music::Provider::Apple => {
            let fallback_url = format!(
                "https://is1-ssl.mzstatic.com/image/thumb/Music/{size}x{size}bb.jpg"
            );
            let lookup_url = format!(
                "https://itunes.apple.com/lookup?id={}&entity=song",
                track.track_id
            );
            let resolved_url = match state.http_client.get(&lookup_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(json) = resp.json::<serde_json::Value>().await {
                        json.get("results")
                            .and_then(|r| r.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|item| item.get("artworkUrl100"))
                            .and_then(|url| url.as_str())
                            .map(|url| url.replace("100x100bb", &format!("{size}x{size}bb")))
                            .unwrap_or(fallback_url)
                    } else {
                        fallback_url
                    }
                }
                _ => fallback_url,
            };
            Some(resolved_url)
        }
        music::Provider::Qobuz => {
            Some(format!(
                "https://static.qobuz.com/images/covers/{}_{size}.jpg",
                track.track_id
            ))
        }
    };

    if let Some(url) = artwork_url {
        Ok(Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(header::LOCATION, url)
            .header(header::CACHE_CONTROL, "public, max-age=86400")
            .body(axum::body::Body::empty())
            .map_err(|e| ServerError::Internal(e.to_string()))?)
    } else {
        Err(ServerError::NotFound("Artwork not found".into()))
    }
}

struct ReqwestLyricsHttp(reqwest::Client);

impl lyrics::LyricsHttp for ReqwestLyricsHttp {
    fn get_json<'a>(&'a self, url: &'a str) -> lyrics::LyricsFuture<'a, Option<String>> {
        Box::pin(async move {
            let res = self
                .0
                .get(url)
                .header("User-Agent", "AlacBot/1.0")
                .send()
                .await
                .ok()?;
            if !res.status().is_success() {
                return None;
            }
            res.text().await.ok()
        })
    }
}

fn parse_lrc_timestamp(tag: &str) -> Option<i64> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() == 2 {
        let mins: i64 = parts[0].parse().ok()?;
        let secs: f64 = parts[1].parse().ok()?;
        Some((mins * 60_000) + (secs * 1000.0) as i64)
    } else {
        None
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LyricsWordDto {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LyricsLineDto {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub words: Vec<LyricsWordDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LyricsResponse {
    pub track_id: i32,
    pub format: String,
    pub plain_text: Option<String>,
    pub lines: Vec<LyricsLineDto>,
}

#[utoipa::path(
    get,
    path = "/api/v1/assets/tracks/{id}/lyrics",
    params(
        ("id" = i32, Path, description = "Track ID")
    ),
    responses(
        (status = 200, description = "Synced or plain lyrics for the track", body = LyricsResponse),
        (status = 404, description = "Lyrics not found")
    )
)]
pub async fn get_lyrics(
    State(state): State<Arc<ServerState>>,
    Path(track_id): Path<i32>,
) -> Result<Json<LyricsResponse>, ServerError> {
    let track = state
        .tracks_repo
        .find_track_by_id(track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .ok_or_else(|| ServerError::NotFound(format!("Track {track_id} not found")))?;

    let http = ReqwestLyricsHttp(state.http_client.clone());
    let mut lookup = lyrics::LyricsLookup::new(&track.title, [&track.artist])
        .with_album(&track.album)
        .with_duration(track.duration as i64);
    if track.provider == music::Provider::Apple {
        lookup = lookup.with_provider_id("apple", &track.track_id);
    }
    let registry = lyrics::LyricsRegistry::all_sources();
    let candidates = lyrics::lookup_ranked(&http, &registry, &lookup).await;

    if let Some(best) = candidates.into_iter().next() {
        let format_str = match best.document.format {
            lyrics::LyricsFormat::Elrc | lyrics::LyricsFormat::Lrc => "ttml_synced",
            lyrics::LyricsFormat::Plain => "plain",
        }
        .to_string();

        let mut lines = Vec::new();
        if let Some(timed_lines) = best.document.word_timing {
            for line in timed_lines {
                let start_ms = line.start_ms as i64;
                let text = line
                    .words
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let words = line
                    .words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        let next_start = line
                            .words
                            .get(i + 1)
                            .map(|nw| nw.start_ms as i64)
                            .unwrap_or(w.start_ms as i64 + 400);
                        LyricsWordDto {
                            text: w.text.clone(),
                            start_ms: w.start_ms as i64,
                            end_ms: next_start,
                        }
                    })
                    .collect();
                let end_ms = line
                    .words
                    .last()
                    .map(|w| w.start_ms as i64 + 500)
                    .unwrap_or(start_ms + 3000);
                lines.push(LyricsLineDto {
                    text,
                    start_ms,
                    end_ms,
                    words,
                });
            }
        } else {
            for line_str in best.document.text.lines() {
                let trimmed = line_str.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.starts_with('[') {
                    if let Some(idx) = trimmed.find(']') {
                        let tag = &trimmed[1..idx];
                        let content = trimmed[idx + 1..].trim();
                        let start_ms = parse_lrc_timestamp(tag).unwrap_or(0);
                        lines.push(LyricsLineDto {
                            text: content.to_string(),
                            start_ms,
                            end_ms: start_ms + 3000,
                            words: Vec::new(),
                        });
                        continue;
                    }
                }
                lines.push(LyricsLineDto {
                    text: trimmed.to_string(),
                    start_ms: 0,
                    end_ms: 0,
                    words: Vec::new(),
                });
            }
            for i in 0..lines.len() {
                if i + 1 < lines.len() && lines[i + 1].start_ms > lines[i].start_ms {
                    lines[i].end_ms = lines[i + 1].start_ms;
                }
            }
        }

        return Ok(Json(LyricsResponse {
            track_id,
            format: format_str,
            plain_text: Some(best.document.text),
            lines,
        }));
    }

    Ok(Json(LyricsResponse {
        track_id,
        format: "plain".to_string(),
        plain_text: Some(format!("{} - {}", track.title, track.artist)),
        lines: vec![LyricsLineDto {
            text: format!("{} - {}", track.title, track.artist),
            start_ms: 0,
            end_ms: (track.duration as i64) * 1000,
            words: Vec::new(),
        }],
    }))
}
