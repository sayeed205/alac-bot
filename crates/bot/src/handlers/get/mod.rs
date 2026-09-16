//! `/get` command policy and pipeline entry point.
//!
//! M5c: the bot owns only preflight policy and input parsing. Status is
//! rendered by one shared dashboard message per chat. All download semantics —
//! cache-first maintenance, queue position, retries, circuit breaker — live
//! in the engine orchestrator.

pub mod cancel;
pub mod gates;
pub mod input;

use std::sync::Arc;

use ferogram::{
    filters::{self, Dispatcher},
    InputMessage,
};

use crate::BotState;

pub fn register(dp: &mut Dispatcher, state: Arc<BotState>) {
    let get_state = Arc::clone(&state);
    dp.on_message(filters::command("get"), move |msg| {
        let state = Arc::clone(&get_state);
        async move { handle_command(state, msg).await }
    });
    let cancel_state = Arc::clone(&state);
    dp.on_message(
        filters::custom(|msg| {
            msg.text().is_some_and(|t| {
                let cmd = t.split_whitespace().next().unwrap_or("");
                let base = cmd.split('@').next().unwrap_or(cmd);
                base.starts_with("/cancel_") && base.len() > "/cancel_".len()
            })
        }),
        move |msg| {
            let state = Arc::clone(&cancel_state);
            async move { handle_cancel_id_command(state, msg).await }
        },
    );
}

async fn handle_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let caller_id = msg.sender_user_id().unwrap_or_default();
    let chat = super::marked_chat_id(&msg);
    if !state
        .auth
        .is_authorized(caller_id, Some(chat))
        .await
        .unwrap_or(false)
    {
        return;
    }

    let caller_is_admin = state.auth.is_admin(caller_id);

    let parsed = input::parse_message(&state.client, &msg, chat, false).await;
    if parsed.items.is_empty() {
        reply(&msg, gates::usage(caller_is_admin)).await;
        return;
    }

    // Determine job owner: if input was provided by a replied-to message, the owner is
    // the sender of that message (x), not the person who ran /get (y).
    let (owner_id, owner_display_name, owner_is_admin) =
        if let (true, Some(id)) = (parsed.from_reply, parsed.reply_sender_id) {
            let is_admin = state.auth.is_admin(id);
            let name = parsed.reply_sender_name.unwrap_or_else(|| format!("User {id}"));
            (id, name, is_admin)
        } else {
            let user = msg.sender_user().await.ok().flatten();
            let name = user
                .as_ref()
                .and_then(|u| {
                    u.username()
                        .filter(|n| !n.trim().is_empty())
                        .map(|n| format!("@{n}"))
                })
                .or_else(|| {
                    user.as_ref().map(|u| {
                        let first = u.first_name().unwrap_or_default().trim();
                        match u.last_name().map(str::trim).filter(|l| !l.is_empty()) {
                            Some(last) if !first.is_empty() => format!("{first} {last}"),
                            Some(last) => last.to_owned(),
                            None if !first.is_empty() => first.to_owned(),
                            None => String::new(),
                        }
                    })
                })
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("User {caller_id}"));
            (caller_id, name, caller_is_admin)
        };

    // If x owns the job and is not an admin, media should be delivered to x (not cache-only).
    let is_cache_only = if parsed.from_reply {
        owner_is_admin
    } else {
        caller_is_admin
    };

    let effective_admin = caller_is_admin || owner_is_admin;
    if let Some(text) = gates::cache_gate(is_cache_only, effective_admin) {
        reply(&msg, text).await;
        return;
    }
    if let Some(text) = gates::force_gate(parsed.force, effective_admin) {
        reply(&msg, text).await;
        return;
    }
    let settings = state.rip_deps.settings_snapshot();
    if let Some(text) = gates::feature_gate(&settings, &parsed.items, effective_admin, parsed.document) {
        reply(&msg, text).await;
        return;
    }

    let is_group = chat != owner_id;

    // Group DM preflight: for non-cache group requests, verify the owner can receive
    // DMs before queueing anything. A failure prompts them to start the bot in DM
    // and stops the job; on success delivery is retargeted to owner's private chat.
    let mut delivery_chat_id = chat;
    if is_group && !is_cache_only {
        let note = InputMessage::html(
            "<b>Download queued</b><br/>Tracks requested in this chat will be delivered to your private chat."
        )
        .silent(true);
        match state
            .client
            .send_message(ferogram::PeerRef::from(owner_id), note)
            .await
        {
            Ok(_) => delivery_chat_id = owner_id,
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
                let text = if owner_id != caller_id {
                    format!(
                        "! <b>Private chat required for {}</b><br/><br/>Audio files are delivered to private chat to keep this group clean.<br/>Start the bot in private chat, then send your request again.",
                        crate::html::escape(&owner_display_name)
                    )
                } else {
                    "! <b>Private chat required</b><br/><br/>Audio files are delivered to your private chat to keep this group clean.<br/>Start the bot in private chat, then send your request again.".to_owned()
                };
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
    super::ensure_dashboard(&state, chat, caller_id, caller_is_admin, super::chat_peer_ref(&msg)).await;

    let options = engine::orchestrator::types::RipJobOptions {
        provider: engine::Provider::Apple,
        chat_id: chat,
        user_id: owner_id,
        user_name: Some(owner_display_name),
        delivery_chat_id,
        is_group,
        is_force: parsed.force,
        is_cache_only,
        single_storefront: parsed.storefront,
        parsed_items: parsed.items,
        reply_to_message_id: Some(i64::from(msg.id())),
        // Status is rendered by the chat dashboard rather than a per-job
        // message. The zero sentinel keeps the engine type stable for other
        // orchestration callers.
        status_msg_id: 0,
        is_admin: owner_is_admin,
        rendition_policy: engine::orchestrator::types::RenditionPolicy::PrimaryWithOptionalAtmos,
    };

    // Engine owns everything from here: resolution, cache-first, queue,
    // pipeline, and terminal events. The bridge refreshes the dashboard;
    // start_job errors are logged only.
    if let Err(error) = state
        .rip_orchestrator
        .start_job(Arc::clone(&state.rip_deps), &options)
        .await
    {
        tracing::error!(error = %error, "get job failed");
    }
}

async fn handle_cancel_id_command(state: Arc<BotState>, msg: ferogram::update::IncomingMessage) {
    let text = msg.text().unwrap_or_default();
    let first_word = text.split_whitespace().next().unwrap_or("");
    let raw_cmd = first_word.split('@').next().unwrap_or(first_word);
    let Some(job_id) = raw_cmd.strip_prefix("/cancel_") else {
        return;
    };
    if job_id.is_empty() {
        return;
    }
    let caller = msg.sender_user_id().unwrap_or_default();
    let admin = state.auth.is_admin(caller);
    match cancel::cancel_inline(&state, job_id, caller, admin) {
        cancel::CancelResult::Cancelled => {
            reply(&msg, cancel::COMMAND_ACK).await;
        }
        cancel::CancelResult::Unauthorized => {
            reply(&msg, cancel::CALLBACK_UNAUTHORIZED).await;
        }
        cancel::CancelResult::Expired => {
            reply(&msg, cancel::CALLBACK_EXPIRED).await;
        }
    }
}

async fn reply(msg: &ferogram::update::IncomingMessage, text: &str) {
    let _ = msg.reply(InputMessage::html(text)).await;
}
