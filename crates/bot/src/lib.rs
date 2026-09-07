//! Telegram handlers and small formatting helpers.

pub mod handlers;
pub mod html;

use std::sync::Arc;

#[derive(Clone)]
pub struct BotState {
    pub client: ferogram::Client,
    pub auth: db::Auth,
}

pub type SharedState = Arc<BotState>;
