//! Callback dispatcher for list pagination and rip cancellation.

use std::sync::Arc;

use ferogram::update::CallbackQuery;

use crate::{
    handlers::rip::{self, cancel},
    BotState,
};

pub async fn dispatch_dashboard(state: Arc<BotState>, query: CallbackQuery) {
    let Some(data) = query.data() else { return };
    let page = data
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<usize>().ok())
        .unwrap_or(1);
    let _ = query.answer().send(&state.client).await;
    crate::dashboard_manager()
        .page(
            query
                .chat_peer
                .map(|p| super::marked_peer_id(&p))
                .unwrap_or(query.user_id),
            page,
        )
        .await;
}

pub async fn dispatch_cancel(state: Arc<BotState>, query: CallbackQuery) {
    let Some(job_id) = query
        .data()
        .and_then(|d| d.strip_prefix("cancel:"))
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let caller = query.user_id;
    let admin = state.auth.is_admin(caller);
    let result = cancel::cancel_inline(
        &rip::active_jobs(),
        job_id,
        caller,
        admin,
        if admin { "Admin" } else { "User" },
    )
    .await;
    let (text, alert) = match result {
        cancel::CancelResult::Cancelled => (cancel::CALLBACK_ACK, false),
        cancel::CancelResult::Unauthorized => (cancel::CALLBACK_UNAUTHORIZED, true),
        cancel::CancelResult::Expired => (cancel::CALLBACK_EXPIRED, false),
    };
    let answer = if alert {
        query.answer().alert(text)
    } else {
        query.answer().text(text)
    };
    let _ = answer.send(&state.client).await;
}
