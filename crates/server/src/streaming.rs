use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, Method, StatusCode},
    response::Response,
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use base64::Engine;
use chrono::Utc;
use hmac::{digest::KeyInit, Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::{IntoParams, ToSchema};

use crate::{auth::AuthedUser, error::ServerError, ServerState};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamTicket {
    pub track_id: i32,
    pub user_id: i64,
    pub expires_at: i64,
}

impl StreamTicket {
    pub fn new(track_id: i32, user_id: i64, ttl_secs: i64) -> Self {
        Self {
            track_id,
            user_id,
            expires_at: Utc::now().timestamp() + ttl_secs,
        }
    }

    fn payload(&self) -> String {
        format!("{}:{}:{}", self.track_id, self.user_id, self.expires_at)
    }

    pub fn encode(&self, secret: &str) -> String {
        let payload = self.payload();
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(payload.as_bytes());
        let sig = mac.finalize().into_bytes();
        let full = format!("{payload}:{}", BASE64URL.encode(sig));
        BASE64URL.encode(full)
    }

    pub fn decode(secret: &str, ticket_str: &str) -> Result<Self, ServerError> {
        let decoded = BASE64URL
            .decode(ticket_str.trim())
            .map_err(|_| ServerError::Unauthorized("Invalid ticket encoding".into()))?;
        let ticket_payload = String::from_utf8(decoded)
            .map_err(|_| ServerError::Unauthorized("Invalid ticket format".into()))?;

        let parts: Vec<&str> = ticket_payload.split(':').collect();
        if parts.len() != 4 {
            return Err(ServerError::Unauthorized("Malformed stream ticket".into()));
        }

        let track_id: i32 = parts[0]
            .parse()
            .map_err(|_| ServerError::Unauthorized("Invalid track ID in ticket".into()))?;
        let user_id: i64 = parts[1]
            .parse()
            .map_err(|_| ServerError::Unauthorized("Invalid user ID in ticket".into()))?;
        let expires_at: i64 = parts[2]
            .parse()
            .map_err(|_| ServerError::Unauthorized("Invalid expiry in ticket".into()))?;
        let provided_sig = parts[3];

        if Utc::now().timestamp() > expires_at {
            return Err(ServerError::Unauthorized("Stream ticket has expired".into()));
        }

        let ticket = Self {
            track_id,
            user_id,
            expires_at,
        };
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(ticket.payload().as_bytes());

        let decoded_sig = BASE64URL
            .decode(provided_sig)
            .map_err(|_| ServerError::Unauthorized("Invalid ticket signature format".into()))?;

        mac.verify_slice(&decoded_sig)
            .map_err(|_| ServerError::Unauthorized("Invalid stream ticket signature".into()))?;

        Ok(ticket)
    }
}

pub fn create_stream_ticket(secret: &str, track_id: i32, user_id: i64, ttl_secs: i64) -> String {
    StreamTicket::new(track_id, user_id, ttl_secs).encode(secret)
}

pub fn verify_stream_ticket(secret: &str, ticket: &str) -> Result<(i32, i64), ServerError> {
    let t = StreamTicket::decode(secret, ticket)?;
    Ok((t.track_id, t.user_id))
}

pub use create_stream_ticket as create_playback_ticket;
pub use verify_stream_ticket as verify_playback_ticket;

#[derive(Debug, Serialize, ToSchema)]
pub struct PlaybackInfo {
    pub stream_url: String,
    pub expires_in: i64,
    pub mime_type: &'static str,
    pub codec: String,
    pub duration: i32,
    pub bit_depth: Option<i32>,
    pub sample_rate: Option<i32>,
    pub file_size: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/tracks/{id}/playback",
    params(
        ("id" = i32, Path, description = "Track ID")
    ),
    responses(
        (status = 200, description = "Playback metadata and stream ticket", body = PlaybackInfo),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Track not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn get_playback_info(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    axum::extract::Path(track_id): axum::extract::Path<i32>,
) -> Result<Json<PlaybackInfo>, ServerError> {
    let track = state
        .tracks_repo
        .find_track_by_id(track_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?
        .ok_or_else(|| ServerError::NotFound(format!("Track {track_id} not found")))?;

    let expires_in = 7200; // 2 hours
    let ticket =
        create_playback_ticket(&state.app_key, track_id, user.telegram_id, expires_in);
    let stream_url = format!("/api/v1/stream?ticket={ticket}");

    let file_size = match state.stream_engine.resolve_track_media(track_id, false).await {
        Ok(meta) => meta.file_size as i64,
        Err(_) => i64::from(track.duration) * 50_000,
    };

    Ok(Json(PlaybackInfo {
        stream_url,
        expires_in,
        mime_type: track.codec.mime_type(),
        codec: track.codec.as_str().to_string(),
        duration: track.duration,
        bit_depth: Some(track.bit_depth),
        sample_rate: Some(track.sample_rate),
        file_size,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct StreamQuery {
    pub ticket: Option<String>,
    pub track_id: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/api/v1/stream",
    params(
        StreamQuery
    ),
    responses(
        (status = 200, description = "Full audio stream"),
        (status = 206, description = "Partial content stream"),
        (status = 401, description = "Invalid or expired ticket"),
        (status = 404, description = "Track not found")
    )
)]
pub async fn stream_handler(
    State(state): State<Arc<ServerState>>,
    method: Method,
    headers: axum::http::HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Response, ServerError> {
    let track_id = if let Some(ref ticket) = query.ticket {
        let (id, _user_id) = verify_playback_ticket(&state.app_key, ticket)?;
        id
    } else if let Some(id) = query.track_id {
        let auth_header = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                ServerError::Unauthorized("Missing playback ticket or Bearer authorization".into())
            })?;
        let token = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
            ServerError::Unauthorized("Invalid Authorization format".into())
        })?;

        if state.token_cache.get(token).await.is_none() {
            let identity = state
                .session_mgr
                .verify_and_slide(token)
                .await
                .map_err(|e| ServerError::Unauthorized(e.to_string()))?;
            let authed_user = AuthedUser {
                telegram_id: identity.telegram_id,
                session_id: identity.session_id,
            };
            state.token_cache.insert(token.to_string(), authed_user).await;
        }
        id
    } else {
        return Err(ServerError::BadRequest(
            "Missing 'ticket' or 'track_id' query parameter".into(),
        ));
    };

    let range_header = headers
        .get(header::RANGE)
        .and_then(|h| h.to_str().ok());

    let response = state
        .stream_engine
        .open_stream(track_id, range_header)
        .await?;

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK))
        .header(header::CONTENT_TYPE, response.content_type)
        .header(header::ACCEPT_RANGES, response.accept_ranges)
        .header(header::CONTENT_LENGTH, response.content_length.to_string());

    if let Some(content_range) = response.content_range {
        builder = builder.header(header::CONTENT_RANGE, content_range);
    }

    if method == Method::HEAD {
        return builder
            .body(Body::empty())
            .map_err(|e| ServerError::Internal(e.to_string()));
    }

    builder
        .body(Body::from_stream(response.stream))
        .map_err(|e| ServerError::Internal(e.to_string()))
}
