//! Telegram handlers and small formatting helpers.

pub mod handlers;
pub mod html;
pub mod rip_deps;
pub mod telegram_sink;

use std::sync::Arc;

#[derive(Clone)]
pub struct BotState {
    pub client: ferogram::Client,
    pub auth: db::Auth,
    pub rip_deps: Arc<rip_deps::RipDeps>,
    pub rip_queue: engine::queue::SequentialRipQueue,
}

pub type SharedState = Arc<BotState>;
