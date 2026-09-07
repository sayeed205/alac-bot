use chrono::{DateTime, Utc};
use welds::connections::{Client, Param};

use crate::{DbError, User};

/// An authorized Telegram peer as returned by [`Auth::list_authorized`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthedPeer {
    pub telegram_id: i64,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Authorization service backed by the users table.
#[derive(Clone)]
pub struct Auth {
    client: welds::connections::postgres::PostgresClient,
    admin_id: i64,
}

impl Auth {
    pub fn new(client: welds::connections::postgres::PostgresClient, admin_id: i64) -> Self {
        Self { client, admin_id }
    }

    pub fn is_admin(&self, user_id: i64) -> bool {
        user_id == self.admin_id
    }

    pub async fn is_authorized(&self, user_id: i64, chat_id: Option<i64>) -> Result<bool, DbError> {
        if self.is_admin(user_id) {
            return Ok(true);
        }

        let user_id_param = user_id;
        let rows = if let Some(chat_id) = chat_id.filter(|id| *id != user_id) {
            let chat_id_param = chat_id;
            self.client
                .fetch_rows(
                    "SELECT telegram_id FROM users WHERE telegram_id = $1 OR telegram_id = $2 LIMIT 1",
                    &[&user_id_param, &chat_id_param],
                )
                .await?
        } else {
            self.client
                .fetch_rows(
                    "SELECT telegram_id FROM users WHERE telegram_id = $1 LIMIT 1",
                    &[&user_id_param],
                )
                .await?
        };
        Ok(!rows.is_empty())
    }

    pub async fn authorize(&self, telegram_id: i64, name: Option<&str>) -> Result<bool, DbError> {
        let existing = self
            .client
            .fetch_rows(
                "SELECT telegram_id FROM users WHERE telegram_id = $1 LIMIT 1",
                &[&telegram_id],
            )
            .await?;
        let name_param = name.map(str::to_owned);
        self.client
            .execute(
                "INSERT INTO users (telegram_id, name) VALUES ($1, $2) ON CONFLICT (telegram_id) DO UPDATE SET name = EXCLUDED.name",
                &[
                    &telegram_id as &(dyn Param + Sync),
                    &name_param as &(dyn Param + Sync),
                ],
            )
            .await?;
        Ok(existing.is_empty())
    }

    pub async fn revoke(&self, telegram_id: i64) -> Result<bool, DbError> {
        let result = self
            .client
            .execute("DELETE FROM users WHERE telegram_id = $1", &[&telegram_id])
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_authorized(&self) -> Result<Vec<AuthedPeer>, DbError> {
        let rows = self
            .client
            .fetch_rows(
                "SELECT telegram_id, name, created_at FROM users ORDER BY created_at",
                &[],
            )
            .await?;
        rows.into_iter()
            .map(|row| {
                Ok(AuthedPeer {
                    telegram_id: row
                        .get("telegram_id")
                        .map_err(|e| DbError::Row(e.to_string()))?,
                    name: row.get("name").map_err(|e| DbError::Row(e.to_string()))?,
                    created_at: row
                        .get("created_at")
                        .map_err(|e| DbError::Row(e.to_string()))?,
                })
            })
            .collect()
    }

    /// Expose the connection for the bot's migration/bootstrap boundary.
    pub fn client(&self) -> &welds::connections::postgres::PostgresClient {
        &self.client
    }
}

// Keep the model imported in this module so the database model remains part
// of the public API and welds' generated implementations are type-checked.
#[allow(dead_code)]
fn _model_type_check(_: User) {}
