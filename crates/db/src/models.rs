use chrono::{DateTime, Utc};
use welds::prelude::*;

/// A Telegram user or chat granted access to the bot.
#[derive(Debug, Clone, Default, WeldsModel)]
#[welds(schema = "public", table = "users")]
pub struct User {
    #[welds(primary_key)]
    pub telegram_id: i64,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
}
