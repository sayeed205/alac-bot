use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{auth::AuthedUser, error::ServerError, ServerState};

#[derive(Debug, Deserialize, ToSchema)]
pub struct RipTaskRequest {
    pub provider: String,
    pub track_id: String,
    pub codec: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RipTaskResponse {
    pub task_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TaskProgressEvent {
    pub task_id: String,
    pub stage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_cached: Option<bool>,
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/tasks/rip",
    request_body = RipTaskRequest,
    responses(
        (status = 200, description = "Rip task queued", body = RipTaskResponse),
        (status = 400, description = "Invalid request payload"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_rip_task(
    State(state): State<Arc<ServerState>>,
    user: AuthedUser,
    Json(payload): Json<RipTaskRequest>,
) -> Result<Json<RipTaskResponse>, ServerError> {
    let provider = match payload.provider.to_lowercase().as_str() {
        "apple" => music::Provider::Apple,
        "qobuz" => music::Provider::Qobuz,
        _ => {
            return Err(ServerError::BadRequest(format!(
                "Unsupported provider: {}",
                payload.provider
            )))
        }
    };

    let codec = payload
        .codec
        .as_deref()
        .map(|c| match c.to_lowercase().as_str() {
            "alac" => music::Codec::Alac,
            "flac" => music::Codec::Flac,
            "aac" => music::Codec::Aac,
            _ => music::Codec::Alac,
        });

    let task_id = format!("task_{}", cuid2::create_id());

    // Emit initial event
    let initial_event = TaskProgressEvent {
        task_id: task_id.clone(),
        stage: "queued".to_string(),
        percent: Some(0.0),
        speed: None,
        track_id: None,
        is_cached: None,
        completed: false,
        error: None,
    };
    let _ = state.tasks_tx.send(initial_event);

    tracing::info!(
        task_id = %task_id,
        user_id = user.telegram_id,
        provider = ?provider,
        track_id = %payload.track_id,
        codec = ?codec,
        "Rip task submitted and dispatched"
    );

    // Dispatch background ripping via RipOrchestrator runner
    (state.rip_task_runner)(
        task_id.clone(),
        provider,
        payload.track_id.clone(),
        codec,
        user.telegram_id,
    );

    Ok(Json(RipTaskResponse {
        task_id,
        status: "queued".to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/tasks/{id}/events",
    params(
        ("id" = String, Path, description = "Task ID")
    ),
    responses(
        (status = 200, description = "Server-Sent Events progress stream", content_type = "text/event-stream")
    )
)]
pub async fn task_events(
    State(state): State<Arc<ServerState>>,
    Path(task_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.tasks_tx.subscribe();
    let target_id = task_id.clone();

    let stream = stream::unfold(
        (rx, target_id, false),
        |(mut rx, target_id, finished)| async move {
            if finished {
                return None;
            }
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event.task_id == target_id {
                            let is_done = event.completed || event.error.is_some();
                            let json = serde_json::to_string(&event).unwrap_or_default();
                            return Some((
                                Ok(Event::default().event("progress").data(json)),
                                (rx, target_id, is_done),
                            ));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(task_id = %target_id, skipped, "SSE receiver lagged, continuing");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return None;
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
