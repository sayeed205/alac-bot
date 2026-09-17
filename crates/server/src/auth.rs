use std::sync::Arc;

use axum::{
    extract::{FromRef, FromRequestParts, State},
    http::request::Parts,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{error::ServerError, ServerState};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuthedUser {
    pub telegram_id: i64,
    pub session_id: String,
}

impl<S> FromRequestParts<S> for AuthedUser
where
    S: Send + Sync,
    Arc<ServerState>: FromRef<S>,
{
    type Rejection = ServerError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let server_state = Arc::<ServerState>::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ServerError::Unauthorized("Missing Authorization header".into()))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                ServerError::Unauthorized(
                    "Invalid Authorization format, expected 'Bearer <token>'".into(),
                )
            })?
            .trim();

        if let Some(user) = server_state.token_cache.get(token).await {
            return Ok(user);
        }

        let identity = server_state
            .session_mgr
            .verify_and_slide(token)
            .await
            .map_err(|e| ServerError::Unauthorized(e.to_string()))?;

        let user = AuthedUser {
            telegram_id: identity.telegram_id,
            session_id: identity.session_id,
        };

        server_state
            .token_cache
            .insert(token.to_string(), user.clone())
            .await;

        Ok(user)
    }
}

pub struct MaybeAuthedUser(pub Option<AuthedUser>);

impl<S> FromRequestParts<S> for MaybeAuthedUser
where
    Arc<ServerState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match AuthedUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeAuthedUser(Some(user))),
            Err(_) => Ok(MaybeAuthedUser(None)),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExchangeRequest {
    pub code: String,
    pub device_name: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserDto {
    pub telegram_id: i64,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExchangeResponse {
    pub token_type: &'static str,
    pub token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub expires_at: DateTime<Utc>,
    pub expires_at_unix: i64,
    pub user: UserDto,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub token: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshResponse {
    pub token_type: &'static str,
    pub token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub expires_at: DateTime<Utc>,
    pub expires_at_unix: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LogoutRequest {
    pub token: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionDto {
    pub id: String,
    pub device_name: Option<String>,
    pub platform: Option<String>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub user: UserDto,
    pub sessions: Vec<SessionDto>,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/exchange",
    request_body = ExchangeRequest,
    responses(
        (status = 200, description = "Code exchanged for access token", body = ExchangeResponse),
        (status = 401, description = "Invalid or expired OTP code")
    )
)]
pub async fn exchange(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<ExchangeRequest>,
) -> Result<Json<ExchangeResponse>, ServerError> {
    let metadata = db::ClientMetadata {
        device_name: payload.device_name.as_deref(),
        platform: payload.platform.as_deref(),
    };

    let session = state
        .session_mgr
        .exchange_code(&payload.code, metadata)
        .await
        .map_err(|e| ServerError::Unauthorized(e.to_string()))?;

    let authed_user = AuthedUser {
        telegram_id: session.telegram_id,
        session_id: session.session_id,
    };

    state
        .token_cache
        .insert(session.refresh_token.clone(), authed_user)
        .await;

    let name = state
        .session_mgr
        .get_user_name(session.telegram_id)
        .await
        .ok()
        .flatten();

    let expires_at_unix = session.expires_at.timestamp();

    Ok(Json(ExchangeResponse {
        token_type: "Bearer",
        token: session.refresh_token.clone(),
        access_token: session.refresh_token.clone(),
        refresh_token: session.refresh_token,
        expires_in: 259200,
        expires_at: session.expires_at,
        expires_at_unix,
        user: UserDto {
            telegram_id: session.telegram_id,
            name,
        },
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token renewed and slid forward", body = RefreshResponse),
        (status = 401, description = "Invalid, expired, or revoked token")
    )
)]
pub async fn refresh(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, ServerError> {
    let token = payload
        .refresh_token
        .or(payload.token)
        .ok_or_else(|| ServerError::BadRequest("Missing token or refresh_token".into()))?;

    let identity = state
        .session_mgr
        .verify_and_slide(&token)
        .await
        .map_err(|e| ServerError::Unauthorized(e.to_string()))?;

    let authed_user = AuthedUser {
        telegram_id: identity.telegram_id,
        session_id: identity.session_id,
    };

    state
        .token_cache
        .insert(token.clone(), authed_user)
        .await;

    let expires_at_unix = identity.expires_at.timestamp();

    Ok(Json(RefreshResponse {
        token_type: "Bearer",
        token: token.clone(),
        access_token: token.clone(),
        refresh_token: token,
        expires_in: 259200,
        expires_at: identity.expires_at,
        expires_at_unix,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    request_body = LogoutRequest,
    responses(
        (status = 200, description = "Session successfully revoked"),
        (status = 401, description = "Missing token or authorization")
    )
)]
pub async fn logout(
    State(state): State<Arc<ServerState>>,
    MaybeAuthedUser(maybe_user): MaybeAuthedUser,
    Json(payload): Json<LogoutRequest>,
) -> Result<Json<serde_json::Value>, ServerError> {
    let token_to_revoke = payload.refresh_token.or(payload.token);
    if let Some(token) = token_to_revoke {
        state.token_cache.remove(&token).await;
        let _ = state.session_mgr.revoke(&token).await;
        Ok(Json(serde_json::json!({ "revoked": true })))
    } else if let Some(user) = maybe_user {
        // Revoke all sessions for current user if no specific token given
        state.token_cache.invalidate_all();
        let _ = state.session_mgr.revoke_all_for_user(user.telegram_id).await;
        Ok(Json(serde_json::json!({ "revoked": true })))
    } else {
        Err(ServerError::Unauthorized(
            "Missing refresh_token or Bearer authorization header".into(),
        ))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user profile", body = MeResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn me(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
) -> Result<Json<MeResponse>, ServerError> {
    let db_sessions = state
        .session_mgr
        .list_active_sessions(user.telegram_id)
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    let sessions = db_sessions
        .into_iter()
        .map(|s| SessionDto {
            id: s.id,
            device_name: s.device_name,
            platform: s.platform,
            last_active_at: s.last_active_at,
            expires_at: s.expires_at,
        })
        .collect();

    let name = state
        .session_mgr
        .get_user_name(user.telegram_id)
        .await
        .ok()
        .flatten();

    Ok(Json(MeResponse {
        user: UserDto {
            telegram_id: user.telegram_id,
            name,
        },
        sessions,
    }))
}
