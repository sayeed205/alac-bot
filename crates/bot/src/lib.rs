//! Telegram handlers and small formatting helpers.

pub mod dashboard;
pub mod handlers;
pub mod html;
pub mod rip_deps;
pub mod telegram_sink;

use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct BotState {
    pub client: ferogram::Client,
    pub auth: db::Auth,
    pub rip_deps: Arc<rip_deps::RipDeps>,
    pub rip_queue: engine::queue::SequentialRipQueue,
}

pub type SharedState = Arc<BotState>;

static DASHBOARD: OnceLock<Arc<dashboard::DashboardManager>> = OnceLock::new();
pub fn dashboard_manager() -> Arc<dashboard::DashboardManager> {
    DASHBOARD
        .get_or_init(|| Arc::new(dashboard::DashboardManager::new()))
        .clone()
}
