//! Callback dispatcher for list pagination, dashboard paging, and rip
//! cancellation.

use std::sync::Arc;

use ferogram::update::CallbackQuery;

use crate::{handlers::rip::cancel, BotState};

pub async fn dispatch_dashboard(state: Arc<BotState>, query: CallbackQuery) {
    let Some(data) = query.data().map(str::to_owned) else {
        return;
    };
    let chat = query
        .chat_peer
        .as_ref()
        .map(super::marked_peer_id)
        .unwrap_or(query.user_id);

    // Buttons are `dashboard:{action}:{current_page}` — the data carries the
    // page the dashboard was rendered on, and the action determines the
    // target page.
    let Some(payload) = data.strip_prefix("dashboard:") else {
        return;
    };
    let mut parts = payload.split(':');
    let action = parts.next().unwrap_or("refresh").to_owned();
    let current = parts
        .next()
        .and_then(|p| p.parse::<usize>().ok())
        .unwrap_or(1);
    let target = match action.as_str() {
        "prev" => current.saturating_sub(1),
        "next" => current + 1,
        _ => current,
    };

    if action == "refresh" {
        // Re-pull the latest engine snapshot rather than trusting the
        // message-local copy.
        let snapshot = crate::event_bridge::current_snapshot(&state).await;
        crate::dashboard_manager()
            .refresh_entry_from(chat, snapshot)
            .await;
    } else {
        crate::dashboard_manager().page(chat, target).await;
    }

    let _ = query.answer().send(&state.client).await;
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
    let result = cancel::cancel_inline(&state, job_id, caller, admin);
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

    // Refresh dashboards so the row disappears promptly.
    let snapshot = crate::event_bridge::current_snapshot(&state).await;
    crate::dashboard_manager().refresh(snapshot, true).await;
}
