use std::borrow::Borrow;

use engine::orchestrator::deps::RequestLog;
use welds::connections::{Client, Param};

use crate::DbError;

/// Append-only request log repository.
#[derive(Clone)]
pub struct RequestLogRepository {
    client: welds::connections::postgres::PostgresClient,
}

impl RequestLogRepository {
    pub fn new(client: welds::connections::postgres::PostgresClient) -> Self {
        Self { client }
    }

    pub async fn log_request<I>(&self, data: I) -> Result<(), DbError>
    where
        I: Borrow<RequestLog>,
    {
        let data = data.borrow();
        let duration_ms = data
            .duration_ms
            .map(|value| i32::try_from(value).map_err(|error| DbError::Row(error.to_string())))
            .transpose()?;
        let params: [&(dyn Param + Sync); 7] = [
            &data.telegram_id,
            &data.chat_id,
            &data.apple_track_id,
            &data.is_cache_hit,
            &duration_ms,
            &data.status,
            &data.error_reason,
        ];
        self.client
            .execute(
                "INSERT INTO requests (telegram_id, chat_id, apple_track_id, is_cache_hit, duration_ms, status, error_reason) VALUES ($1,$2,$3,$4,$5,$6,$7)",
                &params,
            )
            .await?;
        Ok(())
    }
}
