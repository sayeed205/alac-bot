//! Live `/alac` command policy and pipeline entry point.
//!
//! M5c: the bot owns only preflight policy and input parsing. Status is
//! rendered by one shared dashboard message per chat. All rip semantics —
//! cache-first maintenance, queue position, retries, circuit breaker — live
//! in the engine orchestrator (`commands-rip.ts` is the parity oracle).

pub mod cancel;
pub mod gates;
pub mod input;
pub mod status;

use std::sync::Arc;

use engine::orchestrator::deps::OrchestratorDeps;
use ferogram::{
    filters::{self, Dispatcher},
    InputMessage,
};

use crate::BotState;

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    for alias in [
        "alac", "rip", "batch", "dl", "download", "rerip", "cache", "dump",
    ] {
        let state = Arc::clone(&state);
        dp.on_message(filters::command(alias), move |msg| {
            let state = Arc::clone(&state);
            async move { handle_command(state, msg).await }
        });
    }
    let state = Arc::clone(&state);
    dp.on_message(filters::command("cancel"), move |msg| {
        let state = Arc::clone(&state);
        async move { handle_cancel_command(state, msg).await }
    });
}

async fn handle_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let sender = msg.sender_user_id().unwrap_or_default();
    let chat = super::marked_chat_id(&msg);
    if !state
        .auth
        .is_authorized(sender, Some(chat))
        .await
        .unwrap_or(false)
    {
        return;
    }

    let command = input::command_name(msg.text().unwrap_or_default()).unwrap_or_default();
    let is_cache = command == "cache" || command == "dump";
    let admin = state.auth.is_admin(sender);

    // Preflight gates (oracle commands-rip.ts:216-307).
    if let Some(text) = gates::cache_gate(is_cache, admin) {
        reply(&msg, text).await;
        return;
    }
    let parsed = input::parse_message(&state.client, &msg, command == "rerip").await;
    if parsed.items.is_empty() {
        reply(&msg, gates::usage(is_cache)).await;
        return;
    }
    if let Some(text) = gates::force_gate(parsed.force, admin) {
        reply(&msg, text).await;
        return;
    }
    let settings = state.rip_deps.get_settings().await;
    if let Some(text) = gates::feature_gate(&settings, &parsed.items, admin, parsed.document) {
        reply(&msg, text).await;
        return;
    }

    // Requester display name (oracle: displayName || sender.displayName || @username || User id).
    let user = msg.sender_user().await.ok().flatten();
    let display_name = user
        .as_ref()
        .map(|u| {
            let first = u.first_name().unwrap_or_default().trim();
            match u.last_name().map(str::trim).filter(|l| !l.is_empty()) {
                Some(last) if !first.is_empty() => format!("{first} {last}"),
                Some(last) => last.to_owned(),
                None if !first.is_empty() => first.to_owned(),
                None => String::new(),
            }
        })
        .filter(|name| !name.is_empty())
        .or_else(|| {
            user.as_ref()
                .and_then(|u| u.username().map(|n| format!("@{n}")))
        })
        .unwrap_or_else(|| format!("User {sender}"));

    let is_group = chat != sender;

    // Group DM preflight (oracle commands-rip.ts:471-519): for non-cache
    // group requests, verify the requester can receive DMs before queueing
    // anything. A failure prompts them to start the bot in DM and stops the
    // job; on success delivery is retargeted to the DM.
    let mut delivery_chat_id = chat;
    if is_group && !is_cache {
        let note = InputMessage::html(format!(
            "<b>Download queued</b><br/>Tracks requested in <b>{}</b> will be delivered to your private chat.",
            crate::html::escape(&display_name)
        ))
        .silent(true);
        match state
            .client
            .send_message(ferogram::PeerRef::from(sender), note)
            .await
        {
            Ok(_) => delivery_chat_id = sender,
            Err(_) => {
                let bot_username = state
                    .client
                    .get_me()
                    .await
                    .ok()
                    .and_then(|me| me.username)
                    .unwrap_or_else(|| "alac_bot".to_owned());
                let keyboard = ferogram::keyboard::InlineKeyboard::new()
                    .row([ferogram::keyboard::Button::url(
                        "Start bot in private chat",
                        format!("https://t.me/{bot_username}?start=start"),
                    )])
                    .into_markup();
                let text = "! <b>Private chat required</b><br/><br/>Audio files are delivered to your private chat to keep this group clean.<br/>Start the bot in private chat, then send your request again.";
                let _ = msg
                    .reply(InputMessage::html(text).reply_markup(keyboard))
                    .await;
                return;
            }
        }
    }

    // Keep one status dashboard message per chat. New jobs are added to the
    // shared snapshot by the engine's Created event; no per-job progress
    // message is sent.
    super::ensure_dashboard(&state, chat, sender, admin, super::chat_peer_ref(&msg)).await;

    let options = engine::orchestrator::types::RipJobOptions {
        chat_id: chat,
        user_id: sender,
        user_name: Some(display_name),
        delivery_chat_id,
        is_group,
        is_force: parsed.force,
        is_cache_only: is_cache,
        single_storefront: parsed.storefront,
        parsed_items: parsed.items,
        reply_to_message_id: Some(i64::from(msg.id())),
        // Status is rendered by the chat dashboard rather than a per-job
        // message. The zero sentinel keeps the engine type stable for other
        // orchestration callers.
        status_msg_id: 0,
        is_admin: admin,
    };

    // Engine owns everything from here: resolution, cache-first, queue,
    // pipeline, and terminal events. The bridge refreshes the dashboard;
    // start_job errors are logged only.
    if let Err(error) = state
        .rip_orchestrator
        .start_job(Arc::clone(&state.rip_deps), &options)
        .await
    {
        tracing::error!(error = %error, "rip job failed");
    }
}

async fn handle_cancel_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let caller = msg.sender_user_id().unwrap_or_default();
    let admin = state.auth.is_admin(caller);
    let name = if admin { "Admin" } else { "User" };
    if cancel::cancel_command(&state, super::marked_chat_id(&msg), caller, admin, name).await {
        reply(&msg, cancel::COMMAND_ACK).await;
    } else {
        reply(&msg, cancel::NO_ACTIVE).await;
    }
}

async fn reply(msg: &ferogram::update::IncomingMessage, text: &str) {
    let _ = msg.reply(InputMessage::html(text)).await;
}
